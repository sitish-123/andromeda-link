package org.andromeda.link.clipboard

import android.annotation.SuppressLint
import android.content.ContentResolver
import android.content.ContentUris
import android.content.Context
import android.database.ContentObserver
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import androidx.core.os.bundleOf
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Phone to desktop: Android never puts a screenshot on the clipboard, so each new one in the media store is offered as
 * an image clip. Needs the photos permission. A screenshot taken while no desktop is connected is not sent later.
 */
class ScreenshotWatcher(
    private val context: Context,
    private val sync: ClipboardSync,
    private val scope: CoroutineScope,
) {
    /** The newest image id already looked at; screenshots from before [start] are never sent. */
    private var seen = Long.MAX_VALUE

    private val observer =
        object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) = check()
        }

    fun start() {
        seen = newest(0L, screenshotsOnly = false)?.first ?: 0L
        context.contentResolver.registerContentObserver(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true, observer)
    }

    // Runs on the main thread, so two changes in a row never send one screenshot twice.
    private fun check() {
        val (id, path) = newest(seen, screenshotsOnly = true) ?: return
        seen = id
        if (!sync.sendsScreenshots) return
        val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id)
        scope.launch {
            val png = withContext(Dispatchers.Default) { pngWhenWritten(uri, File(path)) } ?: return@launch
            try {
                sync.offerImage(Uri.fromFile(png), PNG)
            } finally {
                png.delete()
            }
        }
    }

    /**
     * A pending screenshot is found as soon as it is inserted, often before its bytes are written; a partial file does
     * not decode, so it is tried again. Only its owner may open a pending item, so until the screenshot app lets go of
     * it (about 5 s) it is read by [path], which All files access allows.
     */
    private suspend fun pngWhenWritten(
        uri: Uri,
        path: File,
    ): File? {
        repeat(WRITE_TRIES) {
            val source = if (path.canRead()) ImageDecoder.createSource(path) else ImageDecoder.createSource(context.contentResolver, uri)
            runCatching { png(source) }.onSuccess { return it }
            delay(WRITE_RETRY_MS)
        }
        return null
    }

    /** Desktop apps paste images as PNG, and many ask for nothing else; phones save screenshots as JPEG. */
    private fun png(source: ImageDecoder.Source): File {
        val bitmap = ImageDecoder.decodeBitmap(source)
        val file = File.createTempFile("screenshot-", ".png", context.cacheDir)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
        return file
    }

    /**
     * The id and file path of the newest image after [after], pending or not. Android 10 ignores the pending match key,
     * so there a screenshot is found only after its preview.
     */
    @SuppressLint("InlinedApi")
    private fun newest(
        after: Long,
        screenshotsOnly: Boolean,
    ): Pair<Long, String>? {
        val id = MediaStore.Images.Media._ID
        val selection = "$id > ?" + if (screenshotsOnly) " AND ${MediaStore.Images.Media.RELATIVE_PATH} LIKE ?" else ""
        val args = listOfNotNull(after.toString(), "%Screenshots%".takeIf { screenshotsOnly }).toTypedArray()
        // The screenshot app keeps the file pending while its preview shows, about 5 s after the file is complete.
        val query =
            bundleOf(
                ContentResolver.QUERY_ARG_SQL_SELECTION to selection,
                ContentResolver.QUERY_ARG_SQL_SELECTION_ARGS to args,
                ContentResolver.QUERY_ARG_SQL_SORT_ORDER to "$id DESC",
                MediaStore.QUERY_ARG_MATCH_PENDING to MediaStore.MATCH_INCLUDE,
            )
        return context.contentResolver
            .query(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, arrayOf(id, MediaStore.Images.Media.DATA), query, null)
            ?.use { if (it.moveToFirst()) it.getLong(0) to it.getString(1).orEmpty() else null }
    }

    private companion object {
        const val PNG = "image/png"
        const val WRITE_TRIES = 40
        const val WRITE_RETRY_MS = 250L
    }
}

package org.andromeda.link.clipboard

import android.content.ContentUris
import android.content.Context
import android.database.ContentObserver
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
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
        seen = newest(0L, screenshotsOnly = false) ?: 0L
        context.contentResolver.registerContentObserver(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true, observer)
    }

    // Runs on the main thread, so two changes in a row never send one screenshot twice.
    private fun check() {
        val id = newest(seen, screenshotsOnly = true) ?: return
        seen = id
        if (!sync.sendsScreenshots) return
        val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id)
        scope.launch {
            val png = withContext(Dispatchers.Default) { png(uri) }
            try {
                sync.offerImage(Uri.fromFile(png), PNG)
            } finally {
                png.delete()
            }
        }
    }

    /** Desktop apps paste images as PNG, and many ask for nothing else; phones save screenshots as JPEG. */
    private fun png(uri: Uri): File {
        val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri))
        val file = File.createTempFile("screenshot-", ".png", context.cacheDir)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
        return file
    }

    /** The id of the newest finished image after [after]; MediaStore leaves out pending files. */
    private fun newest(
        after: Long,
        screenshotsOnly: Boolean,
    ): Long? {
        val id = MediaStore.Images.Media._ID
        val selection = "$id > ?" + if (screenshotsOnly) " AND ${MediaStore.Images.Media.RELATIVE_PATH} LIKE ?" else ""
        val args = listOfNotNull(after.toString(), "%Screenshots%".takeIf { screenshotsOnly }).toTypedArray()
        return context.contentResolver
            .query(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, arrayOf(id), selection, args, "$id DESC")
            ?.use { if (it.moveToFirst()) it.getLong(0) else null }
    }

    private companion object {
        const val PNG = "image/png"
    }
}

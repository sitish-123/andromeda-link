package org.andromeda.link.clipboard

import android.content.ContentUris
import android.content.Context
import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

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
        val (id, mime) = newest(seen, screenshotsOnly = true) ?: return
        seen = id
        if (!sync.sendsScreenshots) return
        val uri = ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id)
        scope.launch { sync.offerImage(uri, mime) }
    }

    /** The newest finished image after [after], as its id and MIME type; pending files are left out by MediaStore. */
    private fun newest(
        after: Long,
        screenshotsOnly: Boolean,
    ): Pair<Long, String>? {
        val id = MediaStore.Images.Media._ID
        val mime = MediaStore.Images.Media.MIME_TYPE
        val selection = "$id > ?" + if (screenshotsOnly) " AND ${MediaStore.Images.Media.RELATIVE_PATH} LIKE ?" else ""
        val args = listOfNotNull(after.toString(), "%Screenshots%".takeIf { screenshotsOnly }).toTypedArray()
        return context.contentResolver
            .query(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, arrayOf(id, mime), selection, args, "$id DESC")
            ?.use { if (it.moveToFirst()) it.getLong(0) to (it.getString(1) ?: "image/png") else null }
    }
}

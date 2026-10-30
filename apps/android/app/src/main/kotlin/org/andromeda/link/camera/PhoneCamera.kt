package org.andromeda.link.camera

import android.Manifest
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.MediaStore
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import org.andromeda.link.R
import org.andromeda.link.ffi.PhotoOutcome
import org.andromeda.link.notifications.Channels
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.andromeda.link.ffi.PhoneCamera as Camera

/**
 * A photo a desktop asks for: the phone's camera app takes it into Pictures/Link, and the desktop reads it from the
 * Photos root. The camera opens at once when Link may draw over other apps (Android lets such an app start an activity
 * from the background); otherwise a notification opens it. [takePhoto] runs off the main thread and blocks until the
 * photo is saved, the user cancels, or [WAIT_MIN] passes.
 */
class PhoneCamera(
    private val context: Context,
) : Camera {
    private val notifications = context.getSystemService(NotificationManager::class.java)

    override fun takePhoto(desktop: String): PhotoOutcome {
        if (context.checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            return PhotoOutcome.Denied
        }
        val name = "Link_${SimpleDateFormat("yyyyMMdd_HHmmss", Locale.ROOT).format(Date())}.jpg"
        val values =
            ContentValues().apply {
                put(MediaStore.MediaColumns.DISPLAY_NAME, name)
                put(MediaStore.MediaColumns.MIME_TYPE, "image/jpeg")
                // Not pending: Android lets only the owner open a pending item, and the camera app writes this one.
                put(MediaStore.MediaColumns.RELATIVE_PATH, "Pictures/$ALBUM")
            }
        val uri =
            runCatching { context.contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values) }.getOrNull()
                ?: return PhotoOutcome.Failed
        val shot = Shot(uri)
        if (!pending.compareAndSet(null, shot)) {
            context.contentResolver.delete(uri, null, null)
            return PhotoOutcome.Failed
        }
        try {
            val open = Intent(context, CameraActivity::class.java).setData(uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (Settings.canDrawOverlays(context)) context.startActivity(open) else ask(desktop, open)
            val answered = shot.done.await(WAIT_MIN, TimeUnit.MINUTES)
            notifications.cancel(NOTIFICATION)
            if (!answered || shot.taken != true) {
                context.contentResolver.delete(uri, null, null)
                return PhotoOutcome.Cancelled
            }
            // The camera app may rename the file; the media store has its final name.
            val stored =
                context.contentResolver
                    .query(uri, arrayOf(MediaStore.MediaColumns.DISPLAY_NAME), null, null, null)
                    ?.use { if (it.moveToFirst()) it.getString(0) else null } ?: name
            return PhotoOutcome.Taken(ALBUM, stored)
        } catch (error: RuntimeException) {
            Log.w(TAG, "taking a photo: ${error.message}")
            context.contentResolver.delete(uri, null, null)
            return PhotoOutcome.Failed
        } finally {
            pending.set(null)
        }
    }

    private fun ask(
        desktop: String,
        open: Intent,
    ) {
        val tap = PendingIntent.getActivity(context, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification =
            NotificationCompat
                .Builder(context, Channels.SCREEN)
                .setSmallIcon(R.drawable.ic_stat_link)
                .setContentTitle(context.getString(R.string.camera_request_title, desktop))
                .setContentText(context.getString(R.string.camera_request_text))
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setContentIntent(tap)
                .setAutoCancel(true)
                .addAction(0, context.getString(R.string.camera_take), tap)
                .build()
        notifications.notify(NOTIFICATION, notification)
    }

    private class Shot(
        val uri: Uri,
    ) {
        val done = CountDownLatch(1)

        @Volatile var taken: Boolean? = null
    }

    companion object {
        private const val TAG = "PhoneCamera"
        private const val ALBUM = "Link"
        private const val NOTIFICATION = 44
        private const val WAIT_MIN = 5L
        private val pending = AtomicReference<Shot?>(null)

        /** The camera app's answer for [uri]. */
        fun finished(
            uri: Uri?,
            taken: Boolean,
        ) {
            val shot = pending.get()?.takeIf { it.uri == uri } ?: return
            shot.taken = taken
            shot.done.countDown()
        }
    }
}

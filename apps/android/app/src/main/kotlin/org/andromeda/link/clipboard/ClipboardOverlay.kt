package org.andromeda.link.clipboard

import android.Manifest
import android.app.PendingIntent
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.PixelFormat
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import org.andromeda.link.R
import org.andromeda.link.notifications.Channels

/**
 * Reads the clipboard from the background: a one-pixel overlay window takes focus, the one state in which Android lets
 * an app read it, and goes as soon as it has the clip. Android 15 and later refuse [ClipboardReadActivity] from the
 * background even with "Display over other apps", which an overlay window needs and nothing more. The URI grant a read
 * gives lasts while the clip is the primary one, so the snapshot runs after the window is gone. An app that hides
 * overlays, as Settings does, keeps the window from focus; then a quiet notification sends the clip in one tap.
 */
class ClipboardOverlay(
    private val context: Context,
    private val sync: ClipboardSync,
    private val scope: CoroutineScope,
) {
    private val windows = context.getSystemService(WindowManager::class.java)
    private val clipboard = context.getSystemService(ClipboardManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private var view: View? = null

    fun read() {
        main.post(::show)
    }

    private fun show() {
        if (view != null) return
        val window =
            object : View(context) {
                override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
                    super.onWindowFocusChanged(hasWindowFocus)
                    if (hasWindowFocus) taken(this)
                }
            }
        val params =
            WindowManager
                .LayoutParams(
                    1,
                    1,
                    WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
                    WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
                    PixelFormat.TRANSLUCENT,
                ).apply { gravity = Gravity.TOP or Gravity.START }
        try {
            windows.addView(window, params)
        } catch (error: RuntimeException) {
            Log.w("link", "Clipboard overlay: ${error.message}")
            return
        }
        view = window
        main.postDelayed({ remove(window) }, FOCUS_TIMEOUT_MS)
    }

    private fun taken(window: View) {
        if (view !== window) return
        view = null
        NotificationManagerCompat.from(context).cancel(PROMPT_TAG, 0)
        val clip = clipboard.primaryClip
        // Removing a window during its own focus dispatch crashes ViewRootImpl.
        main.post { windows.removeView(window) }
        if (clip == null || clip.itemCount == 0) return
        scope.launch { sync.offer(clip).onFailure { Log.w("link", "Sending clipboard: ${it.message}") } }
    }

    private fun remove(window: View) {
        if (view !== window) return
        view = null
        windows.removeView(window)
        prompt()
    }

    private fun prompt() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        val send =
            PendingIntent.getActivity(
                context,
                0,
                Intent(context, ClipboardReadActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val notification =
            NotificationCompat
                .Builder(context, Channels.SHARES)
                .setSmallIcon(R.drawable.ic_stat_link)
                .setContentTitle(context.getString(R.string.clipboard_prompt_title))
                .setContentText(context.getString(R.string.clipboard_prompt_text))
                .setContentIntent(send)
                .setAutoCancel(true)
                .setSilent(true)
                .build()
        NotificationManagerCompat.from(context).notify(PROMPT_TAG, 0, notification)
    }

    private companion object {
        const val FOCUS_TIMEOUT_MS = 2_000L
        const val PROMPT_TAG = "clipboard-prompt"
    }
}

package org.andromeda.link.screen

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.Intent
import android.graphics.Path
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.KeyEvent
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import org.andromeda.link.core.domain.MIRROR_UNIT
import org.andromeda.link.core.domain.MirrorAction
import org.andromeda.link.core.domain.MirrorCommand

/**
 * The desktop's input on the mirrored screen: taps, long presses, swipes, and scrolls as gestures, Back, Home, and
 * Recents as global actions, and text into the focused field. An accessibility service is the only way a sideloaded
 * app can inject input; the user turns it on once in Accessibility settings.
 */
class ScreenInput : AccessibilityService() {
    override fun onServiceConnected() {
        instance = this
    }

    override fun onUnbind(intent: Intent?): Boolean {
        instance = null
        return super.onUnbind(intent)
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit

    override fun onInterrupt() = Unit

    private fun run(input: MirrorCommand.Input) {
        val (width, height) = screen()

        fun px(
            value: Int?,
            side: Int,
        ) = (value ?: 0).toFloat() / MIRROR_UNIT * side
        val x = px(input.x, width)
        val y = px(input.y, height)
        when (input.action) {
            MirrorAction.Tap -> gesture(line(x, y, x, y), TAP_MS)
            MirrorAction.LongPress -> gesture(line(x, y, x, y), input.ms?.toLong() ?: LONG_PRESS_MS)
            MirrorAction.Swipe -> gesture(line(x, y, px(input.x2, width), px(input.y2, height)), input.ms?.toLong() ?: SWIPE_MS)
            MirrorAction.Scroll -> {
                val toX = (x + px(input.x2, width)).coerceIn(0f, width - 1f)
                val toY = (y + px(input.y2, height)).coerceIn(0f, height - 1f)
                gesture(line(x, y, toX, toY), SCROLL_MS)
            }
            MirrorAction.Back -> performGlobalAction(GLOBAL_ACTION_BACK)
            MirrorAction.Home -> performGlobalAction(GLOBAL_ACTION_HOME)
            MirrorAction.Recents -> performGlobalAction(GLOBAL_ACTION_RECENTS)
            MirrorAction.Text -> type(input.text.orEmpty())
            MirrorAction.TouchDown -> touch = Touch(Path().apply { moveTo(x, y) }, System.currentTimeMillis())
            MirrorAction.TouchMove -> touch?.path?.lineTo(x, y)
            MirrorAction.TouchUp ->
                touch?.let {
                    touch = null
                    it.path.lineTo(x, y)
                    gesture(it.path, System.currentTimeMillis() - it.startMs)
                }
            MirrorAction.Wheel -> {
                // A wheel step drags a tenth of the screen; wheel down moves the content up.
                val toX = (x - px(input.x2, width) / WHEEL_STEP * MIRROR_UNIT / 10).coerceIn(0f, width - 1f)
                val toY = (y - px(input.y2, height) / WHEEL_STEP * MIRROR_UNIT / 10).coerceIn(0f, height - 1f)
                gesture(line(x, y, toX, toY), SCROLL_MS)
            }
            MirrorAction.KeyDown -> key(input.code ?: 0)
            MirrorAction.KeyUp, MirrorAction.ScreenOff, MirrorAction.ScreenOn -> Unit
        }
    }

    /** The few keys Accessibility can act on; the shell helper injects every other key. */
    private fun key(code: Int) {
        when (code) {
            KeyEvent.KEYCODE_BACK -> performGlobalAction(GLOBAL_ACTION_BACK)
            KeyEvent.KEYCODE_HOME -> performGlobalAction(GLOBAL_ACTION_HOME)
            KeyEvent.KEYCODE_APP_SWITCH -> performGlobalAction(GLOBAL_ACTION_RECENTS)
            KeyEvent.KEYCODE_ENTER -> type("\n")
            KeyEvent.KEYCODE_DEL -> type(BACKSPACE)
            KeyEvent.KEYCODE_VOLUME_UP, KeyEvent.KEYCODE_VOLUME_DOWN -> {
                val direction = if (code == KeyEvent.KEYCODE_VOLUME_UP) AudioManager.ADJUST_RAISE else AudioManager.ADJUST_LOWER
                getSystemService(AudioManager::class.java)
                    .adjustStreamVolume(AudioManager.STREAM_MUSIC, direction, AudioManager.FLAG_SHOW_UI)
            }
            KeyEvent.KEYCODE_POWER -> performGlobalAction(GLOBAL_ACTION_LOCK_SCREEN)
            else -> Log.i(TAG, "key $code needs the shell helper")
        }
    }

    private class Touch(
        val path: Path,
        val startMs: Long,
    )

    private var touch: Touch? = null

    private fun screen(): Pair<Int, Int> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds.let { it.width() to it.height() }
        } else {
            resources.displayMetrics.let { it.widthPixels to it.heightPixels }
        }

    private fun line(
        x: Float,
        y: Float,
        toX: Float,
        toY: Float,
    ) = Path().apply {
        moveTo(x, y)
        lineTo(toX, toY)
    }

    private fun gesture(
        path: Path,
        ms: Long,
    ) {
        val stroke = GestureDescription.StrokeDescription(path, 0, ms.coerceIn(1, MAX_GESTURE_MS))
        if (!dispatchGesture(GestureDescription.Builder().addStroke(stroke).build(), null, null)) {
            Log.i(TAG, "the gesture was not dispatched")
        }
    }

    /** Appends to the focused field; a backspace (U+0008) removes its last character, a newline presses Enter. */
    private fun type(text: String) {
        val field = rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return
        if (text == "\n" && Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            field.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
            return
        }
        val showsHint = field.isShowingHintText
        val current = if (showsHint) "" else field.text?.toString().orEmpty()
        val next = if (text == BACKSPACE) current.dropLast(1) else current + text
        val arguments =
            Bundle().apply {
                putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, next)
            }
        field.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments)
    }

    companion object {
        @Volatile
        private var instance: ScreenInput? = null
        private val main = Handler(Looper.getMainLooper())

        /** Whether the user turned the service on. */
        val enabled: Boolean get() = instance != null

        fun perform(input: MirrorCommand.Input) {
            if (relayed(input)) return
            val service = instance
            if (service == null) {
                Log.i(TAG, "input dropped: the accessibility service is off")
                return
            }
            main.post { service.run(input) }
        }

        /** Sends [input] to the shell helper, when one runs and the action is one it injects as real events. */
        private fun relayed(input: MirrorCommand.Input): Boolean {
            if (!InputCommandProvider.connected) return false

            fun send(action: String) =
                InputCommandProvider.send(action, input.x, input.y, input.x2, input.y2, input.code, input.meta, input.text)

            fun key(code: Int) = InputCommandProvider.send("key-down", code = code) && InputCommandProvider.send("key-up", code = code)
            return when (input.action) {
                MirrorAction.TouchDown -> send("touch-down")
                MirrorAction.TouchMove -> send("touch-move")
                MirrorAction.TouchUp -> send("touch-up")
                MirrorAction.Wheel -> send("wheel")
                MirrorAction.KeyDown -> send("key-down")
                MirrorAction.KeyUp -> send("key-up")
                MirrorAction.ScreenOff -> send("screen-off")
                MirrorAction.ScreenOn -> send("screen-on")
                MirrorAction.Back -> key(KeyEvent.KEYCODE_BACK)
                MirrorAction.Home -> key(KeyEvent.KEYCODE_HOME)
                MirrorAction.Recents -> key(KeyEvent.KEYCODE_APP_SWITCH)
                // The virtual keyboard map has keys for ASCII only; other text goes into the field through Accessibility.
                MirrorAction.Text -> input.text.orEmpty().all { it in ' '..'~' } && send("text")
                MirrorAction.Tap, MirrorAction.LongPress, MirrorAction.Swipe, MirrorAction.Scroll -> false
            }
        }

        private const val TAG = "ScreenInput"
        private const val BACKSPACE = "\u0008"
        private const val TAP_MS = 50L
        private const val LONG_PRESS_MS = 700L
        private const val SWIPE_MS = 300L
        private const val SCROLL_MS = 150L
        private const val MAX_GESTURE_MS = 10_000L
        private const val WHEEL_STEP = 100f
    }
}

package org.andromeda.link.screen

import android.annotation.SuppressLint
import android.os.IBinder
import android.os.SystemClock
import android.util.Base64
import android.view.InputDevice
import android.view.InputEvent
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import java.io.IOException

/**
 * The desktop's mirror input as real events, in the shell-uid helper ([org.andromeda.link.clipboard.LogHelper]): the
 * shell may inject input and set the panel's power, which an app may not. It reads Link's commands through
 * `content read` on [InputCommandProvider], so it takes commands only from Link, and only the ones in [run]: no shell.
 * Coordinates arrive in 1/10000 of the screen and map to the default display's current, rotated size. It runs only in
 * the app_process helper, where the hidden-API limits on apps do not apply.
 */
@SuppressLint("BlockedPrivateApi", "PrivateApi", "DiscouragedPrivateApi", "SoonBlockedPrivateApi")
object InputRelay {
    private const val UNIT = 10_000f
    private const val WHEEL_STEP = 100f
    private const val POWER_OFF = 0
    private const val POWER_NORMAL = 2

    private var touchDownMs = 0L
    private var touching = false
    private val held = mutableSetOf<Int>()
    private var screenOff = false

    /** Reads commands until Link closes the stream; a panel left off goes back on then. */
    fun follow(pkg: String) {
        val process =
            runCatching {
                ProcessBuilder("content", "read", "--uri", "content://$pkg.inputhelper/commands")
                    .redirectError(ProcessBuilder.Redirect.INHERIT)
                    .start()
            }.getOrNull() ?: return
        try {
            process.inputStream.bufferedReader().forEachLine { line ->
                runCatching { run(line.split(' ')) }.onFailure { println("input: ${it.message}") }
            }
        } catch (_: IOException) {
        } finally {
            process.destroy()
            release()
        }
    }

    /** Lifts a finger and keys still down and turns the panel on, so a lost Link leaves the phone usable. */
    private fun release() {
        val now = SystemClock.uptimeMillis()
        if (touching) touch(MotionEvent.ACTION_CANCEL, 0f, 0f, now)
        held.toList().forEach { inject(key(KeyEvent.ACTION_UP, it, 0, now)) }
        held.clear()
        if (screenOff) power(POWER_NORMAL)
    }

    /** `action x y x2 y2 code meta text`: text is Base64, empty fields are 0. */
    private fun run(fields: List<String>) {
        fun int(index: Int) = fields.getOrNull(index)?.toIntOrNull() ?: 0
        val (width, height) = screen()
        val x = (int(1) / UNIT * width).coerceAtMost(width - 1f)
        val y = (int(2) / UNIT * height).coerceAtMost(height - 1f)
        val now = SystemClock.uptimeMillis()
        when (fields[0]) {
            "touch-down" -> {
                touchDownMs = now
                touching = true
                touch(MotionEvent.ACTION_DOWN, x, y, now)
            }
            "touch-move" -> if (touching) touch(MotionEvent.ACTION_MOVE, x, y, now)
            "touch-up" ->
                if (touching) {
                    touching = false
                    touch(MotionEvent.ACTION_UP, x, y, now)
                }
            "wheel" -> wheel(x, y, int(3) / WHEEL_STEP, -int(4) / WHEEL_STEP, now)
            "key-down" -> {
                held.add(int(5))
                inject(key(KeyEvent.ACTION_DOWN, int(5), int(6), now))
            }
            "key-up" -> {
                held.remove(int(5))
                inject(key(KeyEvent.ACTION_UP, int(5), int(6), now))
            }
            "text" -> {
                val text = String(Base64.decode(fields.getOrNull(7).orEmpty(), Base64.NO_WRAP))
                val events = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD).getEvents(text.toCharArray())
                events?.forEach(::inject) ?: println("input: no keys for some text")
            }
            "screen-off" -> if (power(POWER_OFF)) screenOff = true
            "screen-on" -> if (power(POWER_NORMAL)) screenOff = false
        }
    }

    private fun touch(
        action: Int,
        x: Float,
        y: Float,
        now: Long,
    ) {
        val event = MotionEvent.obtain(touchDownMs, now, action, x, y, 0)
        event.source = InputDevice.SOURCE_TOUCHSCREEN
        inject(event)
        event.recycle()
    }

    private fun wheel(
        x: Float,
        y: Float,
        horizontal: Float,
        vertical: Float,
        now: Long,
    ) {
        val properties =
            MotionEvent.PointerProperties().apply {
                id = 0
                toolType = MotionEvent.TOOL_TYPE_MOUSE
            }
        val coords =
            MotionEvent.PointerCoords().apply {
                this.x = x
                this.y = y
                setAxisValue(MotionEvent.AXIS_HSCROLL, horizontal)
                setAxisValue(MotionEvent.AXIS_VSCROLL, vertical)
            }
        val event =
            MotionEvent.obtain(
                now,
                now,
                MotionEvent.ACTION_SCROLL,
                1,
                arrayOf(properties),
                arrayOf(coords),
                0,
                0,
                1f,
                1f,
                0,
                0,
                InputDevice.SOURCE_MOUSE,
                0,
            )
        inject(event)
        event.recycle()
    }

    private fun key(
        action: Int,
        code: Int,
        meta: Int,
        now: Long,
    ) = KeyEvent(now, now, action, code, 0, meta, KeyCharacterMap.VIRTUAL_KEYBOARD, 0, 0, InputDevice.SOURCE_KEYBOARD)

    private val injector: Pair<Any, java.lang.reflect.Method> by lazy {
        // Android 14 moved the injecting instance to InputManagerGlobal.
        val manager =
            runCatching { Class.forName("android.hardware.input.InputManagerGlobal") }
                .getOrElse { Class.forName("android.hardware.input.InputManager") }
        val instance = manager.getMethod("getInstance").invoke(null)!!
        instance to manager.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
    }

    private fun inject(event: InputEvent) {
        val (instance, method) = injector
        if (method.invoke(instance, event, 0) != true) println("input: an event was not injected")
    }

    private fun screen(): Pair<Int, Int> {
        val global = Class.forName("android.hardware.display.DisplayManagerGlobal")
        val info =
            global
                .getMethod(
                    "getDisplayInfo",
                    Int::class.javaPrimitiveType,
                ).invoke(global.getMethod("getInstance").invoke(null), 0)!!
        return info.javaClass.getField("logicalWidth").getInt(info) to info.javaClass.getField("logicalHeight").getInt(info)
    }

    /** The built-in panel's power, as `scrcpy --turn-screen-off` sets it; the frames keep coming. */
    private fun power(mode: Int): Boolean =
        runCatching {
            val surface = Class.forName("android.view.SurfaceControl")
            surface.getMethod("setDisplayPowerMode", IBinder::class.java, Int::class.javaPrimitiveType).invoke(null, panel, mode)
            true
        }.getOrElse {
            println("input: panel power: ${it.cause?.message ?: it.message}")
            false
        }

    // Android loads DisplayControl's native library into one class loader only, so the token is looked up once.
    private val panel: IBinder by lazy { panelToken() }

    private fun panelToken(): IBinder {
        if (android.os.Build.VERSION.SDK_INT < 34) {
            val surface = Class.forName("android.view.SurfaceControl")
            return surface.getMethod("getInternalDisplayToken").invoke(null) as IBinder
        }
        // Android 14 keeps the physical display tokens in DisplayControl, inside services.jar and its native library.
        val factory = Class.forName("com.android.internal.os.ClassLoaderFactory")
        val create =
            factory.getDeclaredMethod(
                "createClassLoader",
                String::class.java,
                String::class.java,
                String::class.java,
                ClassLoader::class.java,
                Int::class.javaPrimitiveType,
                Boolean::class.javaPrimitiveType,
                String::class.java,
            )
        val loader =
            create.invoke(
                null,
                "/system/framework/services.jar",
                null,
                null,
                ClassLoader.getSystemClassLoader(),
                0,
                true,
                null,
            ) as ClassLoader
        val control = loader.loadClass("com.android.server.display.DisplayControl")
        Runtime::class.java
            .getDeclaredMethod("loadLibrary0", Class::class.java, String::class.java)
            .apply { isAccessible = true }
            .invoke(Runtime.getRuntime(), control, "android_servers")
        val id = (control.getMethod("getPhysicalDisplayIds").invoke(null) as LongArray).first()
        return control.getMethod("getPhysicalDisplayToken", Long::class.javaPrimitiveType).invoke(null, id) as IBinder
    }
}

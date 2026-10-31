package org.andromeda.link.control

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.SystemClock
import android.provider.Settings
import android.util.DisplayMetrics
import android.view.MotionEvent
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerId
import androidx.compose.ui.input.pointer.changedToDownIgnoreConsumed
import androidx.compose.ui.input.pointer.motionEventSpy
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.andromeda.link.core.domain.TouchContact
import kotlin.math.abs

/**
 * Reports every finger on this area each frame, in 1/100 mm from its top left, for a desktop that makes a real
 * touchpad of them; an empty frame once the last finger lifts. The desktop's own settings turn them into moves,
 * taps, scrolls and gestures. [metrics] gives the screen's real pixel density.
 */
@OptIn(ExperimentalComposeUiApi::class)
fun Modifier.rawTouchpad(
    metrics: DisplayMetrics,
    onPinch: ((centroid: Offset, factor: Float, moved: Offset) -> Unit)? = null,
    onTick: () -> Unit = {},
    onFrame: (List<TouchContact>) -> Unit,
): Modifier {
    // Pixels to 1/100 mm: an inch is 2540 of them.
    val (perX, perY) = UNITS_PER_INCH / metrics.xdpi to UNITS_PER_INCH / metrics.ydpi
    var cancelled = false
    return motionEventSpy { if (it.actionMasked == MotionEvent.ACTION_CANCEL) cancelled = true }.pointerInput(metrics) {
        val ids = mutableMapOf<PointerId, Int>()
        var downAt = 0L
        var lift: Job? = null
        var startSpread = 0f
        var pinching = false
        val starts = mutableMapOf<PointerId, Offset>()
        var fingers = 0
        var travel = 0f
        var swiped = false
        coroutineScope {
            awaitPointerEventScope {
                while (true) {
                    val event = awaitPointerEvent()
                    lift?.cancel()
                    if (ids.isEmpty()) {
                        downAt = SystemClock.uptimeMillis()
                        fingers = 0
                        travel = 0f
                        swiped = false
                    }
                    for (change in event.changes.filter { it.pressed }) {
                        val start = starts.getOrPut(change.id) { change.position }
                        val moved = (change.position - start).let { Offset(it.x * perX, it.y * perY) }.getDistance()
                        travel = maxOf(travel, moved)
                    }
                    fingers = maxOf(fingers, event.changes.count { it.pressed })
                    if (!swiped && fingers >= 3 && travel > SWIPE_TICK) {
                        swiped = true
                        onTick()
                    }
                    val contacts =
                        event.changes.filter { it.pressed }.mapNotNull { change ->
                            val id = ids.getOrPut(change.id) { (0..MAX_ID).firstOrNull { it !in ids.values } ?: return@mapNotNull null }
                            TouchContact(id, units(change.position.x * perX), units(change.position.y * perY))
                        }
                    event.changes.filterNot { it.pressed }.forEach {
                        ids.remove(it.id)
                        starts.remove(it.id)
                    }
                    event.changes.forEach { it.consume() }
                    val pressed = event.changes.filter { it.pressed }
                    if (pressed.isEmpty()) pinching = false
                    if (onPinch != null && pressed.size == 2) {
                        val (a, b) = pressed
                        val spread = (a.position - b.position).getDistance()
                        if (event.changes.any { it.changedToDownIgnoreConsumed() }) startSpread = spread
                        if (!pinching && startSpread > 0f && abs(spread / startSpread - 1f) > PINCH_START) {
                            // From here the fingers zoom this phone's picture; the desktop sees them lift.
                            pinching = true
                            onTick()
                            onFrame(emptyList())
                        }
                        if (pinching) {
                            val before = (a.previousPosition - b.previousPosition).getDistance()
                            val moved = (a.positionChange() + b.positionChange()) / 2f
                            onPinch((a.position + b.position) / 2f, if (before > 0f) spread / before else 1f, moved)
                        }
                    }
                    if (pinching) continue
                    if (contacts.isEmpty() && cancelled) {
                        // A system gesture took the touch: lifting at once would read as a tap on the desktop.
                        cancelled = false
                        val wait = CANCELLED_LIFT_MS - (SystemClock.uptimeMillis() - downAt)
                        lift =
                            launch {
                                delay(wait)
                                onFrame(emptyList())
                            }
                    } else {
                        // The desktop's own tap rule: lifted within its timeout, without moving.
                        if (contacts.isEmpty() && SystemClock.uptimeMillis() - downAt < TAP_MS && travel < TAP_SLOP) onTick()
                        onFrame(contacts.take(MAX_CONTACTS))
                    }
                }
            }
        }
    }
}

/** How far two fingers must spread or close, as a fraction of their first distance, to zoom instead of scroll. */
private const val PINCH_START = 0.15f

/** libinput's tap timeout, and how far a tapping finger may move, in 1/100 mm. */
private const val TAP_MS = 180L
private const val TAP_SLOP = 200f

/** How far 3 or more fingers move, in 1/100 mm, before the phone ticks for a swipe. */
private const val SWIPE_TICK = 500f

/** Longer than libinput's 180 ms tap timeout. */
private const val CANCELLED_LIFT_MS = 250L

private fun units(value: Float): Int = value.toInt().coerceIn(0, MAX_POSITION)

private const val UNITS_PER_INCH = 2540f

/** link-proto's MAX_TOUCH_ID, MAX_TOUCH_CONTACTS and MAX_TOUCH_POSITION. */
private const val MAX_ID = 9
private const val MAX_CONTACTS = 5
private const val MAX_POSITION = 30_000

/**
 * OxygenOS and ColorOS take every 3-finger touch for their own gestures and cancel it in this app. Their
 * remote-control switch turns those gestures off, as their cast apps do. Returns what puts the old value back, or null
 * when this phone has no such switch or the app lacks the grant.
 */
fun keepMultiFingerTouches(context: Context): (() -> Unit)? {
    if (!canKeepMultiFingerTouches(context)) return null
    val resolver = context.contentResolver
    val before = Settings.Global.getInt(resolver, OPLUS_REMOTE_CONTROL_GESTURES, 0)
    Settings.Global.putInt(resolver, OPLUS_REMOTE_CONTROL_GESTURES, 1)
    return { Settings.Global.putInt(resolver, OPLUS_REMOTE_CONTROL_GESTURES, before) }
}

/** True on an OxygenOS or ColorOS phone where this app lacks the adb grant that [keepMultiFingerTouches] needs. */
fun needsMultiFingerGrant(context: Context): Boolean = isOplus() && !canKeepMultiFingerTouches(context)

private fun canKeepMultiFingerTouches(context: Context): Boolean =
    isOplus() && context.checkSelfPermission(Manifest.permission.WRITE_SECURE_SETTINGS) == PackageManager.PERMISSION_GRANTED

private fun isOplus(): Boolean = Build.MANUFACTURER.lowercase() in setOf("oneplus", "oppo", "realme")

private const val OPLUS_REMOTE_CONTROL_GESTURES = "remote_control_input_gesture_switch"

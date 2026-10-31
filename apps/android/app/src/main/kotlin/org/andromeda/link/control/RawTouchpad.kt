package org.andromeda.link.control

import android.util.DisplayMetrics
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerId
import androidx.compose.ui.input.pointer.pointerInput
import org.andromeda.link.core.domain.TouchContact

/**
 * Reports every finger on this area each frame, in 1/100 mm from its top left, for a desktop that makes a real
 * touchpad of them; an empty frame once the last finger lifts. The desktop's own settings turn them into moves,
 * taps, scrolls and gestures. [metrics] gives the screen's real pixel density.
 */
fun Modifier.rawTouchpad(
    metrics: DisplayMetrics,
    onFrame: (List<TouchContact>) -> Unit,
): Modifier {
    // Pixels to 1/100 mm: an inch is 2540 of them.
    val (perX, perY) = UNITS_PER_INCH / metrics.xdpi to UNITS_PER_INCH / metrics.ydpi
    return pointerInput(metrics) {
        val ids = mutableMapOf<PointerId, Int>()
        awaitPointerEventScope {
            while (true) {
                val event = awaitPointerEvent()
                val contacts =
                    event.changes.filter { it.pressed }.mapNotNull { change ->
                        val id = ids.getOrPut(change.id) { (0..MAX_ID).firstOrNull { it !in ids.values } ?: return@mapNotNull null }
                        TouchContact(id, units(change.position.x * perX), units(change.position.y * perY))
                    }
                event.changes.filterNot { it.pressed }.forEach { ids.remove(it.id) }
                event.changes.forEach { it.consume() }
                onFrame(contacts.take(MAX_CONTACTS))
            }
        }
    }
}

private fun units(value: Float): Int = value.toInt().coerceIn(0, MAX_POSITION)

private const val UNITS_PER_INCH = 2540f

/** link-proto's MAX_TOUCH_ID, MAX_TOUCH_CONTACTS and MAX_TOUCH_POSITION. */
private const val MAX_ID = 9
private const val MAX_CONTACTS = 5
private const val MAX_POSITION = 30_000

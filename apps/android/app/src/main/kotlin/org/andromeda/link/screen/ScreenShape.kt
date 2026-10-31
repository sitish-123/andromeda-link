package org.andromeda.link.screen

import android.content.Context
import android.hardware.display.DisplayManager
import android.os.Build
import android.view.Display
import android.view.RoundedCorner
import android.view.WindowManager
import org.andromeda.link.core.domain.ScreenShape

/** The built-in screen's shape as it is turned now: corners from Android 12, cutouts from Android 10. */
fun screenShape(context: Context): ScreenShape {
    val display = context.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY)
    val (width, height) =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val bounds = context.getSystemService(WindowManager::class.java).maximumWindowMetrics.bounds
            bounds.width() to bounds.height()
        } else {
            context.resources.displayMetrics.let { it.widthPixels to it.heightPixels }
        }
    val corners =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            listOf(
                RoundedCorner.POSITION_TOP_LEFT,
                RoundedCorner.POSITION_TOP_RIGHT,
                RoundedCorner.POSITION_BOTTOM_RIGHT,
                RoundedCorner.POSITION_BOTTOM_LEFT,
            ).map { display.getRoundedCorner(it)?.radius ?: 0 }
        } else {
            emptyList()
        }
    return ScreenShape(width, height, corners, display.cutout?.boundingRects.orEmpty())
}

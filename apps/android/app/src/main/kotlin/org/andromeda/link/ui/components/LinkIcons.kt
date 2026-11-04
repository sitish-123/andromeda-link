package org.andromeda.link.ui.components

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

object LinkIcons {
    val Computer = glyph("Desktop", "M3 3H21V17H13V19H17V21H7V19H11V17H3ZM5 5V15H19V5Z")
    val Contacts =
        glyph(
            "Contacts",
            "M12 12C14.2 12 16 10.2 16 8S14.2 4 12 4 8 5.8 8 8 9.8 12 12 12ZM12 14C9.3 14 4 15.3 4 18V20H20V18C20 15.3 14.7 14 12 14Z",
        )
    val Folder = glyph("Folder", "M10 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z")
    val Transfers = glyph("Transfers", "M7 2L2 7H6V19H8V7H12ZM17 22L22 17H18V5H16V17H12Z")
    val Download = glyph("Receive", "M11 3H13V13L17 9L18.5 10.5L12 17L5.5 10.5L7 9L11 13ZM4 19H20V21H4Z")
    val Clipboard =
        glyph(
            "Clipboard",
            "M9 2H15V4H18C19.1 4 20 4.9 20 6V20C20 21.1 19.1 22 18 22H6C4.9 22 4 21.1 4 20V6C4 4.9 4.9 4 6 4H9ZM6 6V20H18V6H16V8H8V6Z",
        )
    val Muted =
        glyph(
            "Muted",
            "M3 9V15H7L12 20V4L7 9ZM15.4 9.8L16.8 8.4L19 10.6L21.2 8.4L22.6 9.8L20.4 12L22.6 14.2L21.2 15.6L19 13.4L16.8 15.6L15.4 14.2L17.6 12Z",
        )
    val Ring =
        glyph(
            "Ring",
            "M3 9V15H7L12 20V4L7 9ZM16.5 12A4.5 4.5 0 0 0 14 8V16A4.5 4.5 0 0 0 16.5 12ZM14 3.2V5.3A7 7 0 0 1 14 18.7V20.8A9 9 0 0 0 14 3.2Z",
        )
    val Upload = glyph("Send", "M11 17H13V7L17 11L18.5 9.5L12 3L5.5 9.5L7 11L11 7ZM4 19H20V21H4Z")
    val Lock =
        glyph(
            "Lock",
            "M18 8h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm-6 9c-1.1 0-2-.9-2-2s.9-2 2-2 2 .9 2 2-.9 2-2 2zm3.1-9H8.9V6c0-1.71 1.39-3.1 3.1-3.1 1.71 0 3.1 1.39 3.1 3.1v2z",
        )
    val LockOpen =
        glyph(
            "Unlocked",
            "M12 17c1.1 0 2-.9 2-2s-.9-2-2-2-2 .9-2 2 .9 2 2 2zm6-9h-1V6c0-2.76-2.24-5-5-5S7 3.24 7 6h1.9c0-1.71 1.39-3.1 3.1-3.1 1.71 0 3.1 1.39 3.1 3.1v2H6c-1.1 0-2 .9-2 2v10c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V10c0-1.1-.9-2-2-2zm0 12H6V10h12v10z",
        )
    val Sleep =
        glyph("Sleep", "M12.34 2.02C6.59 1.82 2 6.42 2 12c0 5.52 4.48 10 10 10 3.71 0 6.93-2.02 8.66-5.02-7.51-.25-12.09-8.43-8.32-14.96z")
    val ScreenOff =
        glyph(
            "Screen off",
            "M23 16c0 1.1-.9 2-2 2h-1l-2-2h3V4H6L4 2h17c1.1 0 2 .9 2 2v12zM1.2 1.8 0 3.1l1 1V16c0 1.1.9 2 2 2h7v2H8v2h8v-2h-2v-2h.9l6 6 1.3-1.3zM3 16V6.1l9.9 9.9z",
        )
    val Touchpad =
        glyph(
            "Touchpad",
            "M20 3H4c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16h-7v-4h7zm-9 0H4v-4h7zm9-6H4V5h16z",
        )
    val Presenter =
        glyph("Presenter", "M10 8v8l5-4zm9-5H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zm0 16H5V5h14z")
    val Dnd = glyph("Do not disturb", "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm5 11H7v-2h10z")
    val Brightness =
        glyph(
            "Brightness",
            "M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10zM1 11h3v2H1zm19 0h3v2h-3zM11 1h2v3h-2zm0 19h2v3h-2zM4.2 5.6l1.4-1.4 2.1 2.1-1.4 1.4zm12.1 12.1 1.4-1.4 2.1 2.1-1.4 1.4zm1.4-13.5 1.4 1.4-2.1 2.1-1.4-1.4zM5.6 19.8l-1.4-1.4 2.1-2.1 1.4 1.4z",
        )
    val Previous = glyph("Previous", "M6 6h2v12H6zm3.5 6 8.5 6V6z")
    val Next = glyph("Next", "M6 18l8.5-6L6 6zM16 6v12h2V6z")
    val Pause = glyph("Pause", "M6 19h4V5H6zm8-14v14h4V5z")
    val Photo =
        glyph("Photos", "M21 19V5c0-1.1-.9-2-2-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2zM8.5 13.5l2.5 3.01L14.5 12l4.5 6H5z")
    val Video = glyph("Videos", "M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11z")
    val Charging = glyph("Charging", "M11 21h-1l1-7H7l6-11h1l-1 7h4z")
}

private fun glyph(
    name: String,
    path: String,
) = ImageVector
    .Builder(name, 24.dp, 24.dp, 24f, 24f)
    .addPath(PathParser().parsePathString(path).toNodes(), fill = SolidColor(Color.Black))
    .build()

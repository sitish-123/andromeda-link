package org.andromeda.link.core.domain

/** A desktop's clipboard: [text] is inline and set at once; other types are pulled when an app reads them. */
data class IncomingClip(
    val desktopId: String,
    val clipId: Long,
    val mimes: List<String>,
    val text: String?,
)

/** One type of a phone clip: inline text, or a URI snapshotted while Android's read grant is valid. */
sealed interface ClipPart {
    val mime: String

    data class Text(
        override val mime: String,
        val text: String,
    ) : ClipPart

    data class Content(
        override val mime: String,
        val uri: android.net.Uri,
    ) : ClipPart
}

/** A clip sent or received in this app process; [text] is null for images and files. [time] is Unix milliseconds. */
data class RecentClip(
    val desktopId: String?,
    val incoming: Boolean,
    val text: String?,
    val time: Long = System.currentTimeMillis(),
)

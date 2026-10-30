package org.andromeda.link.core.domain

/** A monitor or window a desktop shares for remote control. */
data class RemoteSource(
    val name: String,
    val window: Boolean,
)

/** One unit of the desktop's stream: H.264 in Annex B form, or with [audio] one 48 kHz stereo Opus packet. */
class RemoteUnit(
    val ptsUs: Long,
    val flags: Int,
    val data: ByteArray,
) {
    val config get() = flags and MIRROR_CONFIG != 0
    val keyframe get() = flags and MIRROR_KEYFRAME != 0
    val audio get() = flags and MIRROR_AUDIO != 0
}

/** The desktop's screen and sound, from [RemoteEvent.Stream]. */
interface RemoteVideo {
    /** Null once the desktop ended the stream or [stop] ran. */
    suspend fun next(): RemoteUnit?

    suspend fun stop()
}

/** What a desktop answers this phone's request for its screen. */
sealed interface RemoteEvent {
    val desktopId: String

    data class Started(
        override val desktopId: String,
        val sources: List<RemoteSource>,
        val current: Int,
    ) : RemoteEvent

    class Stream(
        override val desktopId: String,
        val width: Int,
        val height: Int,
        val video: RemoteVideo,
    ) : RemoteEvent

    data class Stopped(
        override val desktopId: String,
        val reason: String?,
    ) : RemoteEvent
}

const val MIRROR_KEYFRAME = 1
const val MIRROR_CONFIG = 2
const val MIRROR_AUDIO = 4

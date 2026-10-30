package org.andromeda.link.control

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaFormat
import android.util.Log
import android.view.Surface
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.andromeda.link.core.domain.RemoteEvent
import org.andromeda.link.core.domain.RemoteUnit
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.coroutines.coroutineContext

/**
 * Plays a desktop's stream: H.264 into [surface] through the hardware decoder, and its Opus sound through an
 * [AudioTrack] unless [muted] says otherwise. Runs until the stream ends or the caller is cancelled.
 */
suspend fun play(
    stream: RemoteEvent.Stream,
    surface: Surface,
    muted: () -> Boolean,
) = withContext(Dispatchers.IO) {
    Log.i(TAG, "decoding ${stream.width}x${stream.height}")
    val codec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
    codec.configure(MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, stream.width, stream.height), surface, null, 0)
    codec.start()
    // The sound runs beside the picture; a full queue drops the oldest packets, so the sound never falls behind.
    val sound = Channel<RemoteUnit>(SOUND_QUEUE, BufferOverflow.DROP_OLDEST)
    try {
        coroutineScope {
            launch { playSound(sound) }
            while (coroutineContext.isActive) {
                val unit = stream.video.next() ?: break
                if (unit.audio) {
                    if (!muted()) sound.trySend(unit)
                } else {
                    decode(codec, unit)
                }
            }
            sound.close()
        }
    } finally {
        codec.stop()
        codec.release()
    }
}

private fun decode(
    codec: MediaCodec,
    unit: RemoteUnit,
) {
    var index = codec.dequeueInputBuffer(INPUT_WAIT_US)
    while (index < 0) {
        render(codec)
        index = codec.dequeueInputBuffer(INPUT_WAIT_US)
    }
    codec.getInputBuffer(index)?.apply {
        clear()
        put(unit.data)
    }
    val flags = if (unit.config) MediaCodec.BUFFER_FLAG_CODEC_CONFIG else 0
    codec.queueInputBuffer(index, 0, unit.data.size, unit.ptsUs, flags)
    render(codec)
}

/** Shows every decoded frame at once: the picture is live, not timed. */
private fun render(codec: MediaCodec) {
    val info = MediaCodec.BufferInfo()
    while (true) {
        val index = codec.dequeueOutputBuffer(info, 0)
        if (index >= 0) {
            codec.releaseOutputBuffer(index, true)
        } else if (index != MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
            return
        }
    }
}

private suspend fun playSound(packets: Channel<RemoteUnit>) {
    val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_OPUS, RATE, 2)
    format.setByteBuffer("csd-0", opusHead())
    format.setByteBuffer("csd-1", nanos(0))
    format.setByteBuffer("csd-2", nanos(SEEK_PREROLL_NS))
    val codec =
        try {
            MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_AUDIO_OPUS).apply {
                configure(format, null, null, 0)
                start()
            }
        } catch (error: Exception) {
            // Packets then pile up in the queue, which drops the oldest.
            Log.w(TAG, "no Opus decoder: ${error.message}")
            return
        }
    val track =
        AudioTrack
            .Builder()
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).build())
            .setAudioFormat(
                AudioFormat
                    .Builder()
                    .setSampleRate(RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .build(),
            ).setTransferMode(AudioTrack.MODE_STREAM)
            .build()
    track.play()
    var heard = false
    try {
        for (packet in packets) {
            val input = codec.dequeueInputBuffer(INPUT_WAIT_US)
            if (input >= 0) {
                codec.getInputBuffer(input)?.apply {
                    clear()
                    put(packet.data)
                }
                codec.queueInputBuffer(input, 0, packet.data.size, packet.ptsUs, 0)
            }
            val info = MediaCodec.BufferInfo()
            var output = codec.dequeueOutputBuffer(info, 0)
            while (output >= 0 || output == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                if (output >= 0) {
                    codec.getOutputBuffer(output)?.let { pcm ->
                        if (!heard) Log.i(TAG, "playing the desktop's sound")
                        heard = true
                        // Non-blocking: a late packet is dropped rather than delaying the ones after it.
                        track.write(pcm, info.size, AudioTrack.WRITE_NON_BLOCKING)
                    }
                    codec.releaseOutputBuffer(output, false)
                }
                output = codec.dequeueOutputBuffer(info, 0)
            }
        }
    } finally {
        track.stop()
        track.release()
        codec.stop()
        codec.release()
    }
}

/** RFC 7845's identification header for 48 kHz stereo with no pre-skip, which Android's Opus decoder wants. */
private fun opusHead(): ByteBuffer =
    ByteBuffer.allocate(OPUS_HEAD_SIZE).order(ByteOrder.LITTLE_ENDIAN).apply {
        put("OpusHead".toByteArray())
        put(1)
        put(2)
        putShort(0)
        putInt(RATE)
        putShort(0)
        put(0)
        flip()
    }

private fun nanos(value: Long): ByteBuffer =
    ByteBuffer.allocate(Long.SIZE_BYTES).order(ByteOrder.nativeOrder()).apply {
        putLong(value)
        flip()
    }

private const val TAG = "RemotePlayer"
private const val RATE = 48_000
private const val OPUS_HEAD_SIZE = 19
private const val SEEK_PREROLL_NS = 80_000_000L
private const val INPUT_WAIT_US = 10_000L
private const val SOUND_QUEUE = 25

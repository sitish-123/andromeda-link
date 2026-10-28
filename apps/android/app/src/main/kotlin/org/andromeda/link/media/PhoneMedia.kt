package org.andromeda.link.media

import android.content.ComponentName
import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.media.MediaMetadata
import android.media.session.MediaController
import android.media.session.MediaSessionManager
import androidx.core.net.toUri
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.MediaCommandKind
import org.andromeda.link.core.domain.MediaLimits
import org.andromeda.link.core.domain.MediaPlayer
import org.andromeda.link.core.domain.PhoneMediaCommand
import org.andromeda.link.core.domain.PlaybackState
import org.andromeda.link.core.domain.truncateUtf8
import org.andromeda.link.notifications.MirrorService
import java.io.ByteArrayOutputStream
import kotlin.math.abs
import kotlin.math.roundToInt
import android.media.session.PlaybackState as AndroidPlaybackState

/**
 * This phone's media sessions for desktops: every active session, each reported as its own player, so a desktop
 * sees them all and picks the one that plays. `getActiveSessions` is allowed to a notification listener, so this
 * works only while notification access is granted and is attached from [MirrorService]. A desktop's command goes to
 * the session it names.
 */
class PhoneMedia(
    private val context: Context,
    private val repository: LinkRepository,
    private val scope: CoroutineScope,
) {
    private val sessions = context.getSystemService(MediaSessionManager::class.java)
    private val listener = ComponentName(context, MirrorService::class.java)
    private val main = Handler(Looper.getMainLooper())

    /** Followed sessions by player id (the package name); a second session of the same app is not followed. */
    private val followed = LinkedHashMap<String, Followed>()

    /** One followed session: its controller, the last description desktops received, and its compressed artwork. */
    private inner class Followed(
        val controller: MediaController,
    ) {
        var sent: MediaPlayer? = null

        /** The last artwork compressed: its bitmap, the size it was cut to, and the JPEG. */
        var artwork: Triple<Bitmap, Int, ByteArray?>? = null

        /** The artwork URI decoded last and its bitmap, so a URI is read once. */
        var uriArt: Pair<String, Bitmap?>? = null
        val callback =
            object : MediaController.Callback() {
                override fun onPlaybackStateChanged(state: AndroidPlaybackState?) = update(this@Followed)

                override fun onMetadataChanged(metadata: MediaMetadata?) = update(this@Followed)

                override fun onAudioInfoChanged(info: MediaController.PlaybackInfo) = update(this@Followed)

                override fun onSessionDestroyed() = drop(controller.packageName)
            }
    }

    private val changed = MediaSessionManager.OnActiveSessionsChangedListener { follow(it.orEmpty()) }

    fun start() {
        // A desktop that came on Bluetooth needs smaller artwork, so every player is described afresh.
        scope.launch {
            repository.connections.collect {
                for (session in followed.values) {
                    val fresh = describe(session) ?: session.sent?.let { old -> old.copy(positionMs = position(old)) }
                    fresh?.let { publish(session, it) }
                }
            }
        }
        scope.launch { repository.mediaCommands.collect(::obey) }
    }

    fun attach() {
        runCatching {
            sessions.addOnActiveSessionsChangedListener(changed, listener, main)
            follow(sessions.getActiveSessions(listener))
        }.onFailure { Log.w(TAG, "media sessions: ${it.message}") }
    }

    fun detach() {
        runCatching { sessions.removeOnActiveSessionsChangedListener(changed) }
        follow(emptyList())
    }

    private fun follow(active: List<MediaController>) {
        // This app's own sessions show desktops' players; reporting them would send them back where they came from.
        val others = active.filter { it.packageName != context.packageName }
        val byPlayer = others.distinctBy { it.packageName }.associateBy { it.packageName }
        for (player in followed.keys.toList()) {
            if (byPlayer[player]?.sessionToken != followed[player]?.controller?.sessionToken) drop(player)
        }
        for ((player, controller) in byPlayer) {
            if (player in followed) continue
            val session = Followed(controller)
            followed[player] = session
            controller.registerCallback(session.callback, main)
            update(session)
        }
    }

    private fun drop(player: String) {
        val session = followed.remove(player) ?: return
        session.controller.unregisterCallback(session.callback)
        session.sent?.let { gone -> scope.launch { repository.playerGone(gone.player) } }
    }

    private fun update(session: Followed) {
        val player = describe(session) ?: return
        val previous = session.sent
        if (previous != null && !differs(previous, player)) return
        publish(session, player)
    }

    /** A new description is worth sending when anything but the steady advance of the position changed. */
    private fun differs(
        old: MediaPlayer,
        new: MediaPlayer,
    ): Boolean =
        old.copy(positionMs = 0, receivedAt = 0) != new.copy(positionMs = 0, receivedAt = 0) ||
            abs(position(old, new.receivedAt) - new.positionMs) > SEEK_TOLERANCE_MS

    private fun publish(
        session: Followed,
        player: MediaPlayer,
    ) {
        session.sent = player
        scope.launch { repository.publishPlayer(player).onFailure { Log.w(TAG, "publishing: ${it.message}") } }
    }

    private fun position(
        player: MediaPlayer,
        at: Long = SystemClock.elapsedRealtime(),
    ): Long {
        val advanced = if (player.state == PlaybackState.Playing) player.positionMs + (at - player.receivedAt) else player.positionMs
        return player.lengthMs?.let { advanced.coerceAtMost(it) } ?: advanced
    }

    private fun describe(session: Followed): MediaPlayer? {
        val controller = session.controller
        val playback = controller.playbackState ?: return null
        val metadata = controller.metadata
        val now = SystemClock.elapsedRealtime()
        val state =
            when (playback.state) {
                AndroidPlaybackState.STATE_PLAYING, AndroidPlaybackState.STATE_BUFFERING -> PlaybackState.Playing
                AndroidPlaybackState.STATE_PAUSED -> PlaybackState.Paused
                else -> PlaybackState.Stopped
            }
        val position =
            if (state == PlaybackState.Playing && playback.lastPositionUpdateTime > 0) {
                playback.position + ((now - playback.lastPositionUpdateTime) * playback.playbackSpeed).toLong()
            } else {
                playback.position
            }
        val info = controller.playbackInfo
        val volume =
            info
                .takeIf { it.volumeControl != android.media.VolumeProvider.VOLUME_CONTROL_FIXED && it.maxVolume > 0 }
                ?.let { (it.currentVolume * 100f / it.maxVolume).roundToInt() }
        val label =
            runCatching {
                context.packageManager.getApplicationLabel(context.packageManager.getApplicationInfo(controller.packageName, 0))
            }.getOrNull()?.toString() ?: controller.packageName
        val text = { key: String -> metadata?.getString(key).orEmpty().truncateUtf8(MediaLimits.METADATA) }
        return MediaPlayer(
            player = controller.packageName.truncateUtf8(MediaLimits.PLAYER),
            name = label.ifBlank { controller.packageName }.truncateUtf8(MediaLimits.PLAYER),
            state = state,
            title = text(MediaMetadata.METADATA_KEY_TITLE),
            artist = text(MediaMetadata.METADATA_KEY_ARTIST),
            album = text(MediaMetadata.METADATA_KEY_ALBUM),
            lengthMs = metadata?.getLong(MediaMetadata.METADATA_KEY_DURATION)?.takeIf { it > 0 },
            positionMs = position.coerceAtLeast(0),
            volume = volume,
            artwork = art(session, metadata),
            can = commands(playback.actions, volume != null),
            receivedAt = now,
        )
    }

    private fun commands(
        actions: Long,
        volume: Boolean,
    ): Set<MediaCommandKind> =
        buildSet {
            val has = { action: Long -> actions and action != 0L }
            if (has(AndroidPlaybackState.ACTION_PLAY)) add(MediaCommandKind.Play)
            if (has(AndroidPlaybackState.ACTION_PAUSE)) add(MediaCommandKind.Pause)
            if (has(AndroidPlaybackState.ACTION_PLAY_PAUSE) ||
                has(AndroidPlaybackState.ACTION_PLAY) ||
                has(AndroidPlaybackState.ACTION_PAUSE)
            ) {
                add(MediaCommandKind.PlayPause)
            }
            if (has(AndroidPlaybackState.ACTION_SKIP_TO_NEXT)) add(MediaCommandKind.Next)
            if (has(AndroidPlaybackState.ACTION_SKIP_TO_PREVIOUS)) add(MediaCommandKind.Previous)
            if (has(AndroidPlaybackState.ACTION_SEEK_TO)) add(MediaCommandKind.Seek)
            if (volume) add(MediaCommandKind.Volume)
        }

    /**
     * The artwork as a JPEG of at most 256 px, or 128 px while a desktop is connected over Bluetooth, and the
     * protocol's size; compressed once per bitmap and size.
     */
    private fun art(
        session: Followed,
        metadata: MediaMetadata?,
    ): ByteArray? {
        val bitmap =
            metadata?.getBitmap(MediaMetadata.METADATA_KEY_ART)
                ?: metadata?.getBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART)
                ?: artFromUri(session, metadata)
                ?: return null
        val px = if (repository.desktops.value.any { it.connected && it.bluetooth }) BLUETOOTH_ART_PX else ART_PX
        session.artwork?.takeIf { it.first === bitmap && it.second == px }?.let { return it.third }
        val scale = px.toFloat() / maxOf(bitmap.width, bitmap.height)
        val sized =
            if (scale < 1f) {
                Bitmap.createScaledBitmap(bitmap, (bitmap.width * scale).roundToInt(), (bitmap.height * scale).roundToInt(), true)
            } else {
                bitmap
            }
        val jpeg =
            generateSequence(85) { it - 15 }
                .takeWhile { it > 0 }
                .map { quality ->
                    ByteArrayOutputStream().also { sized.compress(Bitmap.CompressFormat.JPEG, quality, it) }.toByteArray()
                }.firstOrNull { it.size <= MediaLimits.ARTWORK }
        session.artwork = Triple(bitmap, px, jpeg)
        return jpeg
    }

    /** Artwork the session gives only as a URI, decoded at most [ART_PX] wide; the last one is reused per URI. */
    private fun artFromUri(
        session: Followed,
        metadata: MediaMetadata?,
    ): Bitmap? {
        val uri =
            metadata?.getString(MediaMetadata.METADATA_KEY_ART_URI)?.takeIf { it.isNotBlank() }
                ?: metadata?.getString(MediaMetadata.METADATA_KEY_ALBUM_ART_URI)?.takeIf { it.isNotBlank() }
                ?: return null
        if (session.uriArt?.first == uri) return session.uriArt?.second
        val bitmap =
            runCatching {
                val source = ImageDecoder.createSource(context.contentResolver, uri.toUri())
                ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
                    val scale = ART_PX.toFloat() / maxOf(info.size.width, info.size.height)
                    if (scale < 1f) decoder.setTargetSize((info.size.width * scale).roundToInt(), (info.size.height * scale).roundToInt())
                    decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
                }
            }.onFailure { Log.i(TAG, "artwork $uri: ${it.message}") }.getOrNull()
        session.uriArt = uri to bitmap
        return bitmap
    }

    private fun obey(command: PhoneMediaCommand) {
        val session = followed[command.player] ?: return
        if (session.sent?.can?.contains(command.command) != true) return
        val target = session.controller
        val controls = target.transportControls
        val value = command.value ?: 0
        when (command.command) {
            MediaCommandKind.Play -> controls.play()
            MediaCommandKind.Pause -> controls.pause()
            MediaCommandKind.PlayPause ->
                if (target.playbackState?.state == AndroidPlaybackState.STATE_PLAYING) controls.pause() else controls.play()
            MediaCommandKind.Next -> controls.skipToNext()
            MediaCommandKind.Previous -> controls.skipToPrevious()
            MediaCommandKind.Seek -> controls.seekTo(value)
            MediaCommandKind.Volume -> {
                val max = target.playbackInfo.maxVolume
                target.setVolumeTo((value * max / 100f).roundToInt(), 0)
            }
        }
    }

    private companion object {
        const val TAG = "PhoneMedia"
        const val ART_PX = 256
        const val BLUETOOTH_ART_PX = 128
        const val SEEK_TOLERANCE_MS = 2000
    }
}

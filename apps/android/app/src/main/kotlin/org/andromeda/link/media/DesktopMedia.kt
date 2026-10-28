package org.andromeda.link.media

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.os.SystemClock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import org.andromeda.link.MainActivity
import org.andromeda.link.R
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.DesktopPlayer
import org.andromeda.link.core.domain.MediaCommandKind
import org.andromeda.link.core.domain.MediaPlayer
import org.andromeda.link.core.domain.PlaybackState
import org.andromeda.link.notifications.Channels
import android.media.session.PlaybackState as AndroidPlaybackState

/**
 * One Android media session per connected desktop, so its player shows in the system media card, on the lock screen,
 * and on headset buttons. The session shows the desktop player that plays; when it stops, another that plays; while
 * all are paused, the last one shown (as KDE Connect does). Its transport controls send `media-command`.
 */
class DesktopMedia(
    private val context: Context,
    private val repository: LinkRepository,
    private val scope: CoroutineScope,
) {
    private val notifications = context.getSystemService(NotificationManager::class.java)

    /** A desktop's session and the player it shows. */
    private inner class Shown(
        val desktopId: String,
    ) {
        val session = MediaSession(context, "link-desktop-$desktopId")
        var player: MediaPlayer? = null

        init {
            session.setCallback(
                object : MediaSession.Callback() {
                    override fun onPlay() = send(MediaCommandKind.Play)

                    override fun onPause() = send(MediaCommandKind.Pause)

                    override fun onSkipToNext() = send(MediaCommandKind.Next)

                    override fun onSkipToPrevious() = send(MediaCommandKind.Previous)

                    override fun onSeekTo(pos: Long) = send(MediaCommandKind.Seek, pos)
                },
            )
        }

        fun send(
            command: MediaCommandKind,
            value: Long? = null,
        ) {
            val player = player ?: return
            if (command !in player.can) return
            scope.launch { repository.mediaCommand(desktopId, player.player, command, value) }
        }
    }

    private val shown = HashMap<String, Shown>()

    fun start() {
        scope.launch {
            combine(repository.desktopPlayers, repository.desktops) { players, desktops ->
                val connected = desktops.filter { it.connected }.associateBy { it.id }
                players
                    .filter { it.desktopId in connected }
                    .groupBy { it.desktopId }
                    .mapValues { (id, list) -> connected.getValue(id).name to list }
            }.collect(::show)
        }
    }

    private fun show(desktops: Map<String, Pair<String, List<DesktopPlayer>>>) {
        for (id in shown.keys - desktops.keys) {
            shown.remove(id)?.session?.release()
            notifications.cancel(notificationId(id))
        }
        for ((id, entry) in desktops) {
            val (desktopName, players) = entry
            val target = shown.getOrPut(id) { Shown(id) }
            val list = players.map { it.player }
            val pick =
                list.lastOrNull { it.state == PlaybackState.Playing }
                    ?: list.firstOrNull { it.player == target.player?.player }
                    ?: list.lastOrNull()
                    ?: continue
            if (pick == target.player) continue
            target.player = pick
            describe(target, pick)
            notify(id, desktopName, target, pick)
        }
    }

    private fun describe(
        target: Shown,
        player: MediaPlayer,
    ) {
        val metadata =
            MediaMetadata
                .Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, player.title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, player.artist)
                .putString(MediaMetadata.METADATA_KEY_ALBUM, player.album)
                .apply { player.lengthMs?.let { putLong(MediaMetadata.METADATA_KEY_DURATION, it) } }
                .apply {
                    player.artwork
                        ?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }
                        ?.let { putBitmap(MediaMetadata.METADATA_KEY_ART, it) }
                }.build()
        target.session.setMetadata(metadata)
        val playing = player.state == PlaybackState.Playing
        val state =
            when (player.state) {
                PlaybackState.Playing -> AndroidPlaybackState.STATE_PLAYING
                PlaybackState.Paused -> AndroidPlaybackState.STATE_PAUSED
                PlaybackState.Stopped -> AndroidPlaybackState.STATE_STOPPED
            }
        // The position was true at receivedAt; Android moves the seek bar on from there at the given speed.
        val at = player.receivedAt.takeIf { it > 0 } ?: SystemClock.elapsedRealtime()
        target.session.setPlaybackState(
            AndroidPlaybackState
                .Builder()
                .setState(state, player.positionMs, if (playing) 1f else 0f, at)
                .setActions(actions(player.can))
                .build(),
        )
        target.session.isActive = true
    }

    private fun actions(can: Set<MediaCommandKind>): Long {
        var actions = 0L
        if (MediaCommandKind.Play in can) actions = actions or AndroidPlaybackState.ACTION_PLAY
        if (MediaCommandKind.Pause in can) actions = actions or AndroidPlaybackState.ACTION_PAUSE
        if (MediaCommandKind.PlayPause in can) actions = actions or AndroidPlaybackState.ACTION_PLAY_PAUSE
        if (MediaCommandKind.Next in can) actions = actions or AndroidPlaybackState.ACTION_SKIP_TO_NEXT
        if (MediaCommandKind.Previous in can) actions = actions or AndroidPlaybackState.ACTION_SKIP_TO_PREVIOUS
        if (MediaCommandKind.Seek in can) actions = actions or AndroidPlaybackState.ACTION_SEEK_TO
        return actions
    }

    private fun notify(
        desktopId: String,
        desktopName: String,
        target: Shown,
        player: MediaPlayer,
    ) {
        val open =
            PendingIntent.getActivity(
                context,
                0,
                Intent(context, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE,
            )
        val notification =
            Notification
                .Builder(context, Channels.DESKTOP_MEDIA)
                .setSmallIcon(R.drawable.ic_stat_link)
                .setContentTitle(player.title)
                .setContentText(player.artist)
                .setSubText(desktopName)
                .setContentIntent(open)
                .setOngoing(player.state == PlaybackState.Playing)
                .setStyle(Notification.MediaStyle().setMediaSession(target.session.sessionToken))
                .build()
        runCatching { notifications.notify(notificationId(desktopId), notification) }
    }

    private fun notificationId(desktopId: String): Int = NOTIFICATION_BASE + (desktopId.hashCode() and 0xffff)

    private companion object {
        const val NOTIFICATION_BASE = 0x4d000
    }
}

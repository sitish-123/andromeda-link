package org.andromeda.link.widget

import android.app.Application
import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.text.format.DateUtils
import android.util.Log
import android.view.View
import android.widget.RemoteViews
import androidx.core.graphics.scale
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch
import org.andromeda.link.LinkApplication
import org.andromeda.link.MainActivity
import org.andromeda.link.R
import org.andromeda.link.clipboard.ClipboardReadActivity
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.MediaCommandKind
import org.andromeda.link.core.domain.MediaPlayer
import org.andromeda.link.core.domain.PlaybackState
import org.andromeda.link.devices.CAN_CONTROL

/** The home screen widget: the first connected desktop (else the first paired one), its player and four keys. */
class DesktopWidget(
    private val application: Application,
    private val repository: LinkRepository,
    private val scope: CoroutineScope,
) {
    fun start() {
        scope.launch {
            combine(repository.desktops, repository.desktopStatuses, repository.capabilities, repository.desktopPlayers) {
                desktops,
                statuses,
                can,
                players,
                ->
                // Player positions tick; only what the widget shows triggers a render.
                val shown =
                    players.map {
                        listOf(it.desktopId, it.player.title, it.player.artist, it.player.state, it.player.artwork?.contentHashCode())
                    }
                listOf(desktops, statuses, can, shown)
            }.distinctUntilChanged().collect { render(application) }
        }
    }

    companion object {
        private const val ART_PX = 160
        private val OPTIONAL =
            listOf(
                R.id.widget_battery_glyph,
                R.id.widget_battery,
                R.id.widget_charging,
                R.id.widget_locked,
                R.id.widget_player,
                R.id.widget_seen,
            )

        fun render(context: Context) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, DesktopWidgetProvider::class.java))
            if (ids.isEmpty()) return
            manager.updateAppWidget(ids, views(context))
        }

        private fun views(context: Context): RemoteViews {
            val repository = (context.applicationContext as LinkApplication).container.repository
            val views = RemoteViews(context.packageName, R.layout.widget_desktop)
            val open = PendingIntent.getActivity(context, 0, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
            views.setOnClickPendingIntent(R.id.widget_root, open)
            // The launcher reapplies an update onto the views it already shows, so every render sets every state.
            OPTIONAL.forEach { views.setViewVisibility(it, View.GONE) }
            val desktops = repository.desktops.value
            val desktop = desktops.firstOrNull { it.connected } ?: desktops.firstOrNull()
            if (desktop == null) {
                views.setTextViewText(R.id.widget_name, context.getString(R.string.widget_no_desktop))
                listOf(R.id.widget_key_lock, R.id.widget_key_sleep, R.id.widget_key_ring, R.id.widget_key_clipboard)
                    .forEach { key(views, it, false, open) }
                return views
            }
            val live = desktop.connected
            views.setTextViewText(R.id.widget_name, desktop.name)

            val status = repository.desktopStatuses.value[desktop.id]
            status?.battery?.let { percent ->
                views.setViewVisibility(R.id.widget_battery_glyph, View.VISIBLE)
                val glyph = if (live) R.drawable.widget_battery else R.drawable.widget_battery_off
                views.setImageViewResource(R.id.widget_battery_glyph, glyph)
                views.setViewVisibility(R.id.widget_battery, View.VISIBLE)
                views.setTextViewText(R.id.widget_battery, context.getString(R.string.home_battery, percent))
                if (live && status.charging == true) views.setViewVisibility(R.id.widget_charging, View.VISIBLE)
            }
            status?.locked?.let { locked ->
                views.setViewVisibility(R.id.widget_locked, View.VISIBLE)
                views.setImageViewResource(R.id.widget_locked, if (locked) R.drawable.widget_lock else R.drawable.widget_lock_open)
                views.setContentDescription(
                    R.id.widget_locked,
                    context.getString(if (locked) R.string.home_locked else R.string.home_unlocked),
                )
            }

            val player = shownPlayer(repository, desktop.id)
            if (live && player != null) {
                views.setViewVisibility(R.id.widget_player, View.VISIBLE)
                views.setTextViewText(R.id.widget_title, player.title.ifEmpty { player.name })
                views.setTextViewText(
                    R.id.widget_artist,
                    if (player.artist.isEmpty()) player.name else context.getString(R.string.home_on_app, player.artist, player.name),
                )
                views.setImageViewBitmap(R.id.widget_art, player.artwork?.let(::art))
                val playing = player.state == PlaybackState.Playing
                views.setImageViewResource(R.id.widget_play, if (playing) R.drawable.widget_pause else R.drawable.widget_play)
                views.setContentDescription(R.id.widget_play, context.getString(if (playing) R.string.media_pause else R.string.media_play))
                views.setOnClickPendingIntent(R.id.widget_play, WidgetActionReceiver.intent(context, WidgetActionReceiver.PLAY, desktop.id))
            }
            if (!live) {
                views.setViewVisibility(R.id.widget_seen, View.VISIBLE)
                val seen =
                    if (desktop.lastSeen <= 0) {
                        context.getString(R.string.last_seen_never)
                    } else {
                        DateUtils.getRelativeTimeSpanString(desktop.lastSeen * 1000).toString()
                    }
                views.setTextViewText(R.id.widget_seen, context.getString(R.string.home_last_seen, seen))
            }

            val can = repository.capabilities.value[desktop.id].orEmpty()
            val control = live && CAN_CONTROL in can
            val action = { name: String -> WidgetActionReceiver.intent(context, name, desktop.id) }
            key(views, R.id.widget_key_lock, control && "lock" in can, action(WidgetActionReceiver.LOCK), open)
            key(views, R.id.widget_key_sleep, control && "suspend" in can, action(WidgetActionReceiver.SLEEP), open)
            key(views, R.id.widget_key_ring, live, action(WidgetActionReceiver.RING), open)
            val clipboard =
                PendingIntent.getActivity(
                    context,
                    1,
                    Intent(context, ClipboardReadActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                    PendingIntent.FLAG_IMMUTABLE,
                )
            key(views, R.id.widget_key_clipboard, live && desktop.sharing.clipboard, clipboard, open)
            return views
        }

        /** An off key is outlined and faint, and opens the app. */
        private fun key(
            views: RemoteViews,
            id: Int,
            on: Boolean,
            action: PendingIntent,
            open: PendingIntent = action,
        ) {
            views.setInt(id, "setBackgroundResource", if (on) R.drawable.widget_key else R.drawable.widget_key_off)
            views.setInt(id, "setImageAlpha", if (on) 255 else 90)
            views.setOnClickPendingIntent(id, if (on) action else open)
        }

        /** As on Home: a playing player first. */
        fun shownPlayer(
            repository: LinkRepository,
            desktopId: String,
        ): MediaPlayer? =
            repository.desktopPlayers.value
                .filter { it.desktopId == desktopId }
                .let { all -> all.firstOrNull { it.player.state == PlaybackState.Playing } ?: all.firstOrNull() }
                ?.player

        private fun art(bytes: ByteArray): Bitmap? =
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.let {
                if (it.width > ART_PX) it.scale(ART_PX, ART_PX * it.height / it.width) else it
            }
    }
}

class DesktopWidgetProvider : AppWidgetProvider() {
    override fun onUpdate(
        context: Context,
        appWidgetManager: AppWidgetManager,
        appWidgetIds: IntArray,
    ) = DesktopWidget.render(context)
}

/** The widget's keys act on the desktop it shows. */
class WidgetActionReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        val repository = (context.applicationContext as LinkApplication).container.repository
        val id = intent.getStringExtra(EXTRA_DESKTOP) ?: return
        val done = goAsync()
        CoroutineScope(SupervisorJob() + Dispatchers.Default).launch {
            val result =
                when (intent.action) {
                    LOCK -> repository.lockDesktop(id)
                    SLEEP -> repository.suspendDesktop(id)
                    RING -> repository.ringDesktop(id, true)
                    PLAY ->
                        DesktopWidget.shownPlayer(repository, id)?.let {
                            repository.mediaCommand(id, it.player, MediaCommandKind.PlayPause, null)
                        }
                    else -> null
                }
            result?.onFailure { Log.w("LinkWidget", "${intent.action} on $id: ${it.message}") }
            done.finish()
        }
    }

    companion object {
        const val LOCK = "org.andromeda.link.widget.LOCK"
        const val SLEEP = "org.andromeda.link.widget.SLEEP"
        const val RING = "org.andromeda.link.widget.RING"
        const val PLAY = "org.andromeda.link.widget.PLAY"
        private const val EXTRA_DESKTOP = "desktop"

        fun intent(
            context: Context,
            action: String,
            desktopId: String,
        ): PendingIntent =
            PendingIntent.getBroadcast(
                context,
                action.hashCode(),
                Intent(context, WidgetActionReceiver::class.java).setAction(action).putExtra(EXTRA_DESKTOP, desktopId),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
    }
}

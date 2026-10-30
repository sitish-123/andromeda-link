package org.andromeda.link.presence

import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.andromeda.link.LinkApplication
import org.andromeda.link.MainActivity
import org.andromeda.link.R
import org.andromeda.link.clipboard.ClipboardReadActivity
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.notifications.Channels

/**
 * Runs only while "Stay connected" is on, so its notification exists only then. It holds no state: [Presence] keeps
 * the phone present and the notification current, and this service only lets that continue in the background.
 */
class PresenceService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int {
        val container = (application as LinkApplication).container
        ServiceCompat.startForeground(
            this,
            NOTIFICATION_ID,
            notification(this, container.repository),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
        )
        container.clipboardWatcher.start()
        return START_STICKY
    }

    override fun onDestroy() {
        (application as LinkApplication).container.clipboardWatcher.stop()
        super.onDestroy()
    }

    companion object {
        const val NOTIFICATION_ID = 1
        private const val GROUP = "presence"

        /** "Connected to Arch" with Send clipboard, Lock (where granted), and Ring, or "Looking for Arch". */
        fun notification(
            context: Context,
            repository: LinkRepository,
        ): Notification {
            val desktops = repository.desktops.value
            val connected = desktops.filter { it.connected }
            val title =
                if (connected.isNotEmpty()) {
                    context.getString(R.string.presence_connected, connected.joinToString { it.name })
                } else {
                    context.getString(R.string.presence_looking, desktops.joinToString { it.name })
                }
            val builder =
                NotificationCompat
                    .Builder(context, Channels.PRESENCE)
                    .setSmallIcon(R.drawable.ic_stat_link)
                    .setContentTitle(title)
                    .setContentIntent(
                        PendingIntent.getActivity(context, 0, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE),
                    ).setOngoing(true)
                    // Its own group, so Android does not bundle it with incoming shares and hide their actions.
                    .setGroup(GROUP)
                    .setSilent(true)
            if (connected.isNotEmpty()) {
                val sendClipboard =
                    PendingIntent.getActivity(
                        context,
                        1,
                        Intent(context, ClipboardReadActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                        PendingIntent.FLAG_IMMUTABLE,
                    )
                builder.addAction(0, context.getString(R.string.clipboard_tile), sendClipboard)
                if (DesktopActions.controllable(repository).isNotEmpty()) {
                    builder.addAction(
                        0,
                        context.getString(R.string.presence_lock),
                        PresenceActionReceiver.intent(context, PresenceActionReceiver.LOCK),
                    )
                }
                builder.addAction(
                    0,
                    context.getString(R.string.presence_ring),
                    PresenceActionReceiver.intent(context, PresenceActionReceiver.RING),
                )
            }
            return builder.build()
        }
    }
}

/** Lock and Ring act on every connected desktop. */
class PresenceActionReceiver : BroadcastReceiver() {
    override fun onReceive(
        context: Context,
        intent: Intent,
    ) {
        val container = (context.applicationContext as LinkApplication).container
        val done = goAsync()
        CoroutineScope(SupervisorJob() + Dispatchers.Default).launch {
            when (intent.action) {
                LOCK -> DesktopActions.lock(container.repository)
                RING -> DesktopActions.ring(container.repository)
            }
            done.finish()
        }
    }

    companion object {
        const val RING = "org.andromeda.link.presence.RING"
        const val LOCK = "org.andromeda.link.presence.LOCK"

        fun intent(
            context: Context,
            action: String,
        ): PendingIntent =
            PendingIntent.getBroadcast(
                context,
                action.hashCode(),
                Intent(context, PresenceActionReceiver::class.java).setAction(action),
                PendingIntent.FLAG_IMMUTABLE,
            )
    }
}

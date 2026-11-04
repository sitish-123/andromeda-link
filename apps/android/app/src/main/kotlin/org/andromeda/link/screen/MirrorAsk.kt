package org.andromeda.link.screen

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import org.andromeda.link.R
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.notifications.Channels

/**
 * A desktop asked to mirror while its Screen switch is off: a notification asks the user. Allow turns the switch on and
 * answers the request; Not now refuses it, so the desktop stops waiting.
 */
class MirrorAsk(
    private val context: Context,
    private val repository: LinkRepository,
    private val scope: CoroutineScope,
) {
    private val notifications = context.getSystemService(NotificationManager::class.java)

    fun start() {
        val receiver =
            object : BroadcastReceiver() {
                override fun onReceive(
                    context: Context,
                    intent: Intent,
                ) {
                    val desktopId = intent.getStringExtra(EXTRA_DESKTOP) ?: return
                    notifications.cancel(NOTIFICATION)
                    scope.launch {
                        if (intent.action == ACTION_ALLOW) {
                            repository.allowMirror(desktopId)
                        } else {
                            repository.stopMirror(desktopId, context.getString(R.string.screen_ask_refused))
                        }
                    }
                }
            }
        val filter = IntentFilter().apply { listOf(ACTION_ALLOW, ACTION_REFUSE).forEach(::addAction) }
        ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        scope.launch { repository.mirrorBlocked.collect(::ask) }
    }

    private fun ask(desktopId: String) {
        val channel = notifications.getNotificationChannel(Channels.SCREEN)
        if (!notifications.areNotificationsEnabled() || channel?.importance == NotificationManager.IMPORTANCE_NONE) {
            scope.launch { repository.stopMirror(desktopId, context.getString(R.string.screen_ask_switch_off)) }
            return
        }
        val name =
            repository.desktops.value
                .firstOrNull { it.id == desktopId }
                ?.name ?: desktopId
        val notification =
            NotificationCompat
                .Builder(context, Channels.SCREEN)
                .setSmallIcon(R.drawable.ic_stat_link)
                .setContentTitle(context.getString(R.string.screen_ask_title, name))
                .setContentText(context.getString(R.string.screen_ask_text))
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setCategory(NotificationCompat.CATEGORY_CALL)
                .setTimeoutAfter(TIMEOUT_MS)
                .setAutoCancel(true)
                .addAction(0, context.getString(R.string.screen_allow), action(ACTION_ALLOW, desktopId))
                .addAction(0, context.getString(R.string.screen_ask_not_now), action(ACTION_REFUSE, desktopId))
                .build()
        notifications.notify(NOTIFICATION, notification)
    }

    private fun action(
        name: String,
        desktopId: String,
    ): PendingIntent =
        PendingIntent.getBroadcast(
            context,
            name.hashCode(),
            Intent(name).setPackage(context.packageName).putExtra(EXTRA_DESKTOP, desktopId),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    private companion object {
        const val ACTION_ALLOW = "org.andromeda.link.MIRROR_ALLOW"
        const val ACTION_REFUSE = "org.andromeda.link.MIRROR_REFUSE"
        const val EXTRA_DESKTOP = "desktop"
        const val NOTIFICATION = 43

        /** The desktop stops waiting after its 60 s consent timeout (link-daemon mirror.rs). */
        const val TIMEOUT_MS = 60_000L
    }
}

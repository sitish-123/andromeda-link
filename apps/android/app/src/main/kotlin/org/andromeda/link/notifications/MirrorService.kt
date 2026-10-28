package org.andromeda.link.notifications

import android.content.ComponentName
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import org.andromeda.link.LinkApplication

/**
 * Bound by Android while the user grants notification access, which also opens the media sessions to
 * [org.andromeda.link.media.PhoneMedia]. It holds no state: every callback goes to the process-wide objects.
 */
class MirrorService : NotificationListenerService() {
    private val container get() = (application as LinkApplication).container

    override fun onListenerConnected() {
        container.mirror.attach(this)
        container.media.attach()
    }

    /** Android unbinds a listener under memory pressure too; while access holds, it binds again on this request. */
    override fun onListenerDisconnected() {
        container.mirror.detach(this)
        container.media.detach()
        requestRebind(ComponentName(this, MirrorService::class.java))
    }

    override fun onNotificationPosted(
        sbn: StatusBarNotification,
        rankingMap: RankingMap,
    ) = container.mirror.posted(sbn, rankingMap)

    override fun onNotificationRemoved(sbn: StatusBarNotification) = container.mirror.removed(sbn)
}

package org.andromeda.link.notifications

import android.app.ActivityOptions
import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationManager
import android.app.RemoteInput
import android.content.BroadcastReceiver
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.service.notification.NotificationListenerService.RankingMap
import android.service.notification.StatusBarNotification
import android.util.Log
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.edit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import org.andromeda.link.R
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.NotificationButton
import org.andromeda.link.core.domain.NotificationCommand
import org.andromeda.link.core.domain.NotificationLimits
import org.andromeda.link.core.domain.PhoneNotification
import org.andromeda.link.core.domain.truncateUtf8
import java.io.ByteArrayOutputStream

/**
 * Mirrors the notifications the user would see to connected desktops, and runs what desktops ask of them. It reports
 * the whole set when the listener binds, the filter changes, or the phone unlocks; the core sends each desktop only
 * what changed, also on connect.
 */
class NotificationMirror(
    private val context: Context,
    private val repository: LinkRepository,
    private val scope: CoroutineScope,
) {
    private val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val keyguard = context.getSystemService(KeyguardManager::class.java)
    private var listener: NotificationListenerService? = null

    /** Keys posted to desktops and not removed since. */
    private val mirrored = mutableSetOf<String>()
    private val icons = HashMap<String, ByteArray?>()
    private val _excluded = MutableStateFlow(prefs.getStringSet(KEY_EXCLUDED, null).orEmpty().toSet())
    private val _granted = MutableStateFlow(false)

    /** Packages the user chose not to mirror; every app is mirrored by default. */
    val excluded: StateFlow<Set<String>> = _excluded.asStateFlow()

    private val _lowImportance = MutableStateFlow(prefs.getStringSet(KEY_LOW_IMPORTANCE, null).orEmpty().toSet())

    /** Packages whose low-importance notifications the user chose to mirror. */
    val lowImportance: StateFlow<Set<String>> = _lowImportance.asStateFlow()

    /** Whether Android has bound the listener, which it does only while notification access is granted. */
    val granted: StateFlow<Boolean> = _granted.asStateFlow()

    /** Why one-time codes arrive without their text, on Android 15 and later; null before. */
    val sensitiveHidden: String? =
        context.getString(R.string.mirror_sensitive_hidden).takeIf { Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM }

    /** The user turned notification access off; false while Android has yet to bind a listener it may bind. */
    val accessOff: Flow<Boolean> =
        granted.map { bound ->
            !bound && context.packageName !in NotificationManagerCompat.getEnabledListenerPackages(context)
        }

    /**
     * Whether desktops get the full text while the phone is locked; off, they get what the lock screen shows, and the
     * full text once the phone unlocks. On by default.
     */
    private val fullWhileLocked get() = prefs.getBoolean(KEY_FULL_WHILE_LOCKED, true)

    fun start() {
        scope.launch { repository.notificationCommands.collect(::run) }
        val unlocked =
            object : BroadcastReceiver() {
                override fun onReceive(
                    context: Context,
                    intent: Intent,
                ) {
                    if (!fullWhileLocked) resync()
                }
            }
        context.registerReceiver(unlocked, IntentFilter(Intent.ACTION_USER_PRESENT))
    }

    fun attach(service: NotificationListenerService) {
        listener = service
        _granted.value = true
        resync()
    }

    fun detach(service: NotificationListenerService) {
        if (listener == service) listener = null
        _granted.value = false
    }

    fun posted(
        sbn: StatusBarNotification,
        ranking: RankingMap?,
    ) {
        val notification = build(sbn, ranking)
        if (notification == null) {
            if (mirrored.remove(sbn.key)) remove(sbn.key)
            return
        }
        mirrored += sbn.key
        scope.launch { repository.postNotification(notification).onFailure { log("posting", it) } }
    }

    fun removed(sbn: StatusBarNotification) {
        if (mirrored.remove(sbn.key)) remove(sbn.key)
    }

    fun setExcluded(
        packageName: String,
        exclude: Boolean,
    ) {
        val next = if (exclude) _excluded.value + packageName else _excluded.value - packageName
        prefs.edit { putStringSet(KEY_EXCLUDED, next) }
        _excluded.value = next
        resync()
    }

    /** Mirrors [packageName]'s low-importance notifications too; off for every app by default. */
    fun setLowImportance(
        packageName: String,
        on: Boolean,
    ) {
        val next = if (on) _lowImportance.value + packageName else _lowImportance.value - packageName
        prefs.edit { putStringSet(KEY_LOW_IMPORTANCE, next) }
        _lowImportance.value = next
        resync()
    }

    /** Settings' page for this listener, where the user grants access. */
    fun accessSettings(): Intent {
        val component = ComponentName(context, MirrorService::class.java)
        val intent =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
                    .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, component.flattenToString())
            } else {
                Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
            }
        return intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }

    /** Reports every shown notification; a changed one replaces the desktop's copy without a sound. */
    private fun resync() {
        val service = listener ?: return
        val active = runCatching { service.activeNotifications }.getOrNull() ?: return
        val ranking = runCatching { service.currentRanking }.getOrNull()
        val shown = active.mapNotNull { sbn -> build(sbn, ranking, alertOnce = true) }
        mirrored.clear()
        mirrored += shown.map { it.id }
        scope.launch { repository.syncNotifications(shown).onFailure { log("syncing", it) } }
    }

    private fun remove(key: String) {
        scope.launch { repository.removeNotification(key).onFailure { log("removing", it) } }
    }

    private fun build(
        sbn: StatusBarNotification,
        rankingMap: RankingMap?,
        alertOnce: Boolean = false,
    ): PhoneNotification? {
        val notification = sbn.notification
        if (sbn.packageName == context.packageName || sbn.packageName in _excluded.value) return null
        if (sbn.isOngoing || notification.flags and Notification.FLAG_GROUP_SUMMARY != 0) return null
        val ranking = rankingMap?.let { map -> NotificationListenerService.Ranking().takeIf { map.getRanking(sbn.key, it) } }
        val low = ranking != null && ranking.importance < NotificationManager.IMPORTANCE_DEFAULT
        if (low && sbn.packageName !in _lowImportance.value) return null
        val shown = lockScreenVersion(notification, ranking) ?: return null
        val extras = shown.extras
        val title =
            (
                extras.getCharSequence(Notification.EXTRA_CONVERSATION_TITLE)
                    ?: extras.getCharSequence(Notification.EXTRA_TITLE)
            )?.toString().orEmpty()
        val text =
            messages(shown).ifEmpty {
                (extras.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: extras.getCharSequence(Notification.EXTRA_TEXT))
                    ?.toString()
                    .orEmpty()
            }
        if (title.isBlank() && text.isBlank()) return null
        val app = appInfo(notification)
        return PhoneNotification(
            id = sbn.key,
            app =
                (app?.let { context.packageManager.getApplicationLabel(it).toString() } ?: sbn.packageName)
                    .ifBlank { sbn.packageName }
                    .truncateUtf8(NotificationLimits.APP),
            title = title.truncateUtf8(NotificationLimits.TITLE),
            text = text.truncateUtf8(NotificationLimits.TEXT),
            icon = icon(sbn.packageName, app),
            actions = buttons(notification),
            alertOnce = alertOnce || notification.flags and Notification.FLAG_ONLY_ALERT_ONCE != 0,
        )
    }

    /**
     * The version a locked phone may show, by the app's visibility or the user's override for its channel: none for a
     * secret one, the public version of a private one when it has one.
     */
    private fun lockScreenVersion(
        notification: Notification,
        ranking: NotificationListenerService.Ranking?,
    ): Notification? {
        if (fullWhileLocked || !keyguard.isKeyguardLocked) return notification
        val override = ranking?.lockscreenVisibilityOverride ?: NotificationListenerService.Ranking.VISIBILITY_NO_OVERRIDE
        val visibility = if (override == NotificationListenerService.Ranking.VISIBILITY_NO_OVERRIDE) notification.visibility else override
        if (visibility == Notification.VISIBILITY_SECRET) return null
        return notification.publicVersion.takeIf { visibility == Notification.VISIBILITY_PRIVATE } ?: notification
    }

    /** A conversation's latest messages, one `Sender: text` line each, as the shade shows them. */
    @Suppress("DEPRECATION")
    private fun messages(notification: Notification): String {
        val bundles = notification.extras.getParcelableArray(Notification.EXTRA_MESSAGES) ?: return ""
        return Notification.MessagingStyle.Message
            .getMessagesFromBundleArray(bundles)
            .takeLast(MESSAGES_SHOWN)
            .joinToString("\n") { message ->
                val sender = message.senderPerson?.name ?: context.getString(R.string.mirror_you)
                "$sender: ${message.text}"
            }
    }

    /** Each action keeps its index as its id, so a command finds it again in the live notification. */
    private fun buttons(notification: Notification): List<NotificationButton> =
        notification.actions
            .orEmpty()
            .withIndex()
            .filter { (_, action) -> !action.isContextual && !action.title.isNullOrBlank() }
            .take(NotificationLimits.ACTIONS)
            .map { (index, action) ->
                NotificationButton(
                    id = index.toString(),
                    label = action.title.toString().truncateUtf8(NotificationLimits.ACTION),
                    reply = action.remoteInputs.orEmpty().any { it.allowFreeFormInput },
                )
            }

    private fun run(command: NotificationCommand) {
        val service = listener ?: return
        val sbn =
            runCatching { service.getActiveNotifications(arrayOf(command.id)) }.getOrNull()?.firstOrNull()
                ?: return run { Log.i(TAG, "a desktop asked for ${command.id}, which is gone") }
        when (command) {
            is NotificationCommand.Dismiss -> service.cancelNotification(sbn.key)
            is NotificationCommand.Action -> {
                val action = command.action.toIntOrNull()?.let { sbn.notification.actions?.getOrNull(it) } ?: return
                runCatching { fire(action, command.replyText) }.onFailure { log("running an action", it) }
            }
        }
    }

    private fun fire(
        action: Notification.Action,
        replyText: String?,
    ) {
        val fillIn = Intent()
        if (replyText != null) {
            val inputs =
                action.remoteInputs
                    .orEmpty()
                    .filter { it.allowFreeFormInput }
                    .toTypedArray()
            if (inputs.isEmpty()) return
            val results = Bundle().apply { inputs.forEach { putCharSequence(it.resultKey, replyText) } }
            RemoteInput.addResultsToIntent(inputs, fillIn, results)
            RemoteInput.setResultsSource(fillIn, RemoteInput.SOURCE_FREE_FORM_INPUT)
        }
        action.actionIntent.send(context, 0, fillIn, null, null, null, backgroundStart())
    }

    /** An action may open an activity; since Android 14 the sender must allow that from the background. */
    private fun backgroundStart(): Bundle? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            ActivityOptions
                .makeBasic()
                .setPendingIntentBackgroundActivityStartMode(ActivityOptions.MODE_BACKGROUND_ACTIVITY_START_ALLOWED)
                .toBundle()
        } else {
            null
        }

    @Suppress("DEPRECATION")
    private fun appInfo(notification: Notification): ApplicationInfo? = notification.extras.getParcelable(EXTRA_APP_INFO)

    /** The app icon at 64 px as PNG, dropped if it exceeds the protocol's limit. */
    private fun icon(
        packageName: String,
        app: ApplicationInfo?,
    ): ByteArray? =
        icons.getOrPut(packageName) {
            runCatching {
                val drawable: Drawable =
                    app?.let { context.packageManager.getApplicationIcon(it) }
                        ?: context.packageManager.getApplicationIcon(packageName)
                val bitmap = Bitmap.createBitmap(ICON_PX, ICON_PX, Bitmap.Config.ARGB_8888)
                drawable.setBounds(0, 0, ICON_PX, ICON_PX)
                drawable.draw(Canvas(bitmap))
                val png = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
                png.takeIf { it.size <= NotificationLimits.ICON }
            }.getOrNull()
        }

    private fun log(
        what: String,
        error: Throwable,
    ) = Log.w(TAG, "$what: ${error.message}")

    private companion object {
        const val TAG = "NotificationMirror"
        const val PREFS = "mirror"
        const val KEY_EXCLUDED = "excluded"
        const val KEY_LOW_IMPORTANCE = "low_importance"
        const val KEY_FULL_WHILE_LOCKED = "full_while_locked"
        const val ICON_PX = 64
        const val MESSAGES_SHOWN = 5

        /** `Notification.EXTRA_BUILDER_APPLICATION_INFO`, hidden in the SDK: the posting app's info, which needs no package visibility. */
        const val EXTRA_APP_INFO = "android.appInfo"
    }
}

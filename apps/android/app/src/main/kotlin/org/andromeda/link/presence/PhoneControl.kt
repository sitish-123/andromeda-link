package org.andromeda.link.presence

import android.app.KeyguardManager
import android.app.NotificationManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.database.ContentObserver
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.IncomingSystemCommand
import org.andromeda.link.core.domain.PhoneControls

/**
 * The phone's media volume, ringer, Do Not Disturb, and lock, as desktops see them, and the desktop commands that set
 * them. Volume changes arrive through the system settings observer, the rest as broadcasts, so nothing polls.
 */
class PhoneControl(
    private val context: Context,
    private val repository: LinkRepository,
    private val scope: CoroutineScope,
) {
    private val audio = context.getSystemService(AudioManager::class.java)
    private val notifications = context.getSystemService(NotificationManager::class.java)
    private val keyguard = context.getSystemService(KeyguardManager::class.java)
    private val _controls = MutableStateFlow(read())
    val controls: StateFlow<PhoneControls> = _controls.asStateFlow()

    fun start() {
        val refresh = { _controls.value = read() }
        context.contentResolver.registerContentObserver(
            Settings.System.CONTENT_URI,
            true,
            object : ContentObserver(Handler(Looper.getMainLooper())) {
                override fun onChange(selfChange: Boolean) = refresh()
            },
        )
        val filter =
            IntentFilter().apply {
                addAction(AudioManager.RINGER_MODE_CHANGED_ACTION)
                addAction(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED)
                addAction(Intent.ACTION_SCREEN_OFF)
                addAction(Intent.ACTION_SCREEN_ON)
                addAction(Intent.ACTION_USER_PRESENT)
            }
        context.registerReceiver(
            object : BroadcastReceiver() {
                override fun onReceive(
                    context: Context,
                    intent: Intent,
                ) = refresh()
            },
            filter,
        )
        scope.launch {
            repository.systemCommands.collect { command ->
                val error = runCatching { run(command) }.exceptionOrNull()?.let { it.message ?: "It failed on the phone" }
                _controls.value = read()
                repository.systemResult(command.desktopId, command.id, error)
            }
        }
    }

    private fun read(): PhoneControls {
        val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(1)
        val ringer =
            when (audio.ringerMode) {
                AudioManager.RINGER_MODE_SILENT -> "silent"
                AudioManager.RINGER_MODE_VIBRATE -> "vibrate"
                else -> "normal"
            }
        return PhoneControls(
            mediaVolume = audio.getStreamVolume(AudioManager.STREAM_MUSIC) * 100 / max,
            ringer = ringer,
            dnd = notifications.currentInterruptionFilter > NotificationManager.INTERRUPTION_FILTER_ALL,
            locked = keyguard.isDeviceLocked,
        )
    }

    /** Throws with a message for the desktop's user when the command cannot be done. */
    private fun run(command: IncomingSystemCommand) {
        val value = command.value ?: error("${command.command} needs a value")
        when (command.command) {
            "set-media-volume" -> {
                val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
                audio.setStreamVolume(AudioManager.STREAM_MUSIC, (value.coerceIn(0, 100) * max + 50) / 100, 0)
            }
            "set-ringer" -> {
                // Silent and leaving it change Do Not Disturb, which needs its access.
                needPolicyAccess()
                audio.ringerMode =
                    when (value) {
                        0 -> AudioManager.RINGER_MODE_SILENT
                        1 -> AudioManager.RINGER_MODE_VIBRATE
                        else -> AudioManager.RINGER_MODE_NORMAL
                    }
            }
            "set-dnd" -> {
                needPolicyAccess()
                notifications.setInterruptionFilter(
                    if (value == 1) NotificationManager.INTERRUPTION_FILTER_PRIORITY else NotificationManager.INTERRUPTION_FILTER_ALL,
                )
            }
            else -> error("${command.command} is not a phone command")
        }
    }

    private fun needPolicyAccess() {
        check(notifications.isNotificationPolicyAccessGranted) { "Allow Do Not Disturb access for Link on the phone" }
    }
}

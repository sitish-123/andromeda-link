package org.andromeda.link

import android.app.Application
import android.content.Intent
import android.os.Build
import android.provider.Settings
import android.util.Log
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.ProcessLifecycleOwner
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import org.andromeda.link.calls.Calls
import org.andromeda.link.clipboard.ClipHistory
import org.andromeda.link.clipboard.ClipboardSync
import org.andromeda.link.clipboard.ClipboardWatcher
import org.andromeda.link.clipboard.LogAccess
import org.andromeda.link.clipboard.ScreenshotWatcher
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.data.MediaLibrary
import org.andromeda.link.core.data.Messaging
import org.andromeda.link.core.data.PhoneContacts
import org.andromeda.link.core.data.PhoneTexts
import org.andromeda.link.files.MediaAccess
import org.andromeda.link.files.PhoneFiles
import org.andromeda.link.media.DesktopMedia
import org.andromeda.link.media.PhoneMedia
import org.andromeda.link.notifications.Channels
import org.andromeda.link.notifications.NotificationMirror
import org.andromeda.link.notifications.ShareNotifier
import org.andromeda.link.notifications.TransferNotifier
import org.andromeda.link.presence.BluetoothBonding
import org.andromeda.link.presence.PhoneControl
import org.andromeda.link.presence.Presence
import org.andromeda.link.presence.StatusReporter
import org.andromeda.link.ring.Ringer
import org.andromeda.link.screen.ScreenMirror
import org.andromeda.link.transfer.TransferService
import org.andromeda.link.widget.DesktopWidget
import java.io.File

class LinkApplication : Application() {
    lateinit var container: AppContainer
        private set

    override fun onCreate() {
        super.onCreate()
        container = AppContainer(this)
        container.start()
    }
}

/** Every long-lived object, built once; screens receive what they need through their ViewModel's constructor. */
class AppContainer(
    val application: Application,
) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val notifier = ShareNotifier(application)
    private val transferNotifier = TransferNotifier(application)
    val repository = LinkRepository(application, deviceName(application))
    val presence = Presence(application, repository, scope)
    val clipHistory = ClipHistory(File(application.filesDir, "clips.json"))
    val clipboard = ClipboardSync(application, repository, clipHistory, scope)
    val clipboardWatcher = ClipboardWatcher(application, clipboard, scope)
    private val screenshots = ScreenshotWatcher(application, clipboard, scope)
    val mirror = NotificationMirror(application, repository, scope)
    private val phoneControl = PhoneControl(application, repository, scope)
    private val status = StatusReporter(application, repository, scope, mirror.accessOff, phoneControl.controls)
    val media = PhoneMedia(application, repository, scope)
    private val desktopMedia = DesktopMedia(application, repository, scope)
    private val widget = DesktopWidget(application, repository, scope)
    val ringer = Ringer(application, repository, scope)
    val screen = ScreenMirror(application, repository, scope)
    val calls = Calls(application, repository, scope)
    private val bonding = BluetoothBonding(application, repository, scope)
    val files = PhoneFiles(application)
    private val library = MediaLibrary(application)
    private val contacts = PhoneContacts(application) { scope.launch { repository.contactsChanged() } }
    private val texts = PhoneTexts(application) { scope.launch { repository.textsChanged() } }

    fun start() {
        Channels.create(application)
        LogAccess.load(application)
        presence.start()
        clipboard.start()
        screenshots.start()
        phoneControl.start()
        status.start()
        mirror.start()
        media.start()
        desktopMedia.start()
        widget.start()
        scope.launch { repository.setMessages(Messaging(contacts, texts)) }
        ringer.start()
        screen.start()
        calls.start()
        bonding.start()
        // All files access is granted in Settings, so the roots are rechecked whenever the app comes back.
        ProcessLifecycleOwner.get().lifecycle.addObserver(
            LifecycleEventObserver { _, event ->
                if (event == Lifecycle.Event.ON_START) {
                    refreshBrowseRoots()
                    contacts.watch()
                    texts.watch()
                }
            },
        )
        scope.launch { repository.incoming.collect(notifier::post) }
        scope.launch { repository.pings.collect(notifier::post) }
        scope.launch { repository.transfers.collect(transferNotifier::post) }
        scope.launch {
            // Transfers start from the share target or a notification action, both of which may start the service.
            repository.activeTransfers.map { it.isNotEmpty() }.distinctUntilChanged().collect { active ->
                val intent = Intent(application, TransferService::class.java)
                if (!active) {
                    application.stopService(intent)
                } else if (runCatching { ContextCompat.startForegroundService(application, intent) }.isFailure) {
                    Log.w("link", "a transfer started while in the background runs without its service")
                }
            }
        }
    }

    /** `Storage` needs All files access; `Photos`, the media store by album, needs only the media permissions. */
    private fun refreshBrowseRoots() {
        val roots = if (files.allFiles()) mapOf("Storage" to files.root.absolutePath) else emptyMap()
        val library = library.takeIf { files.mediaAccess() != MediaAccess.None }
        scope.launch { repository.setBrowseRoots(roots, library) }
    }
}

/** The name desktops show; the protocol allows at most 64 characters. */
private fun deviceName(application: Application): String {
    val name = Settings.Global.getString(application.contentResolver, Settings.Global.DEVICE_NAME)
    return (name?.takeIf { it.isNotBlank() } ?: Build.MODEL).take(64)
}

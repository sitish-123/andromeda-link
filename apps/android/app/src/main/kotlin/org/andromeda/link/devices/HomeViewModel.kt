package org.andromeda.link.devices

import android.content.ClipData
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.andromeda.link.clipboard.ClipboardSync
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.Desktop
import org.andromeda.link.core.domain.DesktopPlayer
import org.andromeda.link.core.domain.DesktopStatus
import org.andromeda.link.core.domain.DesktopVolume
import org.andromeda.link.core.domain.LinkFailure
import org.andromeda.link.core.domain.MediaCommandKind
import org.andromeda.link.core.domain.PlaybackState
import org.andromeda.link.core.domain.RecentClip
import org.andromeda.link.core.domain.TransferRecord
import org.andromeda.link.core.domain.linkFailure
import org.andromeda.link.notifications.NotificationMirror
import org.andromeda.link.presence.Presence

data class HomeState(
    val desktops: List<Desktop> = emptyList(),
    val stayConnected: Boolean = false,
    val ringing: Set<String> = emptySet(),
    val mirrorGranted: Boolean = false,
    val refreshing: Boolean = false,
    val selectedDesktopId: String? = null,
    val activeTransfers: Int = 0,
    val desk: DeskState = DeskState(),
) {
    /** The desktop the hero shows: a connected one first. */
    val primary: Desktop? get() =
        desktops.firstOrNull { it.id == selectedDesktopId }
            ?: desktops.firstOrNull { it.connected } ?: desktops.firstOrNull()
}

/** What every desktop reports, by desktop id, and the recent transfers and clips. */
data class DeskState(
    val statuses: Map<String, DesktopStatus> = emptyMap(),
    val volumes: Map<String, DesktopVolume> = emptyMap(),
    val capabilities: Map<String, Set<String>> = emptyMap(),
    val players: List<DesktopPlayer> = emptyList(),
    val transfers: List<TransferRecord> = emptyList(),
    val clips: List<RecentClip> = emptyList(),
) {
    /** A playing player first. */
    fun player(desktopId: String): DesktopPlayer? =
        players.filter { it.desktopId == desktopId }.let { all ->
            all.firstOrNull { it.player.state == PlaybackState.Playing }
                ?: all.firstOrNull()
        }

    /** Null when the desktop does not report what its session can do. */
    fun can(desktopId: String): Set<String>? = capabilities[desktopId]
}

data class NotificationPrefs(
    val excluded: Int = 0,
    val lowImportance: Int = 0,
    val fullWhileLocked: Boolean = true,
)

sealed interface HomeMessage {
    data class Paired(
        val name: String,
    ) : HomeMessage

    data class Sent(
        val name: String,
    ) : HomeMessage

    data class FilesSending(
        val name: String,
    ) : HomeMessage

    data object ClipboardEmpty : HomeMessage

    data class ClipboardSent(
        val name: String,
    ) : HomeMessage

    data class ClipboardQueued(
        val name: String,
    ) : HomeMessage

    data class Failed(
        val failure: LinkFailure,
    ) : HomeMessage
}

class HomeViewModel(
    private val repository: LinkRepository,
    private val presence: Presence,
    private val mirror: NotificationMirror,
    private val clipboard: ClipboardSync,
) : ViewModel() {
    private val refreshing = MutableStateFlow(false)
    private val selectedDesktopId = MutableStateFlow<String?>(null)
    private val _message = MutableStateFlow<HomeMessage?>(null)
    private val _clearsSynced = MutableStateFlow(clipboard.clearsSynced)
    private val _sendsScreenshots = MutableStateFlow(clipboard.sendsScreenshots)
    private val desk =
        combine(
            repository.desktopStatuses,
            repository.desktopVolumes,
            repository.capabilities,
            repository.desktopPlayers,
            repository.transferActivity,
        ) { statuses, volumes, can, players, transfers -> DeskState(statuses, volumes, can, players, transfers) }
            .combine(repository.recentClips) { desk, clips -> desk.copy(clips = clips) }

    val state: StateFlow<HomeState> =
        combine(
            repository.desktops,
            presence.stayConnected,
            repository.ringingDesktops,
            mirror.granted,
            refreshing,
        ) { desktops, stay, ringing, granted, busy -> HomeState(desktops, stay, ringing, granted, busy) }
            .combine(selectedDesktopId) { state, id -> state.copy(selectedDesktopId = id) }
            .combine(repository.activeTransfers) { state, transfers -> state.copy(activeTransfers = transfers.size) }
            .combine(desk) { state, desk -> state.copy(desk = desk) }
            .stateIn(viewModelScope, SharingStarted.WhileSubscribed(STOP_TIMEOUT_MS), HomeState())

    /** Settings' notification rows: apps left out, apps whose quiet notifications go too, and the locked switch. */
    val notificationPrefs: StateFlow<NotificationPrefs> =
        combine(mirror.excluded, mirror.lowImportance, mirror.fullContentWhileLocked) { excluded, low, full ->
            NotificationPrefs(excluded.size, low.size, full)
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(STOP_TIMEOUT_MS), NotificationPrefs())

    fun setFullContentWhileLocked(on: Boolean) = mirror.setFullContentWhileLocked(on)

    /** "Clear synced clips after 2 minutes"; off by default. */
    val clearsSynced: StateFlow<Boolean> = _clearsSynced.asStateFlow()

    /** "Send screenshots" to the desktop clipboard; on by default. */
    val sendsScreenshots: StateFlow<Boolean> = _sendsScreenshots.asStateFlow()

    /** The one-line message the screen shows, until [messageShown]. */
    val message: StateFlow<HomeMessage?> = _message.asStateFlow()

    init {
        viewModelScope.launch { repository.refresh() }
    }

    /** Pull to refresh: rereads the desktops and dials the ones not connected now, instead of at the next redial. */
    fun refresh() {
        viewModelScope.launch {
            refreshing.value = true
            repository.refresh()
            state.value.desktops
                .filterNot { it.connected }
                .forEach { repository.connect(it.id) }
            refreshing.value = false
        }
    }

    fun setStayConnected(stay: Boolean) = presence.setStayConnected(stay)

    fun setClearsSynced(on: Boolean) {
        clipboard.clearsSynced = on
        _clearsSynced.value = on
    }

    fun setSendsScreenshots(on: Boolean) {
        clipboard.sendsScreenshots = on
        _sendsScreenshots.value = on
    }

    fun selectDesktop(desktop: Desktop) {
        selectedDesktopId.value = desktop.id
    }

    fun connect(desktop: Desktop) {
        if (refreshing.value) return
        refreshing.value = true
        viewModelScope.launch {
            try {
                repository.connect(desktop.id).exceptionOrNull()?.let {
                    _message.value = HomeMessage.Failed(it.linkFailure())
                }
            } finally {
                refreshing.value = false
            }
        }
    }

    fun notificationsAllowed() = presence.notificationsAllowed()

    fun setVolume(
        desktop: Desktop,
        percent: Int,
    ) = act { repository.setDesktopVolume(desktop.id, percent) }

    fun setBrightness(
        desktop: Desktop,
        percent: Int,
    ) = act { repository.setDesktopBrightness(desktop.id, percent) }

    fun setDnd(
        desktop: Desktop,
        on: Boolean,
    ) = act { repository.setDesktopDnd(desktop.id, on) }

    fun lock(desktop: Desktop) = act { repository.lockDesktop(desktop.id) }

    fun sleep(desktop: Desktop) = act { repository.suspendDesktop(desktop.id) }

    fun screenOff(desktop: Desktop) = act { repository.turnDesktopScreenOff(desktop.id) }

    fun media(
        shown: DesktopPlayer,
        command: MediaCommandKind,
        value: Long? = null,
    ) = act { repository.mediaCommand(shown.desktopId, shown.player.player, command, value) }

    fun ring(
        desktop: Desktop,
        on: Boolean,
    ) = act { repository.ringDesktop(desktop.id, on) }

    /** Offers the current clipboard content; the app may read it only while in front. */
    fun sendClipboard(
        desktop: Desktop,
        clip: ClipData?,
    ) {
        if (clip == null ||
            clip.itemCount == 0 ||
            (clip.getItemAt(0).uri == null && clip.getItemAt(0).text.isNullOrBlank())
        ) {
            _message.value = HomeMessage.ClipboardEmpty
            return
        }
        // A clipboard offer, not a share: the desktop puts it on its own clipboard instead of in a notification.
        if (ClipboardSync.isFromDesktop(clip)) {
            _message.value = HomeMessage.ClipboardSent(desktop.name)
            return
        }
        viewModelScope.launch {
            _message.value =
                clipboard.offer(clip).fold(
                    { if (it > 0) HomeMessage.ClipboardSent(desktop.name) else HomeMessage.ClipboardQueued(desktop.name) },
                    { HomeMessage.Failed(it.linkFailure()) },
                )
        }
    }

    fun sendFiles(
        desktop: Desktop,
        uris: List<String>,
    ) {
        if (uris.isNotEmpty()) act(HomeMessage.FilesSending(desktop.name)) { repository.sendFiles(desktop.id, uris) }
    }

    fun announce(desktop: Desktop) {
        _message.value = HomeMessage.Paired(desktop.name)
    }

    fun messageShown() = _message.update { null }

    private fun act(
        onSuccess: HomeMessage? = null,
        block: suspend () -> Result<*>,
    ) {
        viewModelScope.launch {
            val failure = block().exceptionOrNull()
            _message.value = failure?.let { HomeMessage.Failed(it.linkFailure()) } ?: onSuccess ?: _message.value
        }
    }

    private companion object {
        const val STOP_TIMEOUT_MS = 5_000L
    }
}

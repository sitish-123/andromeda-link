package org.andromeda.link.media

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.DesktopVolume
import org.andromeda.link.core.domain.LinkFailure
import org.andromeda.link.core.domain.MediaCommandKind
import org.andromeda.link.core.domain.MediaPlayer
import org.andromeda.link.core.domain.linkFailure

/** A desktop player, with its desktop's output volume when that desktop reports one. */
data class ShownPlayer(
    val desktopId: String,
    val desktopName: String,
    val player: MediaPlayer,
    val desktopVolume: DesktopVolume? = null,
)

/** The players of every connected desktop, and commands to them. */
class MediaViewModel(
    private val repository: LinkRepository,
) : ViewModel() {
    private val failures = Channel<LinkFailure>(Channel.BUFFERED)

    val players: StateFlow<List<ShownPlayer>> =
        combine(
            repository.desktopPlayers,
            repository.desktops,
            repository.desktopVolumes,
        ) { players, desktops, volumes ->
            players.map { shown ->
                val name = desktops.firstOrNull { it.id == shown.desktopId }?.name.orEmpty()
                ShownPlayer(shown.desktopId, name, shown.player, volumes[shown.desktopId])
            }
        }.stateIn(viewModelScope, SharingStarted.WhileSubscribed(STOP_TIMEOUT_MS), emptyList())

    val failed: Flow<LinkFailure> = failures.receiveAsFlow()

    fun command(
        shown: ShownPlayer,
        command: MediaCommandKind,
        value: Long? = null,
    ) {
        viewModelScope.launch {
            repository
                .mediaCommand(shown.desktopId, shown.player.player, command, value)
                .onFailure { failures.send(it.linkFailure()) }
        }
    }

    fun setDesktopVolume(
        shown: ShownPlayer,
        percent: Int,
    ) {
        viewModelScope.launch {
            repository.setDesktopVolume(shown.desktopId, percent).onFailure { failures.send(it.linkFailure()) }
        }
    }

    fun setDesktopMuted(
        shown: ShownPlayer,
        muted: Boolean,
    ) {
        viewModelScope.launch {
            repository.setDesktopMuted(shown.desktopId, muted).onFailure { failures.send(it.linkFailure()) }
        }
    }

    private companion object {
        const val STOP_TIMEOUT_MS = 5_000L
    }
}

package org.andromeda.link.control

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.LinkFailure
import org.andromeda.link.core.domain.RemoteEvent
import org.andromeda.link.core.domain.RemoteSource
import org.andromeda.link.core.domain.linkFailure

data class ViewerState(
    val sources: List<RemoteSource> = emptyList(),
    val current: Int = 0,
    val stream: RemoteEvent.Stream? = null,
    /** Why the desktop ended or refused it; the viewer then only offers to go back. */
    val stopped: String? = null,
    val failure: LinkFailure? = null,
    val touchpad: Boolean = false,
    val muted: Boolean = false,
) {
    /** A window's place on the desktop is unknown, so only the touchpad moves the pointer there. */
    val direct get() = !touchpad && sources.getOrNull(current)?.window != true
}

/**
 * One desktop's screen on this phone: [open] asks for it, [select] switches sources, [leave] ends it. The view model
 * outlives the screen, which survives rotation this way.
 */
class ViewerViewModel(
    private val repository: LinkRepository,
    private val desktopId: String,
) : ViewModel() {
    private val _state = MutableStateFlow(ViewerState())
    val state: StateFlow<ViewerState> = _state
    private var open = false

    init {
        viewModelScope.launch {
            repository.remote.filter { it.desktopId == desktopId }.collect { event ->
                when (event) {
                    is RemoteEvent.Started -> _state.update { it.copy(sources = event.sources, current = event.current) }
                    is RemoteEvent.Stream -> {
                        _state.value.stream
                            ?.video
                            ?.stop()
                        _state.update { it.copy(stream = event, stopped = null) }
                    }
                    is RemoteEvent.Stopped -> _state.update { it.copy(stream = null, stopped = event.reason.orEmpty()) }
                }
            }
        }
    }

    fun open() {
        if (open) return
        open = true
        _state.value = ViewerState()
        viewModelScope.launch {
            repository.startRemote(desktopId, null).exceptionOrNull()?.let { error ->
                _state.update { it.copy(failure = error.linkFailure(), stopped = "") }
            }
        }
    }

    fun leave() {
        if (!open) return
        open = false
        val stream = _state.value.stream
        _state.value = ViewerState()
        // Not the view model's scope: the stop must reach the desktop after the screen is gone.
        CoroutineScope(SupervisorJob() + Dispatchers.IO).launch {
            stream?.video?.stop()
            repository.stopRemote(desktopId)
        }
    }

    fun select(source: Int) {
        if (source == _state.value.current) return
        viewModelScope.launch { repository.startRemote(desktopId, source) }
    }

    fun setTouchpad(on: Boolean) = _state.update { it.copy(touchpad = on) }

    fun toggleMute() = _state.update { it.copy(muted = !it.muted) }

    fun failureShown() = _state.update { it.copy(failure = null) }

    override fun onCleared() = leave()
}

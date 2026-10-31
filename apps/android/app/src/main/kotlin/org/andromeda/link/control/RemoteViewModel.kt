package org.andromeda.link.control

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import org.andromeda.link.core.data.LinkRepository
import org.andromeda.link.core.domain.LinkFailure
import org.andromeda.link.core.domain.RemoteInput
import org.andromeda.link.core.domain.RemoteKey
import org.andromeda.link.core.domain.linkFailure
import kotlin.math.roundToInt

/**
 * The phone as one desktop's touchpad and keyboard. Moves and scrolls add up and go once a frame, so a fast finger
 * stays under the desktop's input rate; everything goes out in the order it happened.
 */
class RemoteViewModel(
    private val repository: LinkRepository,
    private val desktopId: String,
) : ViewModel() {
    private val outbox = Channel<RemoteInput>(Channel.UNLIMITED)
    private val wake = Channel<Unit>(Channel.CONFLATED)
    private var moveX = 0f
    private var moveY = 0f
    private var scrollX = 0f
    private var scrollY = 0f
    private var point: RemoteInput.Point? = null
    private val _failure = MutableStateFlow<LinkFailure?>(null)
    val failure: StateFlow<LinkFailure?> = _failure
    private val _held = MutableStateFlow<Set<String>>(emptySet())

    /** The modifiers held for the next key, as `super` or `ctrl`. */
    val held: StateFlow<Set<String>> = _held

    /** The desktop takes shortcuts: its input can hold modifiers. */
    val shortcuts: StateFlow<Boolean> =
        repository.capabilities
            .map { SHORTCUTS in it[desktopId].orEmpty() }
            .stateIn(viewModelScope, SharingStarted.Eagerly, false)
    /** The desktop makes a real touchpad of raw finger contacts, so its own touchpad settings and gestures apply. */
    val touchpad: StateFlow<Boolean> =
        repository.capabilities
            .map { TOUCHPAD in it[desktopId].orEmpty() }
            .stateIn(viewModelScope, SharingStarted.Eagerly, false)


    init {
        viewModelScope.launch {
            for (input in outbox) {
                repository.sendInput(desktopId, input).exceptionOrNull()?.let { _failure.value = it.linkFailure() }
            }
        }
        viewModelScope.launch {
            for (ignored in wake) {
                delay(FRAME_MS)
                flush()
            }
        }
    }

    /** By (dx, dy) of the phone's pixels. */
    fun move(
        dx: Float,
        dy: Float,
    ) {
        moveX += dx * POINTER_SPEED
        moveY += dy * POINTER_SPEED
        wake.trySend(Unit)
    }

    /** The pointer to a place on remote control's picture; only the latest of a frame goes. */
    fun point(at: RemoteInput.Point) {
        point = at
        wake.trySend(Unit)
    }

    /** Fingers moved by (dx, dy) of the phone's pixels; the content follows them. */
    fun scroll(
        dx: Float,
        dy: Float,
    ) {
        scrollX -= dx
        scrollY -= dy
        wake.trySend(Unit)
    }

    fun send(input: RemoteInput) {
        flush()
        outbox.trySend(input)
    }

    fun key(key: RemoteKey) = send(RemoteInput.Key(key))

    /** What changed in the typing field: removed characters as Backspace, then the added text. */
    fun typed(
        before: String,
        after: String,
    ) {
        val kept = before.commonPrefixWith(after).length
        repeat(before.length - kept) { key(RemoteKey.Backspace) }
        val added = after.substring(kept)
        when {
            added.isEmpty() -> Unit
            _held.value.isNotEmpty() && added.length == 1 -> shortcut(added.lowercase())
            else -> send(RemoteInput.Text(added))
        }
    }

    fun hold(modifier: String) = _held.update { if (modifier in it) it - modifier else it + modifier }

    /** [key] with the held modifiers, which let go after it; [key] may carry its own, as `alt+tab`. */
    fun shortcut(key: String) {
        val held = MODIFIERS.filter { it in _held.value }
        send(RemoteInput.Shortcut((held + key).joinToString("+")))
        _held.value = emptySet()
    }

    /** The presenter opened or closed: the desktop stays awake meanwhile. Runs even as the view model clears. */
    fun presenting(on: Boolean) {
        viewModelScope.launch(NonCancellable) { repository.setPresenting(desktopId, on) }
    }

    fun failureShown() {
        _failure.value = null
    }

    private fun flush() {
        point?.let { outbox.trySend(it) }
        point = null
        val (mx, my) = moveX.roundToInt() to moveY.roundToInt()
        if (mx != 0 || my != 0) {
            outbox.trySend(RemoteInput.Move(mx, my))
            moveX -= mx
            moveY -= my
        }
        val (sx, sy) = scrollX.roundToInt() to scrollY.roundToInt()
        if (sx != 0 || sy != 0) {
            outbox.trySend(RemoteInput.Scroll(sx, sy))
            scrollX -= sx
            scrollY -= sy
        }
    }
        const val TOUCHPAD = "touchpad"

    private companion object {
        const val SHORTCUTS = "shortcuts"

        /** In the order a shortcut names them. */
        val MODIFIERS = listOf("super", "ctrl", "alt", "shift")
        const val FRAME_MS = 16L
        const val POINTER_SPEED = 1.5f
    }
}

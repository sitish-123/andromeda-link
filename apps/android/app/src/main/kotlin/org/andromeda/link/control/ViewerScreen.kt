package org.andromeda.link.control

import android.app.Activity
import android.graphics.SurfaceTexture
import android.view.Surface
import android.view.TextureView
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Menu
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.andromeda.link.R
import org.andromeda.link.core.domain.MIRROR_UNIT
import org.andromeda.link.core.domain.PointerButton
import org.andromeda.link.core.domain.RemoteInput
import org.andromeda.link.ui.components.Glyph
import org.andromeda.link.ui.components.Label
import org.andromeda.link.ui.components.PillButton
import org.andromeda.link.ui.components.PillKind
import org.andromeda.link.ui.components.SlidingPillControl
import org.andromeda.link.ui.text
import org.andromeda.link.ui.theme.LinkTheme
import org.andromeda.link.ui.theme.Radius
import org.andromeda.link.ui.theme.Space
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * A desktop's screen, full screen: direct touch clicks and drags where the finger is, or the picture works as a
 * touchpad. Two fingers scroll; a pinch zooms. A round button, which can be dragged aside, opens the menu.
 */
@Composable
fun ViewerScreen(
    viewModel: ViewerViewModel,
    input: RemoteViewModel,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val leave = {
        viewModel.leave()
        onBack()
    }
    LaunchedEffect(Unit) { viewModel.open() }
    BackHandler(onBack = leave)
    Immersive()
    var surface by remember { mutableStateOf<Surface?>(null) }
    val muted by rememberUpdatedState(state.muted)
    val stream = state.stream
    // One decoder at a time: a surface takes a new one only after the old one let it go.
    val decoding = remember { Mutex() }
    LaunchedEffect(stream, surface) {
        val target = surface
        if (stream != null && target != null) decoding.withLock { play(stream, target) { muted } }
    }
    var menu by remember { mutableStateOf(false) }
    var typing by remember { mutableStateOf(false) }
    Box(modifier.fillMaxSize().background(Color.Black)) {
        if (stream != null) {
            Picture(stream.width, stream.height, state.direct, input) { surface = it }
        }
        val stopped = state.stopped
        when {
            stopped != null -> Ended(stopped, state.failure?.text(LocalContext.current.resources), leave)
            stream == null -> Centered(stringResource(R.string.viewer_connecting))
        }
        if (typing) {
            val focus = remember { FocusRequester() }
            LaunchedEffect(Unit) { focus.requestFocus() }
            val shortcuts by input.shortcuts.collectAsStateWithLifecycle()
            Column(
                Modifier
                    .align(Alignment.BottomCenter)
                    .imePadding()
                    .padding(Space.s12),
                verticalArrangement = Arrangement.spacedBy(Space.s8),
            ) {
                if (shortcuts) ShortcutBar(input)
                Typing(input, Modifier.focusRequester(focus))
            }
        }
        if (menu) {
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = SCRIM)).clickable { menu = false })
            Menu(
                state,
                modifier = Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(Space.s12),
                onTouchpad = viewModel::setTouchpad,
                onKeyboard = {
                    typing = !typing
                    menu = false
                },
                onSource = {
                    viewModel.select(it)
                    menu = false
                },
                onMute = viewModel::toggleMute,
                onDisconnect = leave,
            )
        }
        MenuButton { menu = !menu }
    }
}

/** The system bars hide while the desktop shows, and come back with a swipe from an edge. */
@Composable
private fun Immersive() {
    val view = LocalView.current
    DisposableEffect(view) {
        val window = (view.context as? Activity)?.window
        val bars = window?.let { WindowCompat.getInsetsController(it, view) }
        bars?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars?.hide(WindowInsetsCompat.Type.systemBars())
        view.keepScreenOn = true
        onDispose {
            bars?.show(WindowInsetsCompat.Type.systemBars())
            view.keepScreenOn = false
        }
    }
}

/** The picture, fitted to the screen, then zoomed and moved by the user's pinch. */
@Composable
private fun Picture(
    width: Int,
    height: Int,
    direct: Boolean,
    input: RemoteViewModel,
    onSurface: (Surface?) -> Unit,
) {
    var scale by remember { mutableFloatStateOf(1f) }
    var pan by remember { mutableStateOf(Offset.Zero) }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val density = LocalDensity.current
        val (boxWidth, boxHeight) = with(density) { maxWidth.toPx() to maxHeight.toPx() }
        val fit = minOf(boxWidth / width, boxHeight / height)
        val (shownWidth, shownHeight) = width * fit to height * fit
        val origin = Offset((boxWidth - shownWidth) / 2, (boxHeight - shownHeight) / 2)
        // Kept across zoom steps: a new one would restart the gesture that zooms.
        val view =
            remember(origin, shownWidth, shownHeight) {
                Viewport(origin, shownWidth, shownHeight, { scale }, { pan }) { newScale, newPan ->
                    scale = newScale
                    pan = newPan
                }
            }
        AndroidView(
            factory = { context ->
                TextureView(context).apply {
                    surfaceTextureListener =
                        object : TextureView.SurfaceTextureListener {
                            override fun onSurfaceTextureAvailable(
                                texture: SurfaceTexture,
                                w: Int,
                                h: Int,
                            ) = onSurface(Surface(texture))

                            override fun onSurfaceTextureSizeChanged(
                                texture: SurfaceTexture,
                                w: Int,
                                h: Int,
                            ) = Unit

                            override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
                                onSurface(null)
                                return true
                            }

                            override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit
                        }
                }
            },
            modifier =
                Modifier
                    .offset { IntOffset(origin.x.roundToInt(), origin.y.roundToInt()) }
                    .size(with(density) { shownWidth.toDp() }, with(density) { shownHeight.toDp() })
                    .graphicsLayer {
                        transformOrigin = TransformOrigin(0f, 0f)
                        scaleX = scale
                        scaleY = scale
                        translationX = pan.x
                        translationY = pan.y
                    },
        )
        val description = stringResource(R.string.viewer_picture)
        // A desktop with a real touchpad gets the raw fingers in touchpad mode; its own settings and gestures apply.
        val raw by input.touchpad.collectAsStateWithLifecycle()
        val area = Modifier.fillMaxSize().semantics { contentDescription = description }
        Box(
            if (raw && !direct) {
                area.rawTouchpad(LocalContext.current.resources.displayMetrics) { input.send(RemoteInput.Touch(it)) }
            } else {
                area.pointerInput(direct, view) { if (direct) directTouch(view, input) else touchpad(view, input) }
            },
        )
    }
}

/** Where the picture is on screen, to turn a finger's place into a fraction of the desktop. */
private class Viewport(
    val origin: Offset,
    val width: Float,
    val height: Float,
    val scale: () -> Float,
    val pan: () -> Offset,
    val zoom: (Float, Offset) -> Unit,
) {
    /** The desktop point under [at], in 1/[MIRROR_UNIT], clamped onto the picture. */
    fun point(at: Offset): RemoteInput.Point {
        val local = (at - origin - pan()) / scale()
        val x = (local.x / width).coerceIn(0f, 1f)
        val y = (local.y / height).coerceIn(0f, 1f)
        return RemoteInput.Point((x * MIRROR_UNIT).roundToInt(), (y * MIRROR_UNIT).roundToInt())
    }

    /** Zooms by [factor] around [centroid] and moves by [moved], keeping the picture on screen. */
    fun pinch(
        centroid: Offset,
        factor: Float,
        moved: Offset,
    ) {
        val old = scale()
        val new = (old * factor).coerceIn(1f, MAX_ZOOM)
        // The point under the fingers stays under them.
        val anchored = centroid - origin - (centroid - origin - pan()) * (new / old) + moved
        val limit = Offset(width * (new - 1), height * (new - 1))
        zoom(new, Offset(anchored.x.coerceIn(-limit.x, 0f), anchored.y.coerceIn(-limit.y, 0f)))
    }
}

/**
 * One finger: a tap clicks where it is, a long press right-clicks, a drag holds the left button. Two fingers: a pinch
 * zooms the picture; when it does not zoom, they scroll the desktop.
 */
private suspend fun PointerInputScope.directTouch(
    view: Viewport,
    input: RemoteViewModel,
) = awaitEachGesture {
    val down = awaitFirstDown()
    val start = view.point(down.position)
    input.point(start)
    var dragging = false
    var fingers = 1
    var travelled = 0f
    val held =
        withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) {
            while (true) {
                val pressed = awaitPointerEvent().changes.filter { it.pressed }
                fingers = maxOf(fingers, pressed.size)
                if (pressed.isEmpty() || fingers > 1) break
                travelled += pressed[0].positionChange().getDistance()
                if (travelled > viewConfiguration.touchSlop) break
            }
        } == null
    if (held) {
        input.send(RemoteInput.Click(PointerButton.Right))
        waitUp()
        return@awaitEachGesture
    }
    if (fingers > 1) {
        twoFingers(view, input)
        return@awaitEachGesture
    }
    val first = currentEvent.changes.firstOrNull()
    if (first == null || !first.pressed) {
        input.send(RemoteInput.Click(PointerButton.Left))
        return@awaitEachGesture
    }
    do {
        val event = awaitPointerEvent()
        val change = event.changes.firstOrNull() ?: break
        if (!dragging) {
            input.send(RemoteInput.Press(PointerButton.Left))
            dragging = true
        }
        input.point(view.point(change.position))
        change.consume()
    } while (change.pressed)
    input.send(RemoteInput.Release(PointerButton.Left))
}

/** The picture as a touchpad: one finger moves the pointer and taps to click, two tap to right-click. */
private suspend fun PointerInputScope.touchpad(
    view: Viewport,
    input: RemoteViewModel,
) = awaitEachGesture {
    val down = awaitFirstDown()
    var fingers = 1
    var travelled = 0f
    do {
        val event = awaitPointerEvent()
        val pressed = event.changes.filter { it.pressed }
        fingers = maxOf(fingers, pressed.size)
        if (pressed.size >= 2) {
            val still = twoFingers(view, input) < viewConfiguration.touchSlop
            if (still && currentEvent.changes.first().uptimeMillis - down.uptimeMillis < TAP_MS) {
                input.send(RemoteInput.Click(PointerButton.Right))
            }
            return@awaitEachGesture
        }
        pressed.firstOrNull()?.let {
            val delta = it.positionChange()
            travelled += delta.getDistance()
            input.move(delta.x, delta.y)
        }
        event.changes.forEach { it.consume() }
    } while (pressed.isNotEmpty())
    val quick = currentEvent.changes.first().uptimeMillis - down.uptimeMillis < TAP_MS
    if (quick && travelled < viewConfiguration.touchSlop) {
        input.send(RemoteInput.Click(if (fingers >= 2) PointerButton.Right else PointerButton.Left))
    }
}

/**
 * A pinch zooms the picture; two fingers that keep their distance scroll the desktop. Returns how far they went, or
 * infinity once they zoomed, so the caller can tell a tap.
 */
private suspend fun androidx.compose.ui.input.pointer.AwaitPointerEventScope.twoFingers(
    view: Viewport,
    input: RemoteViewModel,
): Float {
    var spread: Float? = null
    var zooming = false
    var travelled = 0f
    while (true) {
        val event = awaitPointerEvent()
        val pressed = event.changes.filter { it.pressed }
        if (pressed.size < 2) break
        val centroid = (pressed[0].position + pressed[1].position) / 2f
        val distance = (pressed[0].position - pressed[1].position).getDistance()
        val moved = (pressed[0].positionChange() + pressed[1].positionChange()) / 2f
        travelled += moved.getDistance()
        val previous = spread
        spread = distance
        if (previous != null && previous > 0f) {
            val factor = distance / previous
            zooming = zooming || abs(factor - 1f) > PINCH_START
            if (zooming || view.scale() > 1f) {
                view.pinch(centroid, if (zooming) factor else 1f, moved)
            } else {
                input.scroll(moved.x, moved.y)
            }
        }
        event.changes.forEach { it.consume() }
    }
    waitUp()
    return if (zooming) Float.POSITIVE_INFINITY else travelled
}

private suspend fun androidx.compose.ui.input.pointer.AwaitPointerEventScope.waitUp() {
    while (awaitPointerEvent().changes.any { it.pressed }) Unit
}

/** The round button that opens the menu; it can be dragged anywhere on the screen. */
@Composable
private fun MenuButton(onClick: () -> Unit) {
    var moved by remember { mutableStateOf<Offset?>(null) }
    val description = stringResource(R.string.viewer_menu)
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val density = LocalDensity.current
        val size = with(density) { BUTTON.toPx() }
        val margin = with(density) { Space.s16.toPx() }
        val (right, bottom) = with(density) { maxWidth.toPx() - size - margin to maxHeight.toPx() - size - margin }
        val at = moved ?: Offset(right, bottom * START_HEIGHT)
        Box(
            Modifier
                .offset { IntOffset(at.x.roundToInt(), at.y.roundToInt()) }
                .size(BUTTON)
                .shadow(6.dp, CircleShape)
                .clip(CircleShape)
                .background(LinkTheme.colors.surfacePrimary.copy(alpha = BUTTON_ALPHA))
                .semantics { contentDescription = description }
                .clickable(onClick = onClick)
                .pointerInput(Unit) {
                    detectDragGestures { change, drag ->
                        change.consume()
                        val from = moved ?: Offset(right, bottom * START_HEIGHT)
                        moved = Offset((from.x + drag.x).coerceIn(margin, right), (from.y + drag.y).coerceIn(margin, bottom))
                    }
                },
            contentAlignment = Alignment.Center,
        ) {
            Glyph(Icons.Filled.Menu, LinkTheme.colors.textPrimary)
        }
    }
}

@Composable
private fun Menu(
    state: ViewerState,
    onTouchpad: (Boolean) -> Unit,
    onKeyboard: () -> Unit,
    onSource: (Int) -> Unit,
    onMute: () -> Unit,
    onDisconnect: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val window = state.sources.getOrNull(state.current)?.window == true
    Column(
        modifier
            .widthIn(max = MENU_WIDTH)
            .fillMaxWidth()
            .clip(Radius.hero)
            .background(LinkTheme.colors.surfacePrimary)
            .padding(Space.s16),
        verticalArrangement = Arrangement.spacedBy(Space.s12),
    ) {
        SlidingPillControl(
            listOf(stringResource(R.string.viewer_touchpad), stringResource(R.string.viewer_direct)),
            if (state.direct) 1 else 0,
            { onTouchpad(it == 0) },
        )
        if (window) Label(stringResource(R.string.viewer_window_touchpad), LinkTheme.type.bodyMedium, LinkTheme.colors.textSecondary)
        if (state.sources.size > 1) {
            Column {
                state.sources.forEachIndexed { index, source ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clip(Radius.button)
                            .clickable { onSource(index) }
                            .padding(horizontal = Space.s8, vertical = Space.s12),
                        horizontalArrangement = Arrangement.spacedBy(Space.s12),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Label(source.name, LinkTheme.type.bodyLarge, LinkTheme.colors.textPrimary, Modifier.weight(1f))
                        if (index == state.current) Glyph(Icons.Filled.Check, LinkTheme.colors.accent)
                    }
                }
            }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Space.s8)) {
            PillButton(stringResource(R.string.viewer_keyboard), onKeyboard, Modifier.weight(1f), PillKind.Tonal)
            PillButton(
                stringResource(if (state.muted) R.string.viewer_unmute else R.string.viewer_mute),
                onMute,
                Modifier.weight(1f),
                PillKind.Tonal,
            )
        }
        PillButton(stringResource(R.string.viewer_disconnect), onDisconnect, Modifier.fillMaxWidth(), PillKind.Destructive)
    }
}

@Composable
private fun Ended(
    reason: String,
    failure: String?,
    onBack: () -> Unit,
) {
    Column(
        Modifier.fillMaxSize().padding(Space.page),
        verticalArrangement = Arrangement.spacedBy(Space.s16, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Label(stringResource(R.string.viewer_ended), LinkTheme.type.titleMedium, Color.White)
        val why = failure ?: reason.takeIf { it.isNotEmpty() }
        if (why != null) Label(why, LinkTheme.type.bodyMedium, Color.White.copy(alpha = SECONDARY))
        PillButton(stringResource(R.string.viewer_back), onBack, kind = PillKind.Tonal)
    }
}

@Composable
private fun Centered(text: String) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Label(text, LinkTheme.type.bodyMedium, Color.White.copy(alpha = SECONDARY))
    }
}

private val BUTTON = 52.dp
private val MENU_WIDTH = 420.dp
private const val BUTTON_ALPHA = 0.85f
private const val START_HEIGHT = 0.6f
private const val SCRIM = 0.4f
private const val SECONDARY = 0.7f
private const val MAX_ZOOM = 5f
private const val PINCH_START = 0.08f
private const val TAP_MS = 250L

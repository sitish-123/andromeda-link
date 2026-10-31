package org.andromeda.link.devices

import android.Manifest
import android.content.ClipboardManager
import android.content.pm.PackageManager
import android.graphics.BitmapFactory
import android.os.Build
import android.os.SystemClock
import android.text.format.DateUtils
import android.text.format.Formatter
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import org.andromeda.link.R
import org.andromeda.link.core.domain.Desktop
import org.andromeda.link.core.domain.DesktopPlayer
import org.andromeda.link.core.domain.DesktopStatus
import org.andromeda.link.core.domain.DesktopVolume
import org.andromeda.link.core.domain.MediaCommandKind
import org.andromeda.link.core.domain.PlaybackState
import org.andromeda.link.core.domain.RecentClip
import org.andromeda.link.core.domain.TransferRecord
import org.andromeda.link.files.MediaAccess
import org.andromeda.link.files.MediaTile
import org.andromeda.link.files.PhoneFiles
import org.andromeda.link.files.PickerSheet
import org.andromeda.link.files.PickerTab
import org.andromeda.link.files.PickerViewModel
import org.andromeda.link.files.sendLabel
import org.andromeda.link.onboarding.Grant
import org.andromeda.link.ui.components.ConnectionOrb
import org.andromeda.link.ui.components.DashboardCard
import org.andromeda.link.ui.components.Glyph
import org.andromeda.link.ui.components.Label
import org.andromeda.link.ui.components.LinkIcons
import org.andromeda.link.ui.components.PillButton
import org.andromeda.link.ui.components.PillKind
import org.andromeda.link.ui.components.PullRefresh
import org.andromeda.link.ui.components.Readout
import org.andromeda.link.ui.components.Screen
import org.andromeda.link.ui.components.Section
import org.andromeda.link.ui.text
import org.andromeda.link.ui.theme.LinkTheme
import org.andromeda.link.ui.theme.Radius
import org.andromeda.link.ui.theme.Size
import org.andromeda.link.ui.theme.Space
import kotlin.math.abs
import kotlin.math.roundToInt

/**
 * Home is the link itself: the desktop at the top of a line, and every action a stop on it. The line is the one bold
 * element (.agent/DESIGN.md, A3); recent transfers branch off it.
 */
@Composable
fun HomeScreen(
    viewModel: HomeViewModel,
    picker: PickerViewModel,
    /** Required grants not given yet, in setup order. */
    missing: List<Grant>,
    /** How many grants setup requires, for the card's progress. */
    required: Int,
    /** Both grants automatic clipboard needs are given, so Home stops offering to set it up. */
    autoClipboard: Boolean,
    onPair: () -> Unit,
    onDesktop: (Desktop) -> Unit,
    onMedia: () -> Unit,
    onSetup: () -> Unit,
    onClipboardSetup: () -> Unit,
    onActivity: () -> Unit,
    onRemote: (Desktop) -> Unit,
    onPresenter: (Desktop) -> Unit,
    onViewer: (Desktop) -> Unit,
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val message by viewModel.message.collectAsStateWithLifecycle()
    val pick = picker.state.collectAsStateWithLifecycle()
    // Only open and close recompose Home; the sheet and the tray read the rest themselves.
    val pickOpen by remember { derivedStateOf { pick.value.open } }
    val context = LocalContext.current
    val resources = context.resources
    val notificationPermission =
        rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            if (granted) viewModel.notificationsAllowed()
        }
    val paired = state.desktops.isNotEmpty()
    // Asked once a desktop is paired, the first moment one can send this phone anything to notify about.
    LaunchedEffect(paired) {
        val needed =
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
                ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
                PackageManager.PERMISSION_GRANTED
        if (paired && needed) notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
    }
    LifecycleResumeEffect(Unit) {
        picker.refresh()
        onPauseOrDispose {}
    }
    val toast =
        when (val shown = message) {
            is HomeMessage.Paired -> resources.getString(R.string.paired_with, shown.name)
            is HomeMessage.Sent -> resources.getString(R.string.share_sent, shown.name)
            is HomeMessage.FilesSending -> resources.getString(R.string.share_files_sending, shown.name)
            HomeMessage.ClipboardEmpty -> resources.getString(R.string.clipboard_empty)
            is HomeMessage.ClipboardSent -> resources.getString(R.string.clipboard_sent, shown.name)
            is HomeMessage.ClipboardQueued -> resources.getString(R.string.clipboard_queued, shown.name)
            is HomeMessage.Failed -> shown.failure.text(resources)
            null -> null
        }
    val primary = state.primary
    val send = {
        val selected = pick.value.selected
        primary?.let { viewModel.sendFiles(it, selected.keys.toList()) }
        picker.clear()
    }
    Screen(
        title = null,
        toast = toast,
        onToastShown = viewModel::messageShown,
        overlay = { if (primary != null) PickerSheet(picker, pick.value, primary.name, send) },
    ) {
        // Under the open picker, Home's photos and buttons would duplicate the sheet's for TalkBack.
        val hidden = if (pickOpen) Modifier.clearAndSetSemantics {} else Modifier
        PullRefresh(state.refreshing, viewModel::refresh, Modifier.fillMaxSize().then(hidden)) {
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(top = Space.s16)) {
                if (primary == null) {
                    EmptyLine(onPair)
                    return@Column
                }
                val desk = state.desk
                val player = desk.player(primary.id)?.takeIf { primary.connected }
                val clipboardSetup = primary.sharing.clipboard && !autoClipboard
                Section {
                    HomeHeader(primary, desk.statuses[primary.id], state.refreshing, { viewModel.connect(primary) }) { onDesktop(primary) }
                    if (state.desktops.size > 1) DesktopSwitcher(state.desktops, primary, viewModel::selectDesktop)
                }
                if (missing.isNotEmpty()) {
                    Section { SetupCard(required - missing.size, required, onSetup) }
                }
                if (player != null) {
                    Section {
                        PlayerCard(player, onOpen = onMedia) { command, value -> viewModel.media(player, command, value) }
                    }
                }
                Section {
                    Controls(
                        primary,
                        desk.can(primary.id),
                        desk.statuses[primary.id],
                        desk.volumes[primary.id],
                        primary.id in state.ringing,
                        viewModel,
                        onDesktop,
                        onRemote,
                        onPresenter,
                        onViewer,
                    ) {
                        viewModel.sendClipboard(primary, context.getSystemService(ClipboardManager::class.java).primaryClip)
                    }
                }
                Section {
                    if (primary.sharing.files) {
                        SendTray(primary, picker, send)
                    } else {
                        Label(stringResource(R.string.home_files_disabled), LinkTheme.type.titleMedium)
                    }
                }
                Section {
                    RecentList(primary, desk.transfers, desk.clips)
                    Row(Modifier.padding(top = Space.s4)) {
                        Spacer(Modifier.weight(1f))
                        Label(
                            stringResource(R.string.home_recent_all),
                            LinkTheme.type.titleSmall,
                            LinkTheme.colors.accentText,
                            Modifier.clip(Radius.pill).clickable(role = Role.Button, onClick = onActivity).padding(Space.s8),
                        )
                    }
                }
                if (missing.isEmpty() && clipboardSetup) {
                    Section { MissingLine(stringResource(R.string.home_clipboard_setup), null, onClipboardSetup) }
                }
                Spacer(Modifier.height(Space.s32))
            }
        }
    }
}

/** Nothing paired: an empty desktop, the scan action, and this phone. */
@Composable
private fun EmptyLine(onPair: () -> Unit) {
    Section {
        DesktopTile(false, Modifier.padding(bottom = Space.s16))
        Readout(stringResource(R.string.empty_desktop), LinkTheme.colors.textTertiary)
        Label(stringResource(R.string.empty_scan_title), LinkTheme.type.hero, modifier = Modifier.padding(top = Space.s8))
        Label(
            stringResource(R.string.empty_scan_body),
            LinkTheme.type.bodyLarge,
            LinkTheme.colors.textSecondary,
            Modifier.padding(top = Space.s12),
        )
        PillButton(stringResource(R.string.scan_action), onPair, Modifier.padding(top = Space.s24).fillMaxWidth())
        Spacer(Modifier.height(Space.s48))
    }
    Section {
        Label(stringResource(R.string.home_this_phone), LinkTheme.type.headlineLarge, modifier = Modifier.padding(top = Space.s4))
        Label(stringResource(R.string.home_private_hint), LinkTheme.type.bodyMedium, LinkTheme.colors.textSecondary)
    }
}

/** The 7% ink wash behind keys and buttons that sit on paper. */
@Composable
private fun wash() = LinkTheme.colors.textPrimary.copy(alpha = 0.07f)

/** The desktop as the line's top end: how it is reached, its name set large, its battery and lock, or when it was seen. */
@Composable
private fun HomeHeader(
    desktop: Desktop,
    status: DesktopStatus?,
    refreshing: Boolean,
    onConnect: () -> Unit,
    onOpen: () -> Unit,
) {
    val colors = LinkTheme.colors
    val via =
        when {
            desktop.connected && desktop.bluetooth ->
                stringResource(
                    R.string.home_connected_via,
                    stringResource(R.string.home_via_bluetooth),
                )
            desktop.connected -> stringResource(R.string.home_connected_via, stringResource(R.string.home_via_wifi))
            refreshing -> stringResource(R.string.home_via_searching)
            else -> stringResource(R.string.home_last_seen, lastSeen(desktop.lastSeen))
        }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Spacer(Modifier.weight(1f))
        val label = stringResource(R.string.desktop_open)
        Box(
            Modifier
                .size(Size.touch)
                .clip(Radius.pill)
                .clickable(role = Role.Button, onClick = onOpen)
                .semantics { contentDescription = label },
            contentAlignment = Alignment.Center,
        ) {
            Box(Modifier.size(Size.iconContainer).clip(Radius.pill).background(wash()), contentAlignment = Alignment.Center) {
                Glyph(Icons.Filled.Settings, colors.textSecondary, Size.icon - 4.dp)
            }
        }
    }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Space.s16)) {
        DesktopTile(desktop.connected)
        Column(Modifier.weight(1f)) {
            Label(desktop.name, LinkTheme.type.hero, maxLines = 2)
            Row(
                Modifier.padding(top = Space.s4),
                horizontalArrangement = Arrangement.spacedBy(Space.s6),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ConnectionOrb(desktop.connected, size = Space.s16, working = refreshing)
                Label(via, LinkTheme.type.titleSmall, if (desktop.connected) colors.accentText else colors.textTertiary)
            }
        }
    }
    val battery = status?.battery
    val locked = status?.locked
    if (battery != null || locked != null) {
        val ink = if (desktop.connected) colors.textPrimary else colors.textTertiary
        Row(Modifier.padding(top = Space.s16).height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(Space.s12)) {
            if (battery != null) {
                StatTile(
                    stringResource(R.string.home_battery, battery),
                    stringResource(if (status.charging == true) R.string.home_battery_charging else R.string.home_battery_discharging),
                    ink,
                    Modifier.weight(1f),
                ) { BatteryGlyph(battery) }
            }
            if (locked != null) {
                StatTile(
                    stringResource(if (locked) R.string.home_locked else R.string.home_unlocked),
                    null,
                    ink,
                    Modifier.weight(1f),
                ) { Glyph(if (locked) LinkIcons.Lock else LinkIcons.LockOpen, ink, Size.iconSmall) }
            }
        }
    }
    if (!desktop.connected) {
        PillButton(stringResource(R.string.connect), onConnect, Modifier.padding(top = Space.s16), enabled = !refreshing)
    }
}

/** The desktop's tile: its glyph on an accent wash while connected, on the plain wash otherwise. */
@Composable
private fun DesktopTile(
    connected: Boolean,
    modifier: Modifier = Modifier,
) {
    val colors = LinkTheme.colors
    Box(
        modifier
            .size(
                DESKTOP_TILE,
            ).clip(RoundedCornerShape(18.dp))
            .background(if (connected) colors.accent.copy(alpha = 0.14f) else wash()),
        contentAlignment = Alignment.Center,
    ) {
        Glyph(LinkIcons.Computer, if (connected) colors.accent else colors.textTertiary, Size.icon + 8.dp)
    }
}

/** One of the header's stats: a glyph, a value, and an optional caption under them. */
@Composable
private fun StatTile(
    value: String,
    caption: String?,
    ink: Color,
    modifier: Modifier = Modifier,
    glyph: @Composable () -> Unit,
) {
    Column(
        modifier
            .fillMaxHeight()
            .clip(Radius.button)
            .background(wash())
            .padding(Space.s12),
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(Space.s8), verticalAlignment = Alignment.CenterVertically) {
            glyph()
            Label(value, LinkTheme.type.titleLarge, ink)
        }
        if (caption != null) Label(caption, LinkTheme.type.bodySmall, LinkTheme.colors.textSecondary, Modifier.padding(top = Space.s4))
    }
}

@Composable
private fun BatteryGlyph(percent: Int) {
    val colors = LinkTheme.colors
    Box(
        Modifier
            .size(22.dp, 11.dp)
            .border(1.5.dp, colors.textSecondary, RoundedCornerShape(3.dp))
            .padding(2.dp),
    ) {
        Box(
            Modifier
                .fillMaxHeight()
                .fillMaxWidth(percent / 100f)
                .clip(Radius.micro)
                .background(if (percent <= LOW_BATTERY) colors.warning else colors.success),
        )
    }
}

@Composable
private fun DesktopSwitcher(
    desktops: List<Desktop>,
    primary: Desktop,
    onSelect: (Desktop) -> Unit,
) {
    LazyRow(Modifier.padding(top = Space.s16), horizontalArrangement = Arrangement.spacedBy(Space.s8)) {
        items(desktops, key = { it.id }) { desktop ->
            val active = desktop.id == primary.id
            Box(
                Modifier
                    .clip(Radius.pill)
                    .border(1.dp, if (active) LinkTheme.colors.accent else LinkTheme.colors.borderPrimary, Radius.pill)
                    .selectable(active, role = Role.Tab) { onSelect(desktop) }
                    .padding(horizontal = Space.s16, vertical = Space.s10),
            ) { Label(desktop.name, LinkTheme.type.titleSmall, maxLines = 1) }
        }
    }
}

/** The only card on Home: the desktop's player, with the position moving locally while it plays. */
@Composable
private fun PlayerCard(
    shown: DesktopPlayer,
    onOpen: () -> Unit,
    onCommand: (MediaCommandKind, Long?) -> Unit,
) {
    val colors = LinkTheme.colors
    val player = shown.player
    val playing = player.state == PlaybackState.Playing
    Column(
        Modifier
            .fillMaxWidth()
            .clip(Radius.hero)
            .background(colors.surfaceElevated)
            .border(1.dp, colors.borderSecondary, Radius.hero)
            .padding(Space.s14),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Row(
                Modifier.weight(1f).clip(Radius.button).clickable(role = Role.Button, onClick = onOpen),
                horizontalArrangement = Arrangement.spacedBy(Space.s12),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Artwork(player.artwork)
                Column(Modifier.weight(1f)) {
                    Label(player.title.ifEmpty { player.name }, LinkTheme.type.titleLarge, maxLines = 1)
                    val by =
                        if (player.artist.isEmpty()) player.name else stringResource(R.string.home_on_app, player.artist, player.name)
                    Label(by, LinkTheme.type.bodyMedium, colors.textSecondary, maxLines = 1)
                }
            }
            MediaButton(LinkIcons.Previous, stringResource(R.string.media_previous), MediaCommandKind.Previous in player.can) {
                onCommand(MediaCommandKind.Previous, null)
            }
            val playLabel = stringResource(if (playing) R.string.home_pause else R.string.home_play)
            Box(
                Modifier
                    .size(Size.touch)
                    .clip(Radius.pill)
                    .background(colors.textPrimary)
                    .clickable(enabled = MediaCommandKind.PlayPause in player.can, role = Role.Button) {
                        onCommand(MediaCommandKind.PlayPause, null)
                    }.semantics { contentDescription = playLabel },
                contentAlignment = Alignment.Center,
            ) { Glyph(if (playing) LinkIcons.Pause else Icons.Filled.PlayArrow, colors.surfacePrimary, Size.icon - 2.dp) }
            MediaButton(LinkIcons.Next, stringResource(R.string.media_next), MediaCommandKind.Next in player.can) {
                onCommand(MediaCommandKind.Next, null)
            }
        }
        Seek(player.positionMs, player.lengthMs, player.receivedAt, playing, MediaCommandKind.Seek in player.can) {
            onCommand(MediaCommandKind.Seek, it)
        }
    }
}

@Composable
private fun Artwork(bytes: ByteArray?) {
    val colors = LinkTheme.colors
    // The last image stays up while the next one decodes.
    val image =
        produceState<ImageBitmap?>(null, bytes) {
            value = withContext(Dispatchers.Default) { bytes?.let { BitmapFactory.decodeByteArray(it, 0, it.size) }?.asImageBitmap() }
        }.value
    Box(Modifier.size(52.dp).clip(Radius.button).background(colors.accent.copy(alpha = 0.16f)), contentAlignment = Alignment.Center) {
        if (image != null) {
            Image(image, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
        } else {
            Glyph(Icons.Filled.PlayArrow, colors.accentText)
        }
    }
}

@Composable
private fun MediaButton(
    icon: ImageVector,
    label: String,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Box(
        Modifier
            .size(40.dp)
            .clip(Radius.pill)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) { Glyph(icon, if (enabled) LinkTheme.colors.textSecondary else LinkTheme.colors.textDisabled, Size.icon - 2.dp) }
}

/** Times either side of a thin bar; a tap or drag seeks, once the finger lifts. */
@Composable
private fun Seek(
    positionMs: Long,
    lengthMs: Long?,
    receivedAt: Long,
    playing: Boolean,
    seekable: Boolean,
    onSeek: (Long) -> Unit,
) {
    val colors = LinkTheme.colors
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(playing, receivedAt) {
        // Playback has no change event on this side; the bar moves once a second while it plays.
        while (playing) {
            now = SystemClock.elapsedRealtime()
            delay(1_000)
        }
    }
    val position = if (playing) positionMs + (now - receivedAt) else positionMs
    // A player without a length (a live stream, or media still loading) cannot seek: only the elapsed time shows.
    if (lengthMs == null) {
        Label(clock(position), LinkTheme.type.labelMedium, colors.textTertiary, Modifier.padding(top = Space.s10))
        return
    }
    val fraction = (position.toFloat() / lengthMs.coerceAtLeast(1)).coerceIn(0f, 1f)
    Row(
        Modifier.padding(top = Space.s10),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Space.s10),
    ) {
        Label(clock((fraction * lengthMs).toLong()), LinkTheme.type.labelMedium, colors.textTertiary)
        DragTrack(
            fraction,
            stringResource(R.string.media_position),
            seekable,
            Modifier.weight(1f).height(Size.touch),
            onDone = { onSeek((it * lengthMs).toLong()) },
        ) { value, dragging ->
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(3.dp)
                    .clip(Radius.pill)
                    .background(colors.surfaceInput),
            )
            Box(
                Modifier
                    .fillMaxWidth(value)
                    .height(3.dp)
                    .clip(Radius.pill)
                    .background(colors.accent),
            )
            if (seekable) {
                Box(Modifier.fillMaxWidth(value), contentAlignment = Alignment.CenterEnd) {
                    Box(Modifier.size(if (dragging) 16.dp else 12.dp).clip(Radius.pill).background(colors.accent))
                }
            }
        }
        Label(clock(lengthMs), LinkTheme.type.labelMedium, colors.textTertiary)
    }
}

private fun clock(ms: Long): String {
    val seconds = ms / 1000
    return if (seconds >= 3600) {
        "%d:%02d:%02d".format(seconds / 3600, seconds / 60 % 60, seconds % 60)
    } else {
        "%d:%02d".format(seconds / 60, seconds % 60)
    }
}

/**
 * A horizontal track a finger taps or drags to a value from 0 to 1, reported once when the finger lifts; [draw] shows
 * the moving value. Accessibility services set it through its progress range.
 */
@Composable
private fun DragTrack(
    value: Float,
    label: String,
    enabled: Boolean,
    modifier: Modifier = Modifier,
    onDone: (Float) -> Unit = {},
    onChange: ((Float) -> Unit)? = null,
    draw: @Composable (value: Float, dragging: Boolean) -> Unit,
) {
    var pending by remember { mutableStateOf<Float?>(null) }
    val done by rememberUpdatedState<(Float) -> Unit> {
        pending = it
        onDone(it)
    }
    val change by rememberUpdatedState(onChange)
    var dragging by remember { mutableStateOf<Float?>(null) }
    var sentAt by remember { mutableLongStateOf(0L) }
    var width by remember { mutableIntStateOf(1) }
    // The set value stays shown until a report comes close to it, so an older report does not pull the track back.
    LaunchedEffect(pending, value) {
        val target = pending ?: return@LaunchedEffect
        if (abs(value - target) > SETTLED) delay(SETTLE_MS)
        pending = null
    }
    val shown = (dragging ?: pending ?: value).coerceIn(0f, 1f)
    val at = { x: Float -> (x / width).coerceIn(0f, 1f) }
    val input =
        if (enabled) {
            Modifier
                .pointerInput(Unit) { detectTapGestures { done(at(it.x)) } }
                .pointerInput(Unit) {
                    detectHorizontalDragGestures(
                        onDragStart = { dragging = at(it.x) },
                        onDragEnd = {
                            dragging?.let(done)
                            dragging = null
                        },
                        onDragCancel = { dragging = null },
                    ) { event, _ ->
                        val now = at(event.position.x)
                        dragging = now
                        val send = change
                        if (send != null && event.uptimeMillis - sentAt >= LIVE_MS) {
                            sentAt = event.uptimeMillis
                            pending = now
                            send(now)
                        }
                    }
                }
        } else {
            Modifier
        }
    Box(
        modifier
            .onSizeChanged { width = it.width.coerceAtLeast(1) }
            .semantics {
                contentDescription = label
                progressBarRangeInfo = ProgressBarRangeInfo(shown, 0f..1f)
                if (enabled) {
                    setProgress { target ->
                        done(target.coerceIn(0f, 1f))
                        true
                    }
                }
            }.then(input),
        contentAlignment = Alignment.CenterStart,
    ) { draw(shown, dragging != null) }
}

/** Volume, brightness, and the keys: what this phone can do to the desktop now. */
@Composable
private fun Controls(
    desktop: Desktop,
    /** Null when the desktop does not list its capabilities. */
    capabilities: Set<String>?,
    status: DesktopStatus?,
    volume: DesktopVolume?,
    ringing: Boolean,
    viewModel: HomeViewModel,
    onDesktop: (Desktop) -> Unit,
    onRemote: (Desktop) -> Unit,
    onPresenter: (Desktop) -> Unit,
    onViewer: (Desktop) -> Unit,
    onClipboard: () -> Unit,
) {
    val can = capabilities.orEmpty()
    val granted = CAN_CONTROL in can
    // As on the Device page: a desktop that does not list its capabilities hides nothing.
    val input = capabilities?.contains(CAN_INPUT) ?: true
    val live = desktop.connected
    var confirmLock by remember { mutableStateOf(false) }
    Column {
        val dim = Modifier.alpha(if (live) 1f else DISABLED)
        volume?.let { volume ->
            LevelSlider(
                if (volume.muted) LinkIcons.Muted else LinkIcons.Ring,
                stringResource(R.string.home_volume),
                volume.percent,
                live && granted,
                dim,
                onIcon = { viewModel.setMuted(desktop, !volume.muted) },
            ) { viewModel.setVolume(desktop, it, unmute = volume.muted) }
        }
        val brightness = status?.brightness
        if (brightness != null && "brightness" in can) {
            LevelSlider(LinkIcons.Brightness, stringResource(R.string.home_brightness), brightness, live && granted, dim) {
                viewModel.setBrightness(desktop, it)
            }
        }
        val system = { capability: String -> if (capability in can) (if (granted) KeyState.Off else KeyState.NotAllowed) else null }
        val shared = { on: Boolean -> if (on) KeyState.Off else KeyState.NotAllowed }
        val keys =
            listOfNotNull(
                system("lock")?.let { Key(LinkIcons.Lock, R.string.key_lock, it) { confirmLock = true } },
                system("suspend")?.let { Key(LinkIcons.Sleep, R.string.key_sleep, it) { viewModel.sleep(desktop) } },
                system("screen-off")?.let { Key(LinkIcons.ScreenOff, R.string.key_screen_off, it) { viewModel.screenOff(desktop) } },
                Key(
                    LinkIcons.Ring,
                    if (ringing) R.string.key_ringing else R.string.key_ring,
                    if (ringing) KeyState.On else shared(desktop.sharing.ring),
                ) { viewModel.ring(desktop, !ringing) },
                Key(LinkIcons.Touchpad, R.string.key_touchpad, shared(desktop.control && input)) { onRemote(desktop) },
                if (CAN_SCREEN in can) {
                    Key(LinkIcons.Computer, R.string.key_screen, shared(desktop.control && input)) { onViewer(desktop) }
                } else {
                    null
                },
                Key(LinkIcons.Presenter, R.string.key_presenter, shared(desktop.control && input)) { onPresenter(desktop) },
                // A clip sent while the desktop is away waits for it, so Clipboard works offline too.
                Key(LinkIcons.Clipboard, R.string.key_clipboard, shared(desktop.sharing.clipboard), offline = true, onClick = onClipboard),
                system("dnd")?.let { allowed ->
                    val on = status?.dnd == true
                    Key(LinkIcons.Dnd, R.string.key_dnd, if (on && allowed == KeyState.Off) KeyState.On else allowed) {
                        viewModel.setDnd(desktop, !on)
                    }
                },
            )
        Column(Modifier.padding(top = Space.s20), verticalArrangement = Arrangement.spacedBy(Space.s16)) {
            keys.chunked(KEYS_PER_ROW).forEach { row ->
                Row {
                    row.forEach { key ->
                        KeyButton(key, live || key.offline, Modifier.weight(1f)) {
                            if (key.state ==
                                KeyState.NotAllowed
                            ) {
                                onDesktop(desktop)
                            } else {
                                key.onClick()
                            }
                        }
                    }
                    repeat(KEYS_PER_ROW - row.size) { Spacer(Modifier.weight(1f)) }
                }
            }
        }
        val controlOff = !granted && can.any { it in SYSTEM_CAPABILITIES }
        val inputOff = desktop.control && !input
        if (controlOff) {
            Label(
                stringResource(R.string.home_control_off, desktop.name),
                LinkTheme.type.bodyMedium,
                LinkTheme.colors.textSecondary,
                Modifier.padding(top = Space.s14),
            )
        }
        if (inputOff) {
            Label(
                stringResource(R.string.home_input_off, desktop.name),
                LinkTheme.type.bodyMedium,
                LinkTheme.colors.textSecondary,
                Modifier.padding(top = Space.s14),
            )
        }
        if (controlOff || inputOff) {
            Label(
                stringResource(R.string.home_allow_on, desktop.name),
                LinkTheme.type.titleSmall,
                LinkTheme.colors.accentText,
                Modifier
                    .padding(top = Space.s4)
                    .clip(Radius.button)
                    .clickable(role = Role.Button) { onDesktop(desktop) }
                    .padding(vertical = Space.s8),
            )
        }
        if (confirmLock) {
            Dialog(onDismissRequest = { confirmLock = false }) {
                DashboardCard(Modifier.fillMaxWidth()) {
                    Label(stringResource(R.string.home_lock_title, desktop.name), LinkTheme.type.headlineLarge)
                    PillButton(
                        stringResource(R.string.key_lock),
                        {
                            confirmLock = false
                            viewModel.lock(desktop)
                        },
                        Modifier.fillMaxWidth(),
                        PillKind.Filled,
                    )
                    PillButton(stringResource(R.string.cancel), { confirmLock = false }, Modifier.fillMaxWidth(), PillKind.Quiet)
                }
            }
        }
    }
}

private enum class KeyState { Off, On, NotAllowed }

private class Key(
    val icon: ImageVector,
    val label: Int,
    val state: KeyState,
    val offline: Boolean = false,
    val onClick: () -> Unit,
)

@Composable
private fun KeyButton(
    key: Key,
    enabled: Boolean,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    val colors = LinkTheme.colors
    val label = stringResource(key.label)
    val off = key.state == KeyState.NotAllowed
    val on = key.state == KeyState.On
    val notAllowed = stringResource(R.string.home_key_not_allowed)
    Column(
        modifier
            .alpha(if (enabled) 1f else DISABLED)
            .clip(Radius.list)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .semantics(mergeDescendants = true) { if (off) stateDescription = notAllowed }
            .padding(vertical = Space.s4),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Space.s6),
    ) {
        val shape = RoundedCornerShape(18.dp)
        Box(
            Modifier
                .size(54.dp)
                .clip(shape)
                .then(
                    when {
                        on -> Modifier.background(colors.accent)
                        off -> Modifier.border(1.5.dp, colors.borderPrimary, shape)
                        else -> Modifier.background(wash())
                    },
                ),
            contentAlignment = Alignment.Center,
        ) {
            Glyph(
                key.icon,
                if (on) {
                    colors.onAccent
                } else if (off) {
                    colors.textDisabled
                } else {
                    colors.textPrimary
                },
                Size.icon - 2.dp,
            )
        }
        Label(
            label,
            if (on) LinkTheme.type.labelLarge else LinkTheme.type.bodySmall,
            if (off) {
                colors.textDisabled
            } else if (on) {
                colors.textPrimary
            } else {
                colors.textSecondary
            },
            maxLines = 1,
        )
    }
}

/** A 52 dp track the finger drags: solid accent up to the value, the icon riding in the fill, the number at the end. */
@Composable
private fun LevelSlider(
    icon: ImageVector,
    label: String,
    percent: Int,
    enabled: Boolean,
    modifier: Modifier = Modifier,
    /** Tapping the icon, as mute; null when the icon is only a label. */
    onIcon: (() -> Unit)? = null,
    onSet: (Int) -> Unit,
) {
    val colors = LinkTheme.colors
    val shape = RoundedCornerShape(16.dp)
    DragTrack(
        percent / 100f,
        label,
        enabled,
        modifier
            .padding(top = Space.s8)
            .fillMaxWidth()
            .height(52.dp)
            .clip(shape)
            .background(colors.surfaceInput),
        onDone = { onSet((it * 100).roundToInt()) },
        onChange = { onSet((it * 100).roundToInt()) },
    ) { value, _ ->
        Box(
            Modifier
                .fillMaxHeight()
                .widthIn(min = 52.dp)
                .fillMaxWidth(value)
                .clip(shape)
                .background(if (enabled) colors.accent else colors.textDisabled),
        )
        Row(Modifier.fillMaxWidth().padding(horizontal = Space.s16), verticalAlignment = Alignment.CenterVertically) {
            val tap =
                if (onIcon != null && enabled) {
                    Modifier
                        .clip(
                            Radius.pill,
                        ).clickable(role = Role.Button, onClickLabel = stringResource(R.string.home_mute), onClick = onIcon)
                } else {
                    Modifier
                }
            Box(tap.size(36.dp), contentAlignment = Alignment.Center) { Glyph(icon, colors.onAccent, Size.icon - 4.dp) }
            Spacer(Modifier.weight(1f))
            Label("${(value * 100).roundToInt()}", LinkTheme.type.titleMedium, colors.textSecondary)
        }
    }
}

/** Where things leave the phone: recent photos to tap, the picker's tabs, and Send once anything is picked. */
@Composable
private fun SendTray(
    desktop: Desktop,
    picker: PickerViewModel,
    onSend: () -> Unit,
) {
    val pick by picker.state.collectAsStateWithLifecycle()
    val colors = LinkTheme.colors
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { picker.refresh() }
    Label(stringResource(R.string.home_send_to, desktop.name), LinkTheme.type.titleMedium)
    if (pick.access == MediaAccess.None) {
        Row(
            Modifier
                .padding(top = Space.s12)
                .fillMaxWidth()
                .clip(Radius.list)
                .background(wash())
                .clickable { ask.launch(PhoneFiles.mediaPermissions) }
                .padding(Space.s16),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Label(stringResource(R.string.recent_allow), LinkTheme.type.bodyMedium, colors.textSecondary, Modifier.weight(1f))
            Label(stringResource(R.string.picker_media_allow), LinkTheme.type.titleSmall, colors.accentText)
        }
    } else if (pick.photos.isNotEmpty()) {
        LazyRow(Modifier.padding(top = Space.s12), horizontalArrangement = Arrangement.spacedBy(Space.s6)) {
            items(pick.photos.take(RECENT), key = { it.key }) { item ->
                MediaTile(item, pick.selected.keys.indexOf(item.key), picker::thumbnail, Modifier.size(76.dp)) { picker.toggle(item) }
            }
        }
    }
    Row(Modifier.padding(top = Space.s12), horizontalArrangement = Arrangement.spacedBy(Space.s8)) {
        listOf(
            Triple(PickerTab.Photos, R.string.picker_photos, LinkIcons.Photo),
            Triple(PickerTab.Videos, R.string.picker_videos, LinkIcons.Video),
            Triple(PickerTab.Files, R.string.picker_files, LinkIcons.Folder),
        ).forEach { (tab, label, icon) ->
            Row(
                Modifier
                    .weight(1f)
                    .height(Size.touch)
                    .clip(RoundedCornerShape(14.dp))
                    .background(wash())
                    .clickable(role = Role.Button) { picker.open(tab) },
                horizontalArrangement = Arrangement.spacedBy(Space.s8, Alignment.CenterHorizontally),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Glyph(icon, colors.textPrimary, Size.iconSmall + 2.dp)
                Label(stringResource(label), LinkTheme.type.titleMedium, maxLines = 1)
            }
        }
    }
    if (pick.selected.isNotEmpty()) {
        PillButton(sendLabel(pick.selected), onSend, Modifier.padding(top = Space.s12).fillMaxWidth(), icon = LinkIcons.Upload)
    }
}

/** One recent thing that crossed the link, newest first; [time] is Unix milliseconds. */
private data class Crossed(
    val name: String,
    val detail: String,
    val time: Long,
)

/** The last three transfers and clips with this desktop, each branching off the line. */
@Composable
private fun RecentList(
    desktop: Desktop,
    transfers: List<TransferRecord>,
    clips: List<RecentClip>,
) {
    val context = LocalContext.current
    val resources = context.resources
    val colors = LinkTheme.colors
    Label(stringResource(R.string.home_recent), LinkTheme.type.titleMedium)
    val recent =
        remember(desktop.id, desktop.name, transfers, clips, resources) {
            val sent =
                transfers
                    .filter { it.desktopName == desktop.name && it.files.isNotEmpty() }
                    .map { record ->
                        val first = record.files.first().name
                        val more = record.files.size - 1
                        val name = if (more > 0) resources.getQuantityString(R.plurals.recent_more_files, more, first, more) else first
                        val size = Formatter.formatShortFileSize(context, record.files.sumOf { it.size })
                        val failed = record.status !in LIVE_STATUSES
                        val detail =
                            when {
                                failed && record.incoming == true -> resources.getString(R.string.recent_not_received)
                                failed -> resources.getString(R.string.recent_failed)
                                record.incoming == true -> resources.getString(R.string.recent_received, size)
                                else -> resources.getString(R.string.recent_sent, size)
                            }
                        Crossed(name, detail, record.time)
                    }
            val copied =
                clips
                    .filter { it.desktopId == null || it.desktopId == desktop.id }
                    .map { clip ->
                        Crossed(
                            clip.text
                                ?.lines()
                                ?.firstOrNull { it.isNotBlank() }
                                ?.trim() ?: resources.getString(R.string.recent_image),
                            resources.getString(
                                if (clip.incoming) R.string.recent_copied_from else R.string.recent_copied_to,
                                desktop.name,
                            ),
                            clip.time,
                        )
                    }
            (sent + copied).sortedByDescending { it.time }.take(RECENT_ITEMS)
        }
    if (recent.isEmpty()) {
        Label(stringResource(R.string.home_recent_empty), LinkTheme.type.bodyMedium, colors.textSecondary, Modifier.padding(top = Space.s8))
        return
    }
    val now = System.currentTimeMillis()
    recent.forEach { item ->
        Row(
            Modifier
                .fillMaxWidth()
                .padding(top = Space.s12),
            horizontalArrangement = Arrangement.spacedBy(Space.s12),
        ) {
            Column(Modifier.weight(1f)) {
                Label(item.name, LinkTheme.type.bodyLarge, maxLines = 1)
                Label(item.detail, LinkTheme.type.bodySmall, colors.textTertiary)
            }
            Label(
                DateUtils
                    .getRelativeTimeSpanString(
                        item.time,
                        now,
                        DateUtils.MINUTE_IN_MILLIS,
                        DateUtils.FORMAT_ABBREV_RELATIVE,
                    ).toString(),
                LinkTheme.type.bodySmall,
                colors.textTertiary,
            )
        }
    }
}

/** The setup card on Home while required grants are missing; it has no close, so it stays until setup is done. */
@Composable
private fun SetupCard(
    done: Int,
    total: Int,
    onContinue: () -> Unit,
) {
    val colors = LinkTheme.colors
    DashboardCard(Modifier.fillMaxWidth(), onClick = onContinue) {
        Label(stringResource(R.string.setup_card_title), LinkTheme.type.titleMedium, modifier = Modifier.padding(top = Space.s8))
        Label(pluralStringResource(R.plurals.setup_card_progress, total, done, total), LinkTheme.type.bodyMedium, colors.textSecondary)
        Box(
            Modifier
                .fillMaxWidth()
                .height(6.dp)
                .clip(Radius.pill)
                .background(colors.surfacePrimary),
        ) {
            Box(
                Modifier
                    .fillMaxWidth(done.toFloat() / total)
                    .height(6.dp)
                    .clip(Radius.pill)
                    .background(colors.accent),
            )
        }
        Row(Modifier.fillMaxWidth().padding(bottom = Space.s8)) {
            Spacer(Modifier.weight(1f))
            PillButton(stringResource(R.string.setup_card_continue), onContinue)
        }
    }
}

@Composable
private fun MissingLine(
    title: String,
    detail: String?,
    onClick: () -> Unit,
) {
    // The press shape reaches 8 dp past the text on each side, so its corners do not cut the text's first letter.
    Column(
        Modifier
            .offset(x = -Space.s8)
            .fillMaxWidth()
            .clip(Radius.list)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(horizontal = Space.s8, vertical = Space.s4),
    ) {
        Label(title, LinkTheme.type.titleMedium)
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Space.s8)) {
            if (detail != null) Label(detail, LinkTheme.type.bodyMedium, LinkTheme.colors.textSecondary)
            Label(stringResource(R.string.missing_turn_on), LinkTheme.type.titleSmall, LinkTheme.colors.accentText)
        }
    }
}

@Composable
internal fun lastSeen(unixSeconds: Long): String =
    if (unixSeconds <= 0) stringResource(R.string.last_seen_never) else DateUtils.getRelativeTimeSpanString(unixSeconds * 1000).toString()

private val SYSTEM_CAPABILITIES = setOf("lock", "suspend", "screen-off", "brightness", "dnd", "volume")
private const val KEYS_PER_ROW = 4
private const val RECENT_ITEMS = 3

/** Transfer statuses that are not a failure; the rest (failed, declined, no-space, ...) are. */
private val DESKTOP_TILE = 64.dp

/** A level slider sends at most this often while dragged, ms. */
private const val LIVE_MS = 100L

/** How close a report must come to the set value to count as caught up, as a fraction of the track. */
private const val SETTLED = 0.02f

/** How long the set value is shown when no report comes close to it, ms. */
private const val SETTLE_MS = 1500L
private val LIVE_STATUSES = setOf("offered", "transferring", "done")
private const val LOW_BATTERY = 15
private const val DISABLED = 0.45f
private const val RECENT = 12

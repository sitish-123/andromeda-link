package org.andromeda.link.devices

import android.content.Context
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.annotation.StringRes
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.core.net.toUri
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.andromeda.link.R
import org.andromeda.link.clipboard.LogAccess
import org.andromeda.link.core.domain.Feature
import org.andromeda.link.files.PhoneFiles
import org.andromeda.link.screen.ScreenInput
import org.andromeda.link.ui.components.DashboardCard
import org.andromeda.link.ui.components.Glyph
import org.andromeda.link.ui.components.Label
import org.andromeda.link.ui.components.LinkIcons
import org.andromeda.link.ui.components.PaperHeading
import org.andromeda.link.ui.components.PaperRow
import org.andromeda.link.ui.components.PillButton
import org.andromeda.link.ui.components.PillKind
import org.andromeda.link.ui.components.Screen
import org.andromeda.link.ui.components.Section
import org.andromeda.link.ui.text
import org.andromeda.link.ui.theme.LinkTheme
import org.andromeda.link.ui.theme.Radius
import org.andromeda.link.ui.theme.Space

/** One desktop: what this phone shares with it, the per-app notification filter, and unpairing. */
@Composable
fun DeviceScreen(
    viewModel: DeviceViewModel,
    onBack: () -> Unit,
    onMirrorApps: () -> Unit,
    onRemote: () -> Unit,
    onPresenter: () -> Unit,
    onViewer: () -> Unit,
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val unpaired by viewModel.unpaired.collectAsStateWithLifecycle()
    var confirmUnpair by rememberSaveable { mutableStateOf(false) }
    var controlEnabled by rememberSaveable { mutableStateOf(ScreenInput.enabled || LogAccess.helperPossible()) }
    val context = LocalContext.current
    var allFiles by rememberSaveable { mutableStateOf(PhoneFiles.allFiles(context)) }
    LifecycleResumeEffect(Unit) {
        controlEnabled = ScreenInput.enabled || LogAccess.helperPossible()
        allFiles = PhoneFiles.allFiles(context)
        onPauseOrDispose {}
    }
    val resources = LocalContext.current.resources
    LaunchedEffect(unpaired) { if (unpaired) onBack() }
    val desktop = state.desktop ?: return Screen(stringResource(R.string.desktops_title), onBack) {}
    Screen(
        title = null,
        onBack = onBack,
        toast = state.failure?.text(resources),
        onToastShown = viewModel::failureShown,
    ) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            Section {
                val via =
                    when {
                        desktop.connected && desktop.bluetooth -> stringResource(R.string.home_via_bluetooth)
                        desktop.connected -> stringResource(R.string.home_via_wifi)
                        else -> stringResource(R.string.home_last_seen, lastSeen(desktop.lastSeen))
                    }
                Label(via, LinkTheme.type.titleSmall, if (desktop.connected) LinkTheme.colors.accentText else LinkTheme.colors.textTertiary)
                Label(desktop.name, LinkTheme.type.hero, modifier = Modifier.padding(top = Space.s4), maxLines = 2)
                if (!desktop.connected) {
                    PillButton(
                        stringResource(R.string.connect),
                        viewModel::connect,
                        Modifier.padding(top = Space.s16),
                        enabled = !state.busy,
                    )
                }
            }
            Section {
                PaperHeading(stringResource(R.string.device_phone_grants, desktop.name))
                val sharing = desktop.sharing
                listOf(
                    FeatureRow(Feature.Clipboard, R.string.feature_clipboard, R.string.feature_clipboard_hint, LinkIcons.Clipboard),
                    FeatureRow(Feature.Files, R.string.feature_files, R.string.feature_files_hint, LinkIcons.Folder),
                    FeatureRow(
                        Feature.Notifications,
                        R.string.feature_notifications,
                        R.string.feature_notifications_hint,
                        Icons.Filled.Notifications,
                    ),
                    FeatureRow(Feature.Media, R.string.feature_media, R.string.feature_media_hint, Icons.Filled.PlayArrow),
                    FeatureRow(Feature.Ring, R.string.feature_ring, R.string.feature_ring_hint, LinkIcons.Ring),
                    FeatureRow(Feature.Calls, R.string.feature_calls, R.string.feature_calls_hint, Icons.Filled.Call),
                    FeatureRow(Feature.Browse, R.string.feature_browse, R.string.feature_browse_hint, LinkIcons.Folder),
                    FeatureRow(Feature.Messages, R.string.feature_messages, R.string.feature_messages_hint, LinkIcons.Contacts),
                    FeatureRow(Feature.Screen, R.string.feature_screen, R.string.feature_screen_hint, LinkIcons.Computer),
                ).filter { row ->
                    when (row.feature) {
                        Feature.Clipboard -> state.desktopCan(CAN_CLIPBOARD)
                        Feature.Notifications -> state.desktopCan(CAN_NOTIFICATIONS)
                        else -> true
                    }
                }.forEachIndexed { index, row ->
                    PaperRow(
                        stringResource(row.label),
                        subtitle = stringResource(row.hint),
                        icon = row.icon,
                        divided = index > 0,
                        enabled = !state.busy,
                        checked = sharing.allows(row.feature),
                        onToggle = { viewModel.setSharing(row.feature, it) },
                    )
                    if (row.feature == Feature.Notifications && sharing.allows(Feature.Notifications)) {
                        Label(
                            stringResource(R.string.mirror_apps_choose),
                            LinkTheme.type.titleSmall,
                            LinkTheme.colors.accentText,
                            Modifier
                                .padding(start = 36.dp)
                                .clip(Radius.pill)
                                .clickable(role = Role.Button, onClick = onMirrorApps)
                                .padding(vertical = Space.s8),
                        )
                    }
                    if (row.feature == Feature.Browse && sharing.browse && !allFiles) {
                        Label(
                            stringResource(R.string.device_browse_photos_only, desktop.name),
                            LinkTheme.type.bodyMedium,
                            LinkTheme.colors.textSecondary,
                            Modifier.padding(start = 36.dp),
                        )
                        Label(
                            stringResource(R.string.settings_turn_on),
                            LinkTheme.type.titleSmall,
                            LinkTheme.colors.accentText,
                            Modifier
                                .padding(start = 36.dp)
                                .clip(Radius.pill)
                                .clickable(role = Role.Button) { context.startActivity(allFilesSettings(context)) }
                                .padding(vertical = Space.s8),
                        )
                    }
                    if (row.feature == Feature.Screen && sharing.screen && !controlEnabled) {
                        PaperRow(
                            stringResource(R.string.screen_control_off),
                            icon = Icons.Filled.Lock,
                            onClick = { context.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) },
                        ) { Glyph(Icons.AutoMirrored.Filled.KeyboardArrowRight, LinkTheme.colors.textTertiary) }
                    }
                }
            }
            Section {
                PaperHeading(stringResource(R.string.device_desktop_grants, desktop.name))
                val allowed = state.capabilities?.contains(CAN_CONTROL) == true
                PaperRow(
                    stringResource(R.string.device_control_system),
                    subtitle = stringResource(R.string.device_control_system_hint),
                    icon = LinkIcons.Brightness,
                    divided = false,
                ) {
                    Label(
                        stringResource(if (allowed) R.string.device_allowed else R.string.device_not_allowed),
                        LinkTheme.type.titleSmall,
                        if (allowed) LinkTheme.colors.success else LinkTheme.colors.textTertiary,
                    )
                }
                val chevron = @Composable { Glyph(Icons.AutoMirrored.Filled.KeyboardArrowRight, LinkTheme.colors.textTertiary) }
                if (desktop.control) {
                    val input = state.desktopCan(CAN_INPUT)
                    val inputState =
                        @Composable {
                            if (input) {
                                chevron()
                            } else {
                                Label(stringResource(R.string.device_not_allowed), LinkTheme.type.titleSmall, LinkTheme.colors.textTertiary)
                            }
                        }
                    PaperRow(
                        stringResource(R.string.remote_title),
                        subtitle = stringResource(R.string.device_touchpad_hint),
                        icon = LinkIcons.Touchpad,
                        enabled = input,
                        onClick = onRemote,
                        trailing = inputState,
                    )
                    PaperRow(
                        stringResource(R.string.presenter_title),
                        subtitle = stringResource(R.string.device_presenter_hint),
                        icon = LinkIcons.Presenter,
                        enabled = input,
                        onClick = onPresenter,
                        trailing = inputState,
                    )
                    // Only a desktop that says it can share its screen offers it; older ones cannot.
                    if (state.capabilities?.contains(CAN_SCREEN) == true) {
                        PaperRow(
                            stringResource(R.string.viewer_title),
                            subtitle = stringResource(R.string.device_viewer_hint),
                            icon = LinkIcons.Computer,
                            enabled = input,
                            onClick = onViewer,
                            trailing = inputState,
                        )
                    }
                }
                if (desktop.control) {
                    PaperRow(
                        stringResource(R.string.ping),
                        subtitle = stringResource(R.string.ping_hint),
                        icon = Icons.Filled.Notifications,
                        onClick = viewModel::ping,
                        trailing = chevron,
                    )
                }
                Label(
                    stringResource(R.string.device_desktop_decides, desktop.name),
                    LinkTheme.type.bodyMedium,
                    LinkTheme.colors.textSecondary,
                    Modifier.padding(top = Space.s8),
                )
            }
            if (desktop.control && state.commands.isNotEmpty()) {
                Section {
                    PaperHeading(stringResource(R.string.device_run_on, desktop.name))
                    FlowRow(
                        Modifier.padding(top = Space.s8),
                        horizontalArrangement = Arrangement.spacedBy(Space.s6),
                        verticalArrangement = Arrangement.spacedBy(Space.s8),
                    ) {
                        state.commands.forEach { command ->
                            PillButton(command.name, { viewModel.run(command) }, kind = PillKind.Tonal, enabled = !state.busy)
                        }
                    }
                }
            }
            Section {
                Label(
                    stringResource(R.string.device_unpair, desktop.name),
                    LinkTheme.type.titleMedium,
                    LinkTheme.colors.warning,
                    Modifier
                        .padding(top = Space.s14)
                        .clip(Radius.pill)
                        .clickable(enabled = !state.busy, role = Role.Button) { confirmUnpair = true }
                        .padding(vertical = Space.s8),
                )
            }
            Spacer(Modifier.height(Space.s32))
        }
        if (confirmUnpair) {
            Dialog(onDismissRequest = { if (!state.busy) confirmUnpair = false }) {
                DashboardCard(Modifier.fillMaxWidth()) {
                    Label(stringResource(R.string.device_unpair_title), LinkTheme.type.headlineLarge)
                    Label(
                        stringResource(R.string.device_unpair_body, desktop.name),
                        LinkTheme.type.bodyLarge,
                        LinkTheme.colors.textSecondary,
                    )
                    PillButton(
                        stringResource(R.string.unpair),
                        viewModel::unpair,
                        Modifier.fillMaxWidth(),
                        PillKind.Destructive,
                        enabled = !state.busy,
                    )
                    PillButton(
                        stringResource(R.string.cancel),
                        { confirmUnpair = false },
                        Modifier.fillMaxWidth(),
                        PillKind.Quiet,
                        enabled = !state.busy,
                    )
                }
            }
        }
    }
}

/** Android 11+ grants All files access on its own page; Android 10 has only the storage permission in App info. */
private fun allFilesSettings(context: Context): Intent =
    Intent(
        if (Build.VERSION.SDK_INT >= 30) {
            Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION
        } else {
            Settings.ACTION_APPLICATION_DETAILS_SETTINGS
        },
        "package:${context.packageName}".toUri(),
    )

private data class FeatureRow(
    val feature: Feature,
    @param:StringRes val label: Int,
    @param:StringRes val hint: Int,
    val icon: ImageVector,
)

package org.andromeda.link.devices

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Process
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.andromeda.link.R
import org.andromeda.link.core.domain.Desktop
import org.andromeda.link.onboarding.Grant
import org.andromeda.link.ui.components.Glyph
import org.andromeda.link.ui.components.Label
import org.andromeda.link.ui.components.LinkIcons
import org.andromeda.link.ui.components.PaperHeading
import org.andromeda.link.ui.components.PaperRow
import org.andromeda.link.ui.components.Screen
import org.andromeda.link.ui.components.SlidingPillControl
import org.andromeda.link.ui.theme.AppTheme
import org.andromeda.link.ui.theme.LinkTheme
import org.andromeda.link.ui.theme.Radius
import org.andromeda.link.ui.theme.Space
import org.andromeda.link.ui.theme.rememberAppTheme
import org.andromeda.link.ui.theme.setAppTheme

/** The app's own settings (DESIGN.md, Settings): desktops, staying connected, notifications, clipboard, permissions. */
@Composable
fun SettingsScreen(
    viewModel: HomeViewModel,
    autoClipboard: Boolean,
    /** Required grants not given yet. */
    missing: List<Grant>,
    onDesktop: (Desktop) -> Unit,
    onPair: () -> Unit,
    onNotifications: () -> Unit,
    onMirrorApps: () -> Unit,
    onSetup: () -> Unit,
    onClipboardSetup: () -> Unit,
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val clearsSynced by viewModel.clearsSynced.collectAsStateWithLifecycle()
    val prefs by viewModel.notificationPrefs.collectAsStateWithLifecycle()
    val theme by rememberAppTheme()
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val colors = LinkTheme.colors
    val version =
        remember(context) {
            context.packageManager
                .getPackageInfo(context.packageName, 0)
                .versionName
                .orEmpty()
        }
    var logs by remember { mutableStateOf(false) }
    // Read again on resume: the grant comes from adb, Shizuku or root while Link is in the background.
    LifecycleResumeEffect(Unit) {
        logs = context.checkSelfPermission(Manifest.permission.READ_LOGS) == PackageManager.PERMISSION_GRANTED
        onPauseOrDispose {}
    }
    val chevron = @Composable { Glyph(Icons.AutoMirrored.Filled.KeyboardArrowRight, colors.textTertiary) }
    val stateWord = @Composable { on: Boolean, onText: Int, offText: Int ->
        Label(stringResource(if (on) onText else offText), LinkTheme.type.titleSmall, if (on) colors.success else colors.warning)
    }
    Screen(null) {
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = Space.page)) {
            Label(stringResource(R.string.settings_title), LinkTheme.type.displayLarge, modifier = Modifier.padding(top = Space.s16))

            PaperHeading(stringResource(R.string.settings_desktops))
            state.desktops.forEachIndexed { index, desktop ->
                PaperRow(
                    desktop.name,
                    subtitle =
                        if (desktop.connected) {
                            stringResource(
                                if (desktop.bluetooth) R.string.settings_connected_bluetooth else R.string.settings_connected_wifi,
                            )
                        } else {
                            stringResource(R.string.home_last_seen, lastSeen(desktop.lastSeen))
                        },
                    divided = index > 0,
                    onClick = { onDesktop(desktop) },
                    trailing = chevron,
                    leading = { StatusDot(desktop.connected) },
                )
            }
            PaperRow(stringResource(R.string.pair_action), icon = Icons.Filled.Add, divided = state.desktops.isNotEmpty(), onClick = onPair)

            PaperHeading(stringResource(R.string.stay_connected))
            PaperRow(
                stringResource(R.string.settings_keep_open),
                subtitle = stringResource(R.string.settings_keep_open_hint),
                icon = LinkIcons.Transfers,
                divided = false,
                checked = state.stayConnected,
                onToggle = viewModel::setStayConnected,
            )
            if (state.stayConnected) LiveNotificationPreview(state.primary?.name)

            PaperHeading(stringResource(R.string.settings_notifications))
            PaperRow(
                stringResource(R.string.settings_notification_access),
                subtitle = stringResource(R.string.settings_notification_access_hint),
                icon = Icons.Filled.Notifications,
                divided = false,
                onClick = onNotifications,
            ) { stateWord(state.mirrorGranted, R.string.settings_on, R.string.settings_turn_on) }
            PaperRow(
                stringResource(R.string.settings_apps),
                subtitle =
                    if (prefs.excluded == 0) {
                        stringResource(R.string.settings_apps_all)
                    } else {
                        pluralStringResource(R.plurals.settings_apps_except, prefs.excluded, prefs.excluded)
                    },
                icon = LinkIcons.Brightness,
                onClick = onMirrorApps,
                trailing = chevron,
            )
            PaperRow(
                stringResource(R.string.settings_quiet_apps),
                subtitle =
                    if (prefs.lowImportance == 0) {
                        stringResource(R.string.settings_quiet_apps_none)
                    } else {
                        pluralStringResource(R.plurals.settings_quiet_apps_count, prefs.lowImportance, prefs.lowImportance)
                    },
                icon = Icons.Filled.Notifications,
                onClick = onMirrorApps,
                trailing = chevron,
            )
            PaperRow(
                stringResource(R.string.settings_full_locked),
                subtitle =
                    stringResource(
                        if (prefs.fullWhileLocked) R.string.settings_full_locked_on else R.string.settings_full_locked_off,
                    ),
                icon = Icons.Filled.Lock,
                checked = prefs.fullWhileLocked,
                onToggle = viewModel::setFullContentWhileLocked,
            )

            PaperHeading(stringResource(R.string.settings_clipboard))
            PaperRow(
                stringResource(R.string.clipboard_auto_row),
                subtitle = stringResource(R.string.settings_auto_clipboard_hint),
                icon = LinkIcons.Clipboard,
                divided = false,
                onClick = onClipboardSetup,
            ) { stateWord(autoClipboard, R.string.settings_on, R.string.settings_set_up) }
            PaperRow(
                stringResource(R.string.settings_log_access),
                subtitle = stringResource(R.string.settings_log_access_hint),
                icon = Icons.Filled.Lock,
            ) {
                Label(
                    stringResource(if (logs) R.string.settings_on else R.string.settings_off),
                    LinkTheme.type.titleSmall,
                    if (logs) colors.success else colors.textTertiary,
                )
            }
            PaperRow(
                stringResource(R.string.settings_clear_clips),
                subtitle = stringResource(R.string.settings_clear_clips_hint),
                icon = Icons.Filled.Delete,
                checked = clearsSynced,
                onToggle = viewModel::setClearsSynced,
            )

            val required = Grant.entries.filter { it != Grant.Texts && it != Grant.AutoClipboard }
            PaperHeading(
                stringResource(R.string.setup_title),
                Modifier.clip(Radius.button).clickable(role = Role.Button, onClick = onSetup),
                trailing = stringResource(R.string.settings_grants_on, required.size - missing.size, required.size),
            )
            required.forEachIndexed { index, grant ->
                val on = grant !in missing
                PaperRow(
                    stringResource(grant.title()),
                    icon = grant.icon(),
                    divided = index > 0,
                    onClick = onSetup,
                ) {
                    if (on) {
                        Label(stringResource(R.string.settings_on), LinkTheme.type.titleSmall, colors.success)
                    } else {
                        Label(stringResource(R.string.settings_turn_on), LinkTheme.type.titleSmall, colors.accentText)
                    }
                }
            }

            PaperHeading(stringResource(R.string.settings_appearance))
            SlidingPillControl(
                listOf(stringResource(R.string.theme_system), stringResource(R.string.theme_light), stringResource(R.string.theme_dark)),
                theme.ordinal,
                { setAppTheme(context, AppTheme.entries[it]) },
                Modifier.padding(top = Space.s8),
            )

            PaperHeading(stringResource(R.string.settings_diagnostics))
            PaperRow(
                stringResource(R.string.settings_log),
                subtitle = stringResource(R.string.settings_log_hint),
                icon = LinkIcons.Transfers,
                divided = false,
                onClick = { scope.launch { shareLog(context) } },
            ) { Label(stringResource(R.string.settings_export), LinkTheme.type.titleSmall, colors.accentText) }

            Label(
                stringResource(R.string.settings_footer, stringResource(R.string.app_name), version, Build.MODEL),
                LinkTheme.type.bodySmall,
                colors.textTertiary,
                Modifier.padding(top = Space.s28),
            )
            Spacer(Modifier.height(Space.s32))
        }
    }
}

/** Accent with a soft ring while connected, as on Home's line; tertiary while away. */
@Composable
private fun StatusDot(connected: Boolean) {
    val colors = LinkTheme.colors
    Box(
        Modifier
            .size(17.dp)
            .clip(Radius.pill)
            .background(if (connected) colors.accent.copy(alpha = 0.2f) else colors.surfacePrimary),
        contentAlignment = Alignment.Center,
    ) { Box(Modifier.size(9.dp).clip(Radius.pill).background(if (connected) colors.accent else colors.textTertiary)) }
}

/** What the live notification looks like while Stay connected is on. */
@Composable
private fun LiveNotificationPreview(desktop: String?) {
    val colors = LinkTheme.colors
    Row(
        Modifier
            .padding(start = 36.dp, top = Space.s4, bottom = Space.s4)
            .fillMaxWidth()
            .clip(Radius.list)
            .background(colors.surfaceElevated)
            .border(1.dp, colors.borderSecondary, Radius.list)
            .padding(Space.s12),
        horizontalArrangement = Arrangement.spacedBy(Space.s10),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(18.dp).clip(Radius.small).background(colors.accent))
        Column {
            Label(
                if (desktop !=
                    null
                ) {
                    stringResource(R.string.settings_preview_title, desktop)
                } else {
                    stringResource(R.string.settings_preview_idle)
                },
                LinkTheme.type.titleSmall,
            )
            Label(stringResource(R.string.settings_preview_body), LinkTheme.type.bodySmall, colors.textSecondary)
        }
    }
}

private fun Grant.title(): Int =
    when (this) {
        Grant.NotificationAccess -> R.string.settings_grant_notifications
        Grant.DoNotDisturb -> R.string.settings_grant_dnd
        Grant.PostNotifications -> R.string.settings_grant_post
        Grant.Battery -> R.string.settings_grant_battery
        Grant.Calls -> R.string.settings_grant_calls
        Grant.Bluetooth -> R.string.settings_grant_bluetooth
        Grant.Texts -> R.string.onboarding_texts_title
        Grant.AutoClipboard -> R.string.clipboard_auto_row
    }

private fun Grant.icon() =
    when (this) {
        Grant.NotificationAccess, Grant.PostNotifications -> Icons.Filled.Notifications
        Grant.DoNotDisturb -> LinkIcons.Dnd
        Grant.Battery -> LinkIcons.Charging
        Grant.Calls, Grant.Texts -> LinkIcons.Contacts
        Grant.Bluetooth -> LinkIcons.Transfers
        Grant.AutoClipboard -> LinkIcons.Clipboard
    }

/** The last 500 lines this app logged, through the share sheet; an app may read its own log without READ_LOGS. */
private suspend fun shareLog(context: Context) {
    val log =
        withContext(Dispatchers.IO) {
            runCatching {
                val logcat =
                    ProcessBuilder("logcat", "-d", "-v", "time", "-t", "$LOG_LINES", "--pid", "${Process.myPid()}")
                        .redirectErrorStream(true)
                        .start()
                try {
                    logcat.inputStream.bufferedReader().use { it.readText() }
                } finally {
                    logcat.destroy()
                }
            }.getOrElse { context.getString(R.string.settings_log_failed, it.message.orEmpty()) }
                // A share intent travels through Binder, which fails past about 1 MB.
                .takeLast(LOG_CHARS)
        }
    val send =
        Intent(Intent.ACTION_SEND)
            .setType("text/plain")
            .putExtra(Intent.EXTRA_SUBJECT, context.getString(R.string.settings_log_subject))
            .putExtra(Intent.EXTRA_TEXT, log)
    context.startActivity(Intent.createChooser(send, context.getString(R.string.settings_log)))
}

private const val LOG_LINES = 500
private const val LOG_CHARS = 200_000

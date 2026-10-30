package org.andromeda.link.devices

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import org.andromeda.link.R
import org.andromeda.link.clipboard.LogAccess
import org.andromeda.link.clipboard.LogAccess.Method
import org.andromeda.link.ui.components.Label
import org.andromeda.link.ui.components.PaperRow
import org.andromeda.link.ui.components.PillButton
import org.andromeda.link.ui.components.PillKind
import org.andromeda.link.ui.theme.LinkColors
import org.andromeda.link.ui.theme.LinkTheme
import org.andromeda.link.ui.theme.Motion
import org.andromeda.link.ui.theme.Radius
import org.andromeda.link.ui.theme.Space

/** What each method needs, read when Settings resumes or the pick changes. */
data class LogMethods(
    val inUse: Method?,
    val shizuku: Boolean,
    val shizukuAsks: Boolean,
    val root: Boolean,
    val su: Boolean,
    val helper: Boolean,
    val readLogs: Boolean,
) {
    fun ready(method: Method): Boolean =
        when (method) {
            Method.Automatic -> inUse != null
            Method.Shizuku -> shizuku
            Method.Root -> root
            Method.Helper -> helper
            Method.Android -> readLogs
        }
}

/** Settings > Advanced (DESIGN.md, direction A): one radio row per method, and the setup step of a pick not ready. */
@Suppress("ktlint:standard:function-naming")
@Composable
fun BoxScope.LogAccessSheet(
    open: Boolean,
    picked: Method,
    methods: LogMethods,
    onPick: (Method) -> Unit,
    onClipboardSetup: () -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    BackHandler(open, onClose)
    val colors = LinkTheme.colors
    AnimatedVisibility(open, enter = fadeIn(tween(Motion.FAST)), exit = fadeOut(tween(Motion.FADE_OUT))) {
        Box(
            Modifier
                .fillMaxSize()
                .background(colors.surfaceOverlay)
                .clickable(remember { MutableInteractionSource() }, indication = null, onClick = onClose),
        )
    }
    AnimatedVisibility(
        open,
        modifier = modifier.align(Alignment.BottomCenter),
        enter = slideInVertically(tween(Motion.NORMAL, easing = Motion.smoothEnter)) { it },
        exit = slideOutVertically(tween(Motion.FAST, easing = Motion.exit)) { it },
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp))
                .background(if (colors.dark) colors.surfaceSecondary else colors.surfacePrimary)
                .clickable(remember { MutableInteractionSource() }, indication = null) {}
                .navigationBarsPadding()
                .padding(start = Space.page, end = Space.page, bottom = Space.s16),
        ) {
            Box(
                Modifier
                    .padding(top = Space.s8)
                    .align(Alignment.CenterHorizontally)
                    .size(36.dp, 4.dp)
                    .clip(Radius.pill)
                    .background(colors.borderPrimary),
            )
            Label(
                stringResource(R.string.log_method),
                LinkTheme.type.headlineMedium,
                modifier = Modifier.padding(top = Space.s16, bottom = Space.s8),
            )
            Method.entries.forEachIndexed { index, method ->
                val selected = picked == method
                PaperRow(
                    stringResource(method.title()),
                    Modifier.selectable(selected, role = Role.RadioButton) { onPick(method) },
                    subtitle =
                        when (method) {
                            Method.Automatic ->
                                methods.inUse?.let { stringResource(R.string.log_method_found, stringResource(it.title())) }
                                    ?: stringResource(R.string.log_method_none)
                            Method.Helper -> stringResource(R.string.log_method_helper_hint)
                            Method.Android -> stringResource(R.string.log_method_android_hint)
                            else -> null
                        },
                    leading = {
                        Box(
                            Modifier
                                .size(20.dp)
                                .clip(Radius.pill)
                                .border(2.dp, if (selected) colors.accent else colors.borderPrimary, Radius.pill),
                            contentAlignment = Alignment.Center,
                        ) { if (selected) Box(Modifier.size(10.dp).clip(Radius.pill).background(colors.accent)) }
                    },
                    divided = index > 0,
                ) {
                    val state = stateWord(method, methods, colors)
                    if (state != null) Label(stringResource(state.first), LinkTheme.type.titleSmall, state.second)
                }
                if (selected && !methods.ready(method)) {
                    Column(
                        Modifier.padding(start = 36.dp, bottom = Space.s12),
                        verticalArrangement = Arrangement.spacedBy(Space.s8),
                    ) {
                        Label(stringResource(setupStep(method, methods)), LinkTheme.type.bodyMedium, colors.textSecondary)
                        if (method == Method.Shizuku && methods.shizukuAsks) {
                            PillButton(stringResource(R.string.onboarding_clipboard_shizuku), LogAccess::askShizuku, kind = PillKind.Tonal)
                        } else if (method == Method.Automatic || method == Method.Android) {
                            PillButton(stringResource(R.string.log_method_show_command), onClipboardSetup, kind = PillKind.Tonal)
                        }
                    }
                }
            }
        }
    }
}

private fun stateWord(
    method: Method,
    methods: LogMethods,
    colors: LinkColors,
): Pair<Int, Color>? =
    when (method) {
        Method.Automatic -> null
        Method.Shizuku ->
            if (methods.shizuku) R.string.log_method_running to colors.success else R.string.settings_set_up to colors.warning
        Method.Root ->
            when {
                methods.root -> R.string.log_method_granted to colors.success
                methods.su -> R.string.settings_set_up to colors.warning
                else -> R.string.log_method_no_su to colors.textTertiary
            }
        Method.Helper ->
            if (methods.helper) R.string.log_method_running to colors.success else R.string.log_method_off to colors.textTertiary
        Method.Android -> if (methods.readLogs) null else R.string.settings_set_up to colors.warning
    }

private fun setupStep(
    method: Method,
    methods: LogMethods,
): Int =
    when (method) {
        Method.Automatic, Method.Android -> R.string.log_method_android_step
        Method.Shizuku -> if (methods.shizukuAsks) R.string.log_method_shizuku_allow_step else R.string.log_method_shizuku_step
        Method.Root -> if (methods.su) R.string.log_method_root_step else R.string.log_method_no_su_step
        Method.Helper -> R.string.log_method_helper_step
    }

fun Method.title(): Int =
    when (this) {
        Method.Automatic -> R.string.log_method_automatic
        Method.Shizuku -> R.string.log_method_shizuku
        Method.Root -> R.string.log_method_root
        Method.Helper -> R.string.log_method_helper
        Method.Android -> R.string.log_method_android
    }

fun logMethods(context: Context): LogMethods =
    LogMethods(
        LogAccess.inUse(context),
        LogAccess.shizukuReady(),
        LogAccess.shizukuAsks(),
        LogAccess.rootGranted(),
        LogAccess.suFound(),
        LogAccess.helperRunning(),
        LogAccess.readLogs(context),
    )

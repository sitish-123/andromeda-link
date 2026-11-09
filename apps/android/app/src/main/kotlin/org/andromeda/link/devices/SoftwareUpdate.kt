package org.andromeda.link.devices

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import org.andromeda.link.R
import org.andromeda.link.ui.components.Label
import org.andromeda.link.ui.components.LinkIcons
import org.andromeda.link.ui.components.PaperHeading
import org.andromeda.link.ui.components.PaperRow
import org.andromeda.link.ui.theme.LinkTheme
import org.andromeda.link.update.AppUpdate

/** Settings' update row: checks the newest GitHub release once per app run, then downloads and installs it in place. */
@Suppress("ktlint:standard:function-naming")
@Composable
internal fun SoftwareUpdate() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val update by AppUpdate.state.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { if (update == AppUpdate.State.Idle) AppUpdate.check(context) }
    val check = { scope.launch { AppUpdate.check(context) } }
    val subtitle: String
    var action: Int? = null
    var onAction: (() -> Unit)? = null
    when (val u = update) {
        AppUpdate.State.Idle, AppUpdate.State.Current -> {
            subtitle = stringResource(R.string.update_current)
            action = R.string.update_check
            onAction = { check() }
        }
        AppUpdate.State.Checking -> subtitle = stringResource(R.string.update_checking)
        is AppUpdate.State.Available -> {
            subtitle = stringResource(R.string.update_available, u.version)
            action = R.string.update_install
            onAction = { scope.launch { AppUpdate.install(context, u) } }
        }
        is AppUpdate.State.Downloading ->
            subtitle = stringResource(R.string.update_downloading, u.version, ((u.progress ?: 0f) * 100).toInt())
        AppUpdate.State.Installing -> subtitle = stringResource(R.string.update_installing)
        is AppUpdate.State.Failed -> {
            subtitle = stringResource(R.string.update_failed, u.message)
            action = R.string.update_retry
            onAction = { check() }
        }
    }
    PaperHeading(stringResource(R.string.update_heading))
    PaperRow(
        stringResource(R.string.update_row),
        subtitle = subtitle,
        icon = LinkIcons.Transfers,
        divided = false,
        enabled = onAction != null,
        onClick = onAction,
    ) { action?.let { Label(stringResource(it), LinkTheme.type.titleSmall, LinkTheme.colors.accentText) } }
}

package org.andromeda.link.devices

import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import org.andromeda.link.R
import org.andromeda.link.core.domain.Desktop

/** The status of a desktop that is not connected: that it paused this phone, else when it was last seen. */
@Composable
internal fun disconnectedLine(desktop: Desktop): String =
    if (desktop.paused) {
        stringResource(R.string.home_paused_by, desktop.name)
    } else {
        stringResource(R.string.home_last_seen, lastSeen(desktop.lastSeen))
    }

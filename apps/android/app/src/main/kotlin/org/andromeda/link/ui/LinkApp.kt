package org.andromeda.link.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import kotlinx.coroutines.flow.StateFlow
import org.andromeda.link.AppContainer
import org.andromeda.link.R
import org.andromeda.link.control.PresenterScreen
import org.andromeda.link.control.RemoteScreen
import org.andromeda.link.control.RemoteViewModel
import org.andromeda.link.devices.DeviceScreen
import org.andromeda.link.devices.DeviceViewModel
import org.andromeda.link.devices.HomeScreen
import org.andromeda.link.devices.HomeViewModel
import org.andromeda.link.devices.SettingsScreen
import org.andromeda.link.files.PickerViewModel
import org.andromeda.link.media.MediaScreen
import org.andromeda.link.media.MediaViewModel
import org.andromeda.link.notifications.AppFilterScreen
import org.andromeda.link.notifications.AppFilterViewModel
import org.andromeda.link.onboarding.Grant
import org.andromeda.link.onboarding.OnboardingScreen
import org.andromeda.link.onboarding.OnboardingViewModel
import org.andromeda.link.pairing.PairingScreen
import org.andromeda.link.pairing.PairingViewModel
import org.andromeda.link.transfer.ActivityScreen
import org.andromeda.link.transfer.ActivityViewModel
import org.andromeda.link.ui.components.LinkIcons
import org.andromeda.link.ui.components.PillNav
import org.andromeda.link.ui.theme.Motion

private enum class Screen { Home, Activity, Settings, Device, Pairing, Setup, Media, MirrorApps, Remote, Presenter }

private val rootScreens = listOf(Screen.Home, Screen.Activity, Screen.Settings)

/** A handful of screens, so navigation is one piece of state rather than a navigation library. */
@Composable
fun LinkApp(
    container: AppContainer,
    pairingLink: StateFlow<String?>,
    onLinkHandled: () -> Unit,
) {
    var screen by rememberSaveable { mutableStateOf(Screen.Home) }
    var returnScreen by rememberSaveable { mutableStateOf(Screen.Home) }
    var desktopId by rememberSaveable { mutableStateOf<String?>(null) }
    var setupStart by rememberSaveable { mutableStateOf<Grant?>(null) }
    val link by pairingLink.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val home = viewModel { HomeViewModel(container.repository, container.presence, container.mirror, container.clipboard) }
    val pairing = viewModel { PairingViewModel(container.repository) }
    val picker = viewModel { PickerViewModel(container.files) }
    val onboarding = viewModel { OnboardingViewModel(container.application, container.mirror, container.ringer) }
    val activity = viewModel { ActivityViewModel(container.repository, container.clipboard, container.clipHistory) }
    val pick by picker.state.collectAsStateWithLifecycle()
    val setup by onboarding.state.collectAsStateWithLifecycle()
    // Grants change in Settings and over adb, so they are rechecked whenever the app returns.
    LifecycleResumeEffect(Unit) {
        onboarding.refresh()
        onPauseOrDispose {}
    }

    LaunchedEffect(link) {
        link?.let {
            pairing.offerLink(it)
            onLinkHandled()
            returnScreen = if (screen in rootScreens) screen else Screen.Home
            screen = Screen.Pairing
        }
    }
    if (screen != Screen.Home) {
        BackHandler {
            screen =
                when {
                    screen in rootScreens -> Screen.Home
                    screen in listOf(Screen.MirrorApps, Screen.Remote, Screen.Presenter) && desktopId != null -> Screen.Device
                    else -> returnScreen
                }
        }
    }
    val openDesktop: (org.andromeda.link.core.domain.Desktop) -> Unit = {
        desktopId = it.id
        returnScreen = screen
        screen = Screen.Device
    }
    val openPair: () -> Unit = {
        returnScreen = screen
        screen = Screen.Pairing
    }
    val openSetup: () -> Unit = {
        returnScreen = screen
        setupStart = null
        screen = Screen.Setup
    }
    val openClipboardSetup: () -> Unit = {
        returnScreen = screen
        setupStart = Grant.AutoClipboard
        screen = Screen.Setup
    }
    val openNotifications: () -> Unit = {
        returnScreen = screen
        desktopId = null
        if (Grant.NotificationAccess in setup.granted) {
            screen = Screen.MirrorApps
        } else {
            setupStart = null
            screen = Screen.Setup
        }
    }
    Column(Modifier.fillMaxSize()) {
        Box(Modifier.weight(1f)) {
            AnimatedContent(
                targetState = screen,
                transitionSpec = {
                    val forward = targetState.ordinal > initialState.ordinal
                    val enter =
                        slideInHorizontally(tween(Motion.MORPH, easing = Motion.smoothEnter)) {
                            if (forward) it / 6 else -it / 6
                        } + fadeIn(tween(Motion.FAST))
                    enter togetherWith fadeOut(tween(Motion.FADE_OUT)) +
                        slideOutHorizontally(tween(Motion.MORPH, easing = Motion.exit)) { if (forward) -it / 8 else it / 8 }
                },
                label = "screen",
            ) { current ->
                when (current) {
                    Screen.Home ->
                        HomeScreen(
                            viewModel = home,
                            picker = picker,
                            missing = setup.missing,
                            autoClipboard = Grant.AutoClipboard in setup.granted,
                            onPair = openPair,
                            onDesktop = openDesktop,
                            onMedia = {
                                returnScreen = screen
                                screen = Screen.Media
                            },
                            onSetup = openSetup,
                            onClipboardSetup = openClipboardSetup,
                            onActivity = { screen = Screen.Activity },
                            onRemote = {
                                desktopId = it.id
                                screen = Screen.Remote
                            },
                            onPresenter = {
                                desktopId = it.id
                                screen = Screen.Presenter
                            },
                        )
                    Screen.Activity -> ActivityScreen(activity)
                    Screen.Settings ->
                        SettingsScreen(
                            home,
                            Grant.AutoClipboard in setup.granted,
                            setup.missing,
                            openDesktop,
                            openPair,
                            openNotifications,
                            openNotifications,
                            openSetup,
                            openClipboardSetup,
                        )
                    Screen.Device ->
                        desktopId?.let { id ->
                            DeviceScreen(
                                viewModel(key = "device-$id") { DeviceViewModel(container.repository, id) },
                                onBack = { screen = returnScreen },
                                onMirrorApps = { screen = Screen.MirrorApps },
                                onRemote = { screen = Screen.Remote },
                                onPresenter = { screen = Screen.Presenter },
                            )
                        }
                    Screen.Remote, Screen.Presenter ->
                        desktopId?.let { id ->
                            val remote = viewModel(key = "remote-$id") { RemoteViewModel(container.repository, id) }
                            val name =
                                container.repository.desktops.value
                                    .firstOrNull { it.id == id }
                                    ?.name
                                    .orEmpty()
                            if (current == Screen.Remote) {
                                RemoteScreen(remote, name) { screen = Screen.Device }
                            } else {
                                PresenterScreen(remote, name) { screen = Screen.Device }
                            }
                        }
                    Screen.Pairing ->
                        PairingScreen(
                            viewModel = pairing,
                            onBack = { screen = returnScreen },
                            onPaired = { desktop ->
                                container.presence.paired()
                                home.announce(desktop)
                                screen = returnScreen
                            },
                        )
                    Screen.Setup ->
                        OnboardingScreen(
                            onboarding,
                            container.mirror.accessSettings(),
                            start = setupStart,
                            onDone = { screen = returnScreen },
                        )
                    Screen.Media ->
                        MediaScreen(
                            viewModel { MediaViewModel(container.repository) },
                            onBack = { screen = returnScreen },
                        )
                    Screen.MirrorApps ->
                        AppFilterScreen(
                            viewModel { AppFilterViewModel(container.mirror, context.packageManager) },
                            onBack = { screen = if (desktopId != null) Screen.Device else returnScreen },
                        )
                }
            }
        }
        if (screen in rootScreens && !pick.open) {
            PillNav(
                rootScreens.indexOf(screen),
                listOf(
                    stringResource(R.string.home_title),
                    stringResource(R.string.activity_title),
                    stringResource(R.string.settings_title),
                ),
                listOf(Icons.Filled.Home, LinkIcons.Transfers, Icons.Filled.Settings),
            ) { index ->
                screen = rootScreens[index]
                desktopId = null
            }
        }
    }
}

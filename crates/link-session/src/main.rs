//! `andromeda-link-session`: Link's desktop integration outside `andromeda-linkd`'s sandbox: phone notifications
//! through the session's notification server, the clipboard both ways, the phone as touchpad and keyboard, and the
//! output volume, each through a backend the session offers. See "Stage 0" in `docs/link-v2.md`.

#[cfg_attr(target_os = "macos", path = "macos/awake.rs")]
mod awake;
mod clipboard;
mod input;
mod link;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod notify;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
mod screen;
#[cfg_attr(target_os = "macos", path = "macos/system.rs")]
mod system;
#[cfg_attr(target_os = "macos", path = "macos/volume.rs")]
mod volume;

#[cfg(target_os = "linux")]
use anyhow::Context;
#[cfg(target_os = "linux")]
use futures_util::StreamExt;
#[cfg(target_os = "linux")]
use tokio::signal::unix::{SignalKind, signal};

#[cfg(target_os = "linux")]
use crate::notify::{Notifications, Posted};

/// The backend a connect attempt found, if any; a failure is logged and skipped.
fn attempt<T>(name: &str, result: anyhow::Result<Option<T>>) -> Option<T> {
    match result {
        Ok(Some(backend)) => Some(backend),
        Ok(None) => {
            log::debug!("no {name} in this session");
            None
        }
        Err(error) => {
            log::warn!("{name}: {error:#}");
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn on_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("WAYLAND_SOCKET").is_some()
}

async fn next_clipboard(clipboard: &mut Option<clipboard::Backend>) -> anyhow::Result<clipboard::Event> {
    match clipboard {
        Some(clipboard) => clipboard.next().await,
        None => std::future::pending().await,
    }
}

async fn pump_input(input: &mut Option<input::Backend>) -> anyhow::Result<()> {
    match input {
        Some(input) => input.pump().await,
        None => std::future::pending().await,
    }
}

async fn volume_changed(volume: &mut Option<volume::Volume>) -> anyhow::Result<()> {
    match volume {
        Some(volume) => volume.changed().await,
        None => std::future::pending().await,
    }
}

/// Runs a phone's command; the daemon sends it only from devices holding the system grant. The error is for the user.
async fn system_command(
    volume: Option<&volume::Volume>,
    system: &mut system::System,
    command: &str,
    value: u32,
) -> Result<(), String> {
    let volume = || volume.ok_or_else(|| "No volume control on this desktop".to_owned());
    let failed = |error: anyhow::Error| format!("Volume failed: {error:#}");
    match command {
        "set-volume" => volume()?.set_volume(u8::try_from(value.min(100)).unwrap_or(100)).await.map_err(failed),
        "set-mute" => volume()?.set_muted(value != 0).await.map_err(failed),
        _ => system.run(command, value).await,
    }
}

/// What this session can do, as `org.andromeda.Link1.SetSessionCapabilities` takes it.
#[cfg(target_os = "linux")]
fn capabilities(optional: [(bool, &'static str); 4], system: &system::System) -> Vec<&'static str> {
    std::iter::once("notifications")
        .chain(optional.into_iter().filter(|(on, _)| *on).map(|(_, name)| name))
        .chain(system.capabilities())
        .collect()
}

async fn report(link: &link::Link, capabilities: &[&str], volume: Option<&volume::Volume>, system: &system::System) {
    link.report_capabilities(capabilities).await;
    report_status(link, volume, system).await;
}

/// Reports the desktop's state to the daemon, which sends phones a change.
async fn report_status(link: &link::Link, volume: Option<&volume::Volume>, system: &system::System) {
    let volume = match volume {
        Some(volume) => volume.read().await.inspect_err(|error| log::warn!("reading the volume: {error:#}")).ok(),
        None => None,
    };
    link.report_desktop_status(volume, &system.read().await).await;
}

/// Gives a new selection to the daemon, which sends it to phones now or on connect, or serves a paste of `remote`,
/// the phone clip the selection holds as (device id, clip id). A phone clip's own selection never comes back here.
fn on_clipboard(link: &link::Link, event: clipboard::Event, remote: Option<(String, u64)>) {
    match event {
        clipboard::Event::Local { types } => {
            // In a task: a large offer must not hold up a paste the loop has to serve meanwhile.
            let link = link.clone();
            tokio::spawn(async move { link.offer_clipboard(types).await });
        }
        clipboard::Event::Paste { mime, sink, done } => {
            let link = link.clone();
            tokio::spawn(async move {
                let pulled = match remote {
                    Some((device, id)) => link.pull_clipboard(&device, id, &mime, sink).await,
                    None => false,
                };
                if let Some(done) = done {
                    let _ = done.send(pulled);
                }
            });
        }
    }
}

/// The native backends, then on Wayland the portal for what they lack. The portal's first session asks the user.
#[cfg(target_os = "linux")]
async fn backends(bus: &zbus::Connection) -> (Option<clipboard::Backend>, Option<input::Backend>) {
    let mut clipboard = clipboard::Backend::native();
    let mut input = input::Backend::native();
    if on_wayland()
        && (clipboard.is_none() || input.is_none())
        && let Some(session) = attempt("RemoteDesktop portal", portal::Session::start(bus).await)
    {
        if clipboard.is_none() {
            clipboard = clipboard::Backend::portal(&session).await;
        }
        if input.is_none() {
            input = input::Backend::portal(&session).await;
        }
    }
    match &clipboard {
        Some(backend) => log::info!("clipboard through {}", backend.name()),
        None => log::warn!("no clipboard backend"),
    }
    match &input {
        Some(backend) => log::info!("input through {}", backend.name()),
        None => log::warn!("no input backend"),
    }
    (clipboard, input)
}

#[cfg(target_os = "macos")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,zbus=warn,tracing=warn"))
        .format_timestamp(None)
        .init();
    macos::run().await
}

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "current_thread")]
#[expect(clippy::too_many_lines, reason = "one select arm per event source")]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,zbus=warn,tracing=warn"))
        .format_timestamp(None)
        .init();
    let bus = zbus::Connection::session().await?;
    let link = link::Link::connect(&bus).await?;
    let mut notifications = Notifications::connect(&bus, link.clone()).await?;
    // Subscribed before the backends start, so signals sent while a portal dialog waits are queued, not lost.
    let mut posted = link.proxy.receive_notification_posted().await?;
    let mut removed = link.proxy.receive_notification_removed().await?;
    let mut invoked = notifications.proxy.receive_signal("ActionInvoked").await?;
    let mut replied = notifications.proxy.receive_signal("NotificationReplied").await?;
    let mut closed = notifications.proxy.receive_signal("NotificationClosed").await?;
    let mut clip_offered = link.proxy.receive_clipboard_offered().await?;
    let mut remote_input = link.proxy.receive_remote_input().await?;
    let mut remote_requested = link.proxy.receive_remote_control_requested().await?;
    let mut remote_stopped = link.proxy.receive_remote_control_stopped().await?;
    let mut system_commands = link.proxy.receive_system_command().await?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut daemon_owner = link.proxy.inner().receive_owner_changed().await?;
    let (mut lock_changed, mut show_locked_changed, mut access_off) = notifications.watch(&bus).await?;
    let mut keep_awake = link.proxy.receive_keep_awake_changed().await;
    let mut awake = awake::Awake::connect(&bus).await?;
    awake.set(link.proxy.keep_awake().await.unwrap_or(false)).await;
    let (mut clipboard, mut input) = backends(&bus).await;
    let mut volume = attempt("WirePlumber volume", volume::Volume::connect().await);
    let mut system = system::System::connect(&bus).await;
    // Remote control needs input too: the phone without it would only watch.
    let screen = input.is_some() && screen::available(&bus).await;
    let (screen_requests, screen_target) = screen::spawn(bus.clone(), link.clone());
    let optional = [
        (clipboard.is_some(), "clipboard"),
        (input.is_some(), "input"),
        (volume.is_some(), "volume"),
        (screen, "screen"),
    ];
    let capabilities = capabilities(optional, &system);
    log::info!("can: {}", capabilities.join(", "));
    report(&link, &capabilities, volume.as_ref(), &system).await;
    // The phone clip the selection serves, as (device id, clip id).
    let mut remote: Option<(String, u64)> = None;
    log::info!("running");
    loop {
        tokio::select! {
            Some(signal) = posted.next() => {
                if let Ok(args) = signal.args() {
                    notifications.posted(Posted::from(&args)).await;
                }
            }
            Some(signal) = removed.next() => {
                if let Ok(args) = signal.args() {
                    notifications.removed(args.device_id, args.id).await;
                }
            }
            Some(message) = invoked.next() => {
                if let Ok((shown, key)) = message.body().deserialize::<(u32, String)>() {
                    notifications.action_invoked(shown, &key).await;
                }
            }
            Some(message) = replied.next() => {
                if let Ok((shown, text)) = message.body().deserialize::<(u32, String)>() {
                    notifications.replied(shown, text).await;
                }
            }
            Some(message) = closed.next() => {
                if let Ok((shown, reason)) = message.body().deserialize::<(u32, u32)>() {
                    notifications.closed(shown, reason).await;
                }
            }
            Some(signal) = clip_offered.next() => {
                // The daemon forwards only offers from devices holding the clipboard grant.
                let Ok(args) = signal.args() else { continue };
                let (device, id) = (args.device_id.to_owned(), args.id);
                let mimes: Vec<String> = args.mimes.iter().map(|&mime| mime.to_owned()).collect();
                if let Some(backend) = clipboard.as_mut() {
                    match backend.offer_remote(&mimes).await {
                        Ok(()) => remote = Some((device, id)),
                        Err(error) => log::warn!("taking the selection for {device}'s clip: {error}"),
                    }
                }
            }
            event = next_clipboard(&mut clipboard) => match event {
                Ok(event) => on_clipboard(&link, event, remote.clone()),
                // The unit restarts the service, which connects the backends again.
                Err(error) => return Err(error.context("the clipboard backend stopped")),
            },
            Some(signal) = remote_input.next() => {
                // The daemon sends it only from devices holding the input grant.
                let Ok(args) = signal.args() else { continue };
                let Some(backend) = input.as_mut() else { continue };
                let action = args.action;
                if action == "point" {
                    match screen_target.point(args.dx, args.dy) {
                        Some(point) => backend.run(input::Action::Point(point)).context("the input backend stopped")?,
                        None => log::debug!("a point outside a shared monitor ignored"),
                    }
                    continue;
                }
                match input::Action::parse(action, args.dx, args.dy, args.detail.to_owned()) {
                    Some(action) => backend.run(action).context("the input backend stopped")?,
                    None => log::debug!("remote input {action} ignored"),
                }
            }
            result = pump_input(&mut input) => result.context("the input backend stopped")?,
            Some(signal) = remote_requested.next(), if screen => {
                let Ok(args) = signal.args() else { continue };
                let source = u32::try_from(args.source).ok();
                drop(screen_requests.send(screen::Request::Show { device: args.device_id.to_owned(), source }));
            }
            Some(signal) = remote_stopped.next() => {
                let Ok(args) = signal.args() else { continue };
                drop(screen_requests.send(screen::Request::Stop { device: args.device_id.to_owned() }));
            }
            Some(signal) = system_commands.next() => {
                let Ok(args) = signal.args() else { continue };
                let result = system_command(volume.as_ref(), &mut system, args.command, args.value).await;
                if let Err(error) = &result {
                    log::warn!("{} {}: {error}", args.command, args.value);
                }
                link.system_result(args.device_id, args.serial, result.err().as_deref()).await;
                // Without a watcher, nothing else tells the phone its command took.
                report_status(&link, volume.as_ref(), &system).await;
            }
            result = volume_changed(&mut volume) => match result {
                Ok(()) => report_status(&link, volume.as_ref(), &system).await,
                Err(error) => log::warn!("watching the volume: {error:#}"),
            },
            () = system.changed() => report_status(&link, volume.as_ref(), &system).await,
            Some(message) = lock_changed.next() => notifications.set_locked(message.body().deserialize().ok(), None).await,
            Some(change) = show_locked_changed.next() => notifications.set_locked(None, change.get().await.ok()).await,
            Some(change) = access_off.next() => notifications.set_access_off(change.get().await.unwrap_or_default()).await,
            Some(change) = keep_awake.next() => awake.set(change.get().await.unwrap_or(false)).await,
            // A restarted daemon starts with no capabilities and no status; a stopped one keeps nothing awake.
            Some(owner) = daemon_owner.next() => match owner {
                Some(_) => report(&link, &capabilities, volume.as_ref(), &system).await,
                None => awake.set(false).await,
            },
            _ = terminate.recv() => return Ok(()),
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

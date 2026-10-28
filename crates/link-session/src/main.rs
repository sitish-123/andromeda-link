//! `andromeda-link-session`: Link's desktop integration outside `andromeda-linkd`'s sandbox: phone notifications
//! through the session's notification server, the clipboard both ways, the phone as touchpad and keyboard, and the
//! output volume, each through a backend the session offers. See "Stage 0" in `docs/link-v2.md`.

mod clipboard;
mod input;
mod link;
mod notify;
mod portal;
mod volume;

use anyhow::Context;
use futures_util::StreamExt;
use tokio::signal::unix::{SignalKind, signal};

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

/// Runs a phone's `set-volume` or `set-mute`; the daemon sends it only from devices holding the system grant.
async fn system_command(volume: &volume::Volume, command: &str, value: u32) -> anyhow::Result<()> {
    match command {
        "set-volume" => volume.set_volume(u8::try_from(value.min(100))?).await,
        "set-mute" => volume.set_muted(value != 0).await,
        other => {
            log::debug!("system command {other} ignored");
            Ok(())
        }
    }
}

/// What this session can do, as `org.andromeda.Link1.SetSessionCapabilities` takes it.
fn capabilities(clipboard: bool, input: bool, volume: bool) -> Vec<&'static str> {
    let optional = [(clipboard, "clipboard"), (input, "input"), (volume, "volume")];
    std::iter::once("notifications").chain(optional.into_iter().filter(|(on, _)| *on).map(|(_, name)| name)).collect()
}

async fn report(link: &link::Link, capabilities: &[&str], volume: Option<&volume::Volume>) {
    link.report_capabilities(capabilities).await;
    if let Some(volume) = volume {
        report_volume(link, volume).await;
    }
}

/// Reports the output's volume and mute to the daemon, which sends phones a change.
async fn report_volume(link: &link::Link, volume: &volume::Volume) {
    match volume.read().await {
        Ok((percent, muted)) => link.report_desktop_status(percent, muted).await,
        Err(error) => log::warn!("reading the volume: {error:#}"),
    }
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

#[tokio::main(flavor = "current_thread")]
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
    let mut system = link.proxy.receive_system_command().await?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut daemon_owner = link.proxy.inner().receive_owner_changed().await?;
    let (mut lock_changed, mut show_locked_changed, mut access_off) = notifications.watch(&bus).await?;
    let (mut clipboard, mut input) = backends(&bus).await;
    let mut volume = attempt("WirePlumber volume", volume::Volume::connect().await);
    let capabilities = capabilities(clipboard.is_some(), input.is_some(), volume.is_some());
    log::info!("can: {}", capabilities.join(", "));
    report(&link, &capabilities, volume.as_ref()).await;
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
                match input::Action::parse(action, args.dx, args.dy, args.detail.to_owned()) {
                    Some(action) => backend.run(action).context("the input backend stopped")?,
                    None => log::debug!("remote input {action} ignored"),
                }
            }
            result = pump_input(&mut input) => result.context("the input backend stopped")?,
            Some(signal) = system.next() => {
                let (Ok(args), Some(backend)) = (signal.args(), volume.as_ref()) else { continue };
                match system_command(backend, args.command, args.value).await {
                    // Without a watcher, nothing else tells the phone its command took.
                    Ok(()) => report_volume(&link, backend).await,
                    Err(error) => log::warn!("{} {}: {error:#}", args.command, args.value),
                }
            }
            result = volume_changed(&mut volume) => match (result, volume.as_ref()) {
                (Ok(()), Some(backend)) => report_volume(&link, backend).await,
                (Err(error), _) => log::warn!("watching the volume: {error:#}"),
                (Ok(()), None) => {}
            },
            Some(message) = lock_changed.next() => notifications.set_locked(message.body().deserialize().ok(), None).await,
            Some(change) = show_locked_changed.next() => notifications.set_locked(None, change.get().await.ok()).await,
            Some(change) = access_off.next() => notifications.set_access_off(change.get().await.unwrap_or_default()).await,
            // A restarted daemon starts with no capabilities and no status.
            Some(Some(_)) = daemon_owner.next() => report(&link, &capabilities, volume.as_ref()).await,
            _ = terminate.recv() => return Ok(()),
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

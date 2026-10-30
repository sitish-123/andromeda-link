//! `link-session` on macOS: the daemon's signals served with the macOS backends (`NSPasteboard`, `CGEvent`,
//! `UNUserNotificationCenter`, `CoreAudio`, `IOKit`). macOS gives no public API for desktop media control since 15.4, and
//! the phone's remote control needs a screen capture backend, so neither is offered.

pub mod notify;

use anyhow::Context;
use futures_util::StreamExt;
use tokio::signal::unix::{SignalKind, signal};

use crate::{
    attempt, awake, clipboard, input, link, next_clipboard, on_clipboard, pump_input, report, report_status, system,
    system_command, volume, volume_changed,
};

pub async fn run() -> anyhow::Result<()> {
    // There is no session bus: the daemon's socket, which closes when the daemon stops; launchd then restarts this.
    let daemon = link_api::connect().await.context("connecting to andromeda-linkd")?;
    let link = link::Link::connect(&daemon).await?;
    let mut posted = link.proxy.receive_notification_posted().await?;
    let mut removed = link.proxy.receive_notification_removed().await?;
    let mut clip_offered = link.proxy.receive_clipboard_offered().await?;
    let mut remote_input = link.proxy.receive_remote_input().await?;
    let mut system_commands = link.proxy.receive_system_command().await?;
    let mut terminate = signal(SignalKind::terminate())?;
    let mut keep_awake = link.proxy.receive_keep_awake_changed().await;
    let mut awake = awake::Awake::default();
    awake.set(link.proxy.keep_awake().await.unwrap_or(false));
    let mut notifications = notify::Notifications::connect(link.clone());
    let mut clipboard = clipboard::Backend::native();
    let mut input = input::Backend::native();
    for (what, backend) in [
        ("clipboard", clipboard.as_ref().map(clipboard::Backend::name)),
        ("input", input.as_ref().map(input::Backend::name)),
    ] {
        match backend {
            Some(name) => log::info!("{what} through {name}"),
            None => log::warn!("no {what} backend"),
        }
    }
    let mut volume = attempt("CoreAudio volume", volume::Volume::connect().await);
    let mut system = system::System::connect();
    let optional = [
        (notifications.is_some(), "notifications"),
        (clipboard.is_some(), "clipboard"),
        (input.is_some(), "input"),
        (volume.is_some(), "volume"),
    ];
    let capabilities: Vec<&str> =
        optional.into_iter().filter_map(|(on, name)| on.then_some(name)).chain(system.capabilities()).collect();
    log::info!("can: {}", capabilities.join(", "));
    report(&link, &capabilities, volume.as_ref(), &system).await;
    // The phone clip the pasteboard holds, as (device id, clip id).
    let mut remote: Option<(String, u64)> = None;
    log::info!("running");
    loop {
        tokio::select! {
            Some(signal) = posted.next() => {
                if let (Ok(args), Some(notifications)) = (signal.args(), notifications.as_mut()) {
                    notifications.posted(&args);
                }
            }
            Some(signal) = removed.next() => {
                if let (Ok(args), Some(notifications)) = (signal.args(), notifications.as_ref()) {
                    notifications.removed(args.device_id, args.id);
                }
            }
            Some(signal) = clip_offered.next() => {
                let Ok(args) = signal.args() else { continue };
                let mimes: Vec<String> = args.mimes.iter().map(|&mime| mime.to_owned()).collect();
                if let Some(backend) = clipboard.as_mut() {
                    match backend.offer_remote(&mimes).await {
                        Ok(()) => remote = Some((args.device_id.to_owned(), args.id)),
                        Err(error) => log::warn!("taking the pasteboard for {}'s clip: {error}", args.device_id),
                    }
                }
            }
            event = next_clipboard(&mut clipboard) => match event {
                Ok(event) => on_clipboard(&link, event, remote.clone()),
                Err(error) => return Err(error.context("the clipboard backend stopped")),
            },
            Some(signal) = remote_input.next() => {
                let Ok(args) = signal.args() else { continue };
                let Some(backend) = input.as_mut() else { continue };
                match input::Action::parse(args.action, args.dx, args.dy, args.detail.to_owned()) {
                    Some(action) => backend.run(action).context("the input backend stopped")?,
                    None => log::debug!("remote input {} ignored", args.action),
                }
            }
            result = pump_input(&mut input) => result.context("the input backend stopped")?,
            Some(signal) = system_commands.next() => {
                let Ok(args) = signal.args() else { continue };
                let result = system_command(volume.as_ref(), &mut system, args.command, args.value).await;
                if let Err(error) = &result {
                    log::warn!("{} {}: {error}", args.command, args.value);
                }
                link.system_result(args.device_id, args.serial, result.err().as_deref()).await;
                report_status(&link, volume.as_ref(), &system).await;
            }
            result = volume_changed(&mut volume) => match result {
                Ok(()) => report_status(&link, volume.as_ref(), &system).await,
                Err(error) => log::warn!("watching the volume: {error:#}"),
            },
            () = system.changed() => report_status(&link, volume.as_ref(), &system).await,
            Some(change) = keep_awake.next() => awake.set(change.get().await.unwrap_or(false)),
            () = daemon.closed() => anyhow::bail!("andromeda-linkd closed the connection"),
            _ = terminate.recv() => return Ok(()),
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

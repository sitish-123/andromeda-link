//! `link-session` on macOS: the daemon's signals served with the macOS backends (`NSPasteboard`, `CGEvent`,
//! `UNUserNotificationCenter`, `CoreAudio`, `IOKit`). macOS gives no public API for desktop media control since 15.4, and
//! the phone's remote control needs a screen capture backend, so neither is offered.

pub mod notify;
mod screen;

use anyhow::Context;
use futures_util::StreamExt;
use tokio::signal::unix::{SignalKind, signal};

use crate::{
    attempt, awake, clipboard, input, link, next_clipboard, on_clipboard, pump_input, report, report_status, ring,
    system, system_command, volume, volume_changed,
};

#[expect(clippy::too_many_lines, reason = "one select arm per event source")]
pub async fn run() -> anyhow::Result<()> {
    // There is no session bus: the daemon's socket, which closes when the daemon stops; launchd then restarts this.
    let daemon = link_api::connect().await.context("connecting to andromeda-linkd")?;
    let link = link::Link::connect(&daemon).await?;
    let mut posted = link.proxy.receive_notification_posted().await?;
    let mut removed = link.proxy.receive_notification_removed().await?;
    let mut clip_offered = link.proxy.receive_clipboard_offered().await?;
    let mut remote_input = link.proxy.receive_remote_input().await?;
    let mut media_keys = link.proxy.receive_media_key().await?;
    let mut ring_requests = link.proxy.receive_ring_requested().await?;
    let mut ring = ring::Ring::default();
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
    // Remote control needs input too: the phone without it would only watch.
    let screen = input.is_some() && screen::available();
    let (screen_requests, screen_target) = screen::spawn(link.clone());
    let optional = [
        (notifications.is_some(), "notifications"),
        (clipboard.is_some(), "clipboard"),
        (input.is_some(), "input"),
        (volume.is_some(), "volume"),
        (screen, "screen"),
    ];
    let can: Vec<&str> =
        optional.into_iter().filter_map(|(on, name)| on.then_some(name)).chain(system.capabilities()).collect();
    log::info!("can: {}", can.join(", "));
    report(&link, &can, volume.as_ref(), &system).await;
    let mut remote_requested = link.proxy.receive_remote_control_requested().await?;
    let mut remote_stopped = link.proxy.receive_remote_control_stopped().await?;
    // A process keeps the Accessibility and Screen Recording answers it started with, so a new process checks them;
    // when one was given since, this one exits and launchd starts it again with it.
    let mut permission_check = tokio::time::interval(std::time::Duration::from_secs(2));
    let had = permission_bits(input.is_some(), screen);
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
                Ok(event) => on_clipboard(&link, event, remote.clone(), None),
                Err(error) => return Err(error.context("the clipboard backend stopped")),
            },
            Some(signal) = remote_input.next() => {
                let Ok(args) = signal.args() else { continue };
                let Some(backend) = input.as_mut() else { continue };
                if args.action == "point" {
                    match screen_target.point(args.dx, args.dy) {
                        Some(point) => backend.run(input::Action::Point(point)).context("the input backend stopped")?,
                        None => log::debug!("a point outside a shared display ignored"),
                    }
                    continue;
                }
                match input::Action::parse(args.action, args.dx, args.dy, args.detail.to_owned()) {
                    Some(action) => backend.run(action).context("the input backend stopped")?,
                    None => log::debug!("remote input {} ignored", args.action),
                }
            }
            result = pump_input(&mut input) => result.context("the input backend stopped")?,
            Some(signal) = ring_requests.next() => {
                let Ok(args) = signal.args() else { continue };
                ring.set(&link, args.device_id, args.on).await;
            }
            () = ring.timed_out() => ring.stop(&link).await,
            Some(signal) = media_keys.next() => {
                let Ok(args) = signal.args() else { continue };
                // Posting events needs the Accessibility permission, as input does.
                if input.is_none() {
                    log::warn!("media key {} needs the Accessibility permission", args.key);
                } else if let Err(error) = input::mac::media_key(args.key) {
                    log::warn!("media key {}: {error:#}", args.key);
                }
            }
            _ = permission_check.tick(), if had != ALL_PERMISSIONS => {
                if fresh_permissions().await & !had != 0 {
                    log::info!("a permission was given; restarting to use it");
                    return Ok(());
                }
            }
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

/// Run with this argument, the program exits with [`permissions`] and does nothing else.
pub const PERMISSIONS_FLAG: &str = "--permissions";
const ACCESSIBILITY: i32 = 1;
const SCREEN_RECORDING: i32 = 2;
const ALL_PERMISSIONS: i32 = ACCESSIBILITY | SCREEN_RECORDING;

fn permission_bits(accessibility: bool, screen: bool) -> i32 {
    (i32::from(accessibility) * ACCESSIBILITY) | (i32::from(screen) * SCREEN_RECORDING)
}

/// This process's permissions as bits, asked without a prompt.
pub fn permissions() -> i32 {
    permission_bits(input::mac::trusted(), screen::granted())
}

/// The permissions a new process of this program has; none when it cannot run.
async fn fresh_permissions() -> i32 {
    let Ok(program) = std::env::current_exe() else { return 0 };
    let status = tokio::process::Command::new(program).arg(PERMISSIONS_FLAG).status().await;
    status.ok().and_then(|status| status.code()).unwrap_or(0)
}

//! `andromeda-link-session`: Link's desktop integration outside `andromeda-linkd`'s sandbox: phone notifications
//! through the session's notification server, the clipboard both ways, and the phone as touchpad and keyboard, each
//! through a backend the session offers. See "Stage 0" in `docs/link-v2.md`.

mod clipboard;
mod input;
mod link;
mod notify;
mod portal;

use std::hash::{DefaultHasher, Hash, Hasher};

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

/// Sends a new selection to phones, unless it is the one sent last, or serves a paste of `remote`, the phone clip
/// the selection holds as (device id, clip id).
fn on_clipboard(link: &link::Link, event: clipboard::Event, sent: &mut u64, remote: Option<(String, u64)>) {
    match event {
        clipboard::Event::Local { mimes, data } => {
            let mut hasher = DefaultHasher::new();
            data.hash(&mut hasher);
            let hash = hasher.finish();
            if hash != *sent {
                *sent = hash;
                // In a task: a large offer must not hold up a paste the loop has to serve meanwhile.
                let link = link.clone();
                tokio::spawn(async move {
                    if link.clipboard_wanted().await {
                        link.offer_clipboard(mimes, &data).await;
                    }
                });
            }
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
    let mut posted = link.proxy.receive_signal("NotificationPosted").await?;
    let mut removed = link.proxy.receive_signal("NotificationRemoved").await?;
    let mut invoked = notifications.proxy.receive_signal("ActionInvoked").await?;
    let mut replied = notifications.proxy.receive_signal("NotificationReplied").await?;
    let mut closed = notifications.proxy.receive_signal("NotificationClosed").await?;
    let mut clip_offered = link.proxy.receive_signal("ClipboardOffered").await?;
    let mut remote_input = link.proxy.receive_signal("RemoteInput").await?;
    let mut terminate = signal(SignalKind::terminate())?;
    let (mut clipboard, mut input) = backends(&bus).await;
    // The phone clip the selection serves, as (device id, clip id).
    let mut remote: Option<(String, u64)> = None;
    // The last selection sent, so a re-read of the same one is not sent again.
    let mut sent = 0u64;
    log::info!("running");
    loop {
        tokio::select! {
            Some(message) = posted.next() => {
                type Body = (String, String, String, String, String, Vec<u8>, Vec<(String, String, bool)>);
                let Ok((device, id, app, title, text, icon, actions)) = message.body().deserialize::<Body>() else {
                    continue;
                };
                notifications.posted(Posted { device, id, app, title, text, icon, actions }).await;
            }
            Some(message) = removed.next() => {
                if let Ok((device, id)) = message.body().deserialize::<(String, String)>() {
                    notifications.removed(&device, &id).await;
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
            Some(message) = clip_offered.next() => {
                // The daemon forwards only offers from devices holding the clipboard grant.
                let Ok((device, id, mimes, _)) = message.body().deserialize::<(String, u64, Vec<String>, u64)>() else {
                    continue;
                };
                if let Some(backend) = clipboard.as_mut() {
                    match backend.offer_remote(&mimes).await {
                        Ok(()) => {
                            remote = Some((device, id));
                            // The phone's clip replaced the last copy, so copying that again must send it again.
                            sent = 0;
                        }
                        Err(error) => log::warn!("taking the selection for {device}'s clip: {error}"),
                    }
                }
            }
            event = next_clipboard(&mut clipboard) => match event {
                Ok(event) => on_clipboard(&link, event, &mut sent, remote.clone()),
                // The unit restarts the service, which connects the backends again.
                Err(error) => return Err(error.context("the clipboard backend stopped")),
            },
            Some(message) = remote_input.next() => {
                // The daemon sends it only from devices holding the input grant.
                let Ok((_, action, dx, dy, detail)) = message.body().deserialize::<(String, String, i32, i32, String)>()
                else {
                    continue;
                };
                let Some(backend) = input.as_mut() else { continue };
                match input::Action::parse(&action, dx, dy, detail) {
                    Some(action) => backend.run(action).context("the input backend stopped")?,
                    None => log::debug!("remote input {action} ignored"),
                }
            }
            result = pump_input(&mut input) => result.context("the input backend stopped")?,
            _ = terminate.recv() => return Ok(()),
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
    }
}

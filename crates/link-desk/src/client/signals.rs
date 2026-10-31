//! Converts the service's properties and signals into events.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use tokio::sync::mpsc::UnboundedSender;

use super::{Controls, Device, Event, LoggedCall, Notification, Status, Text};

pub(super) fn send_devices(events: &UnboundedSender<Event>, devices: Vec<(String, String, bool)>) {
    let devices = devices.into_iter().map(|(id, name, connected)| Device { id, name, connected }).collect();
    let _ = events.send(Event::Devices(devices));
}

pub(super) fn send_info(events: &UnboundedSender<Event>, info: HashMap<String, HashMap<String, zbus::OwnedVariant>>) {
    let info = info
        .into_iter()
        .map(|(id, fields)| {
            let seen = fields.get("last_seen").and_then(|value| i64::try_from(value).ok()).unwrap_or(0);
            let transport = fields.get("transport").and_then(|value| String::try_from(value.clone()).ok());
            (id, (seen, transport.unwrap_or_default()))
        })
        .collect();
    let _ = events.send(Event::Info(info));
}

pub(super) fn send_status(events: &UnboundedSender<Event>, status: HashMap<String, (u32, bool, String, i32)>) {
    let status = status
        .into_iter()
        .map(|(id, (battery, charging, network, signal))| (id, Status { battery, charging, network, signal }))
        .collect();
    let _ = events.send(Event::Status(status));
}

pub(super) fn send_controls(events: &UnboundedSender<Event>, controls: HashMap<String, (i32, String, i32, i32)>) {
    let controls = controls
        .into_iter()
        .map(|(id, (media_volume, ringer, dnd, _locked))| (id, Controls { media_volume, ringer, dnd }))
        .collect();
    let _ = events.send(Event::Controls(controls));
}

/// (number, name, date in Unix ms, duration in s, kind), as `Calls` gives each call.
type RawCall = (String, String, u64, u32, String);

pub(super) fn send_calls(events: &UnboundedSender<Event>, calls: HashMap<String, Vec<RawCall>>) {
    let calls = calls
        .into_iter()
        .map(|(id, calls)| {
            let calls = calls
                .into_iter()
                .map(|(number, name, date, duration, kind)| LoggedCall { number, name, date, duration, kind })
                .collect();
            (id, calls)
        })
        .collect();
    let _ = events.send(Event::Calls(calls));
}

/// (id, thread, address, body, date in Unix ms, sent), as `Texts` gives each text.
type RawText = (u64, u64, String, String, u64, bool);

pub(super) fn send_texts(events: &UnboundedSender<Event>, texts: HashMap<String, Vec<RawText>>) {
    let texts = texts
        .into_iter()
        .map(|(id, texts)| {
            let texts = texts
                .into_iter()
                .map(|(_, thread, address, body, date, sent)| Text { thread, address, body, date, sent })
                .collect();
            (id, texts)
        })
        .collect();
    let _ = events.send(Event::Texts(texts));
}

/// The cache folder of the app, created on demand.
pub fn cache(name: &str) -> std::io::Result<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .ok_or_else(|| std::io::Error::other("no HOME"))?;
    let folder = base.join("andromeda-link").join(name);
    std::fs::create_dir_all(&folder)?;
    Ok(folder)
}

/// A file name that is the same for the same key.
pub(super) fn hashed(key: &impl Hash, extension: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    format!("{:016x}.{extension}", hasher.finish())
}

/// Writes a notification icon to the cache once per content, for QML to load by URL.
fn cache_icon(icon: &[u8]) -> Option<PathBuf> {
    if icon.is_empty() {
        return None;
    }
    let path = cache("icons").ok()?.join(hashed(&icon, "png"));
    if path.exists() {
        return Some(path);
    }
    match std::fs::write(&path, icon) {
        Ok(()) => Some(path),
        Err(error) => {
            log::warn!("{}: {error}", path.display());
            None
        }
    }
}

pub(super) fn signal(message: zbus::Message) -> Option<Event> {
    let owned = |text: &str| text.to_owned();
    if let Some(signal) = link_api::NotificationPosted::from_message(message.clone()) {
        let args = signal.args().ok()?;
        let actions = args.actions.iter().map(|&(id, label, reply)| (owned(id), owned(label), reply)).collect();
        return Some(Event::Notification(Notification {
            device: owned(args.device_id),
            id: owned(args.id),
            app: owned(args.app),
            title: owned(args.title),
            text: owned(args.text),
            icon: cache_icon(&args.icon),
            actions,
        }));
    }
    if let Some(signal) = link_api::NotificationRemoved::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::NotificationRemoved { device: owned(args.device_id), id: owned(args.id) });
    }
    if let Some(signal) = link_api::HotspotRequested::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::HotspotAsk { device: owned(args.device_id), ssid: owned(args.ssid) });
    }
    if let Some(signal) = link_api::TransferOffered::from_message(message.clone()) {
        let args = signal.args().ok()?;
        let files = args.files.iter().map(|&(name, size)| (owned(name), size)).collect();
        return Some(Event::Offered { transfer: owned(args.transfer_id), device: owned(args.device_id), files });
    }
    if let Some(signal) = link_api::TransferProgress::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::Progress { transfer: owned(args.transfer_id), bytes: args.bytes, total: args.total });
    }
    if let Some(signal) = link_api::TransferFinished::from_message(message.clone()) {
        let args = signal.args().ok()?;
        let paths = args.paths.iter().map(|&path| owned(path)).collect();
        return Some(Event::Finished { transfer: owned(args.transfer_id), status: owned(args.status), paths });
    }
    if let Some(signal) = link_api::Received::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::Received { device: owned(args.device_id), text: owned(args.text) });
    }
    if let Some(signal) = link_api::TextReceived::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::TextReceived { device: owned(args.device_id), address: owned(args.address) });
    }
    if let Some(signal) = link_api::PairingFinished::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::Paired(owned(args.name)));
    }
    if let Some(signal) = link_api::PairingFailed::from_message(message) {
        let args = signal.args().ok()?;
        return Some(Event::PairingFailed(owned(args.reason)));
    }
    None
}

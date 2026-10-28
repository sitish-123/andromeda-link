//! Phone notifications shown through the session's `org.freedesktop.Notifications` server, with their actions and
//! inline reply.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use zbus::Variant;

use crate::link::Link;

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const ACTION_PREFIX: &str = "phone:";
const INLINE_REPLY: &str = "inline-reply";
/// The Andromeda shell sends the reply text as an `ActionInvoked` key; KDE sends `NotificationReplied`.
const INLINE_REPLY_PREFIX: &str = "inline-reply::";
/// The `NotificationClosed` reason for a user's dismissal.
const DISMISSED: u32 = 2;
const MAX_ICON_SIDE: u32 = 256;
/// Phone notifications on show per phone; older ones fold into one "N more from <phone>".
const MAX_SHOWN: usize = 5;
const SCREENSAVER: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
/// `<device id>/<app>`: the shell's notification center groups by it.
const GROUP_HINT: &str = "x-andromeda-group";
const CAPABILITIES_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub struct Posted {
    pub device: String,
    pub id: String,
    pub app: String,
    pub title: String,
    pub text: String,
    /// PNG, or empty.
    pub icon: Vec<u8>,
    /// `(action id, label, takes a reply)`.
    pub actions: Vec<(String, String, bool)>,
    /// False: replace the shown one without a sound.
    pub alert: bool,
}

impl From<&link_api::NotificationPostedArgs<'_>> for Posted {
    fn from(args: &link_api::NotificationPostedArgs<'_>) -> Self {
        Self {
            device: args.device_id.to_owned(),
            id: args.id.to_owned(),
            app: args.app.to_owned(),
            title: args.title.to_owned(),
            text: args.text.to_owned(),
            icon: args.icon.clone(),
            actions: args.actions.iter().map(|&(id, label, reply)| (id.to_owned(), label.to_owned(), reply)).collect(),
            alert: args.alert,
        }
    }
}

/// The lock's `ActiveChanged` signals, and changes of `NotificationsWhileLocked` and `NotificationsOff`.
pub type Watches = (
    zbus::proxy::SignalStream<'static>,
    zbus::proxy::PropertyStream<'static, bool>,
    zbus::proxy::PropertyStream<'static, Vec<String>>,
);

struct Mirrored {
    device: String,
    id: String,
    reply_action: Option<String>,
    acted: bool,
    icon: Option<PathBuf>,
    /// Order of showing, so the cap folds the oldest.
    seq: u64,
}

pub struct Notifications {
    pub proxy: zbus::Proxy<'static>,
    link: Link,
    inline_reply: bool,
    icons: PathBuf,
    shown: HashMap<u32, Mirrored>,
    next_seq: u64,
    /// Per phone: the ids the cap folded away, and the shown id of the "N more" summary that counts them.
    folded: HashMap<String, (Vec<String>, u32)>,
    /// The desktop is locked, and whether phone notifications show then anyway.
    locked: bool,
    show_locked: bool,
    /// Posts held while locked, and the shown id of the notification that counts them.
    held: Vec<Posted>,
    count: u32,
    /// Phones whose notification access is off, to the shown id of the notice saying so.
    access_off: HashMap<String, u32>,
}

impl Notifications {
    pub async fn connect(bus: &zbus::Connection, link: Link) -> zbus::Result<Self> {
        let proxy = zbus::Proxy::new_owned(bus.clone(), SERVICE, PATH, SERVICE).await?;
        // Without a notification server yet, the clipboard and input still work; a later server is used. The bound
        // keeps a server whose activation hangs (Plasma's outside Plasma) from holding up the start.
        let query = tokio::time::timeout(CAPABILITIES_TIMEOUT, proxy.call::<_, _, Vec<String>>("GetCapabilities", &()));
        let capabilities = match query.await {
            Ok(Ok(capabilities)) => capabilities,
            Ok(Err(error)) => {
                log::warn!("no notification server: {error}");
                Vec::new()
            }
            Err(_) => {
                log::warn!("no notification server: it did not answer in time");
                Vec::new()
            }
        };
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        Ok(Self {
            proxy,
            link,
            inline_reply: capabilities.iter().any(|capability| capability == INLINE_REPLY),
            icons: runtime.join("andromeda-link-session"),
            shown: HashMap::new(),
            next_seq: 0,
            folded: HashMap::new(),
            locked: false,
            show_locked: false,
            held: Vec::new(),
            count: 0,
            access_off: HashMap::new(),
        })
    }

    /// While the desktop is locked and the setting is off, only a count shows.
    pub async fn posted(&mut self, posted: Posted) {
        if self.locked && !self.show_locked {
            self.held.retain(|held| held.device != posted.device || held.id != posted.id);
            self.held.push(posted);
            return self.update_count().await;
        }
        self.show(posted).await;
    }

    /// Reads the lock state, which the shell announces on org.freedesktop.ScreenSaver, the setting, and the phones
    /// with notification access off; returns the streams of their changes.
    pub async fn watch(&mut self, bus: &zbus::Connection) -> zbus::Result<Watches> {
        let screensaver = zbus::Proxy::new_owned(bus.clone(), SCREENSAVER, SCREENSAVER_PATH, SCREENSAVER).await?;
        let lock = screensaver.receive_signal("ActiveChanged").await?;
        let show_locked_changed = self.link.proxy.receive_notifications_while_locked_changed().await;
        let access_off = self.link.proxy.receive_notifications_off_changed().await;
        let locked = screensaver.call::<_, _, bool>("GetActive", &()).await.ok();
        let show_locked = self.link.proxy.notifications_while_locked().await.ok();
        self.set_locked(locked, show_locked).await;
        self.set_access_off(self.link.proxy.notifications_off().await.unwrap_or_default()).await;
        Ok((lock, show_locked_changed, access_off))
    }

    /// At unlock the count closes and the held posts show without a sound; None keeps a value.
    pub async fn set_locked(&mut self, locked: Option<bool>, show_locked: Option<bool>) {
        self.locked = locked.unwrap_or(self.locked);
        self.show_locked = show_locked.unwrap_or(self.show_locked);
        if self.locked && !self.show_locked {
            return;
        }
        for posted in std::mem::take(&mut self.held) {
            self.show(Posted { alert: false, ..posted }).await;
        }
        self.update_count().await;
    }

    /// A notice per phone whose notification access is off, closed once it is on again.
    pub async fn set_access_off(&mut self, off: Vec<String>) {
        let back: Vec<String> = self.access_off.keys().filter(|device| !off.contains(device)).cloned().collect();
        for device in back {
            if let Some(shown) = self.access_off.remove(&device) {
                self.close(shown).await;
            }
        }
        for device in off {
            if self.access_off.contains_key(&device) {
                continue;
            }
            let summary = format!("Notification access is off on {}", self.device_name(&device).await);
            let body = "Turn it on in Link on the phone to see its notifications here.";
            if let Some(shown) = self.notify_own(0, &summary, body, true).await {
                self.access_off.insert(device, shown);
            }
        }
    }

    async fn show(&mut self, posted: Posted) {
        if let Some((ids, _)) = self.folded.get_mut(&posted.device)
            && let Some(at) = ids.iter().position(|id| *id == posted.id)
        {
            ids.remove(at);
            self.update_summary(&posted.device.clone()).await;
        }
        let replaces = self.find(&posted.device, &posted.id).unwrap_or(0);
        let mut actions = Vec::new();
        let mut reply_action = None;
        for (id, label, reply) in &posted.actions {
            if !reply {
                actions.push(format!("{ACTION_PREFIX}{id}"));
                actions.push(label.clone());
            } else if self.inline_reply && reply_action.is_none() {
                // Servers show one reply field per notification, so a second reply action is left out.
                reply_action = Some(id.clone());
                actions.push(INLINE_REPLY.to_owned());
                actions.push(label.clone());
            }
        }
        let icon = self.write_icon(&posted).await;
        let mut hints: HashMap<&str, Variant<'_>> = HashMap::new();
        hints.insert("desktop-entry", Variant::from("org.andromeda.Link"));
        let group = format!("{}/{}", self.device_name(&posted.device).await, posted.app);
        hints.insert(GROUP_HINT, Variant::from(group.as_str()));
        if !posted.alert {
            hints.insert("suppress-sound", Variant::from(true));
        }
        if let Some(path) = &icon {
            hints.insert("image-path", Variant::from(path.to_string_lossy().into_owned()));
        }
        let summary = if posted.title.is_empty() { &posted.app } else { &posted.title };
        let app_icon = if icon.is_some() { "" } else { "phone" };
        let result: zbus::Result<u32> = self
            .proxy
            .call("Notify", &(&posted.app, replaces, app_icon, summary, &posted.text, actions, hints, -1i32))
            .await;
        let Ok(shown) = result.inspect_err(|error| log::warn!("Notify: {error}")) else { return };
        if let Some(old) = self.shown.remove(&replaces).and_then(|old| old.icon)
            && Some(&old) != icon.as_ref()
        {
            let _ = tokio::fs::remove_file(old).await;
        }
        self.next_seq += 1;
        let device = posted.device.clone();
        let mirrored =
            Mirrored { device: posted.device, id: posted.id, reply_action, acted: false, icon, seq: self.next_seq };
        self.shown.insert(shown, mirrored);
        self.fold(&device).await;
    }

    /// Past [`MAX_SHOWN`] of a phone, the oldest close and count in its "N more" summary.
    async fn fold(&mut self, device: &str) {
        let mut on_show: Vec<(u64, u32)> = self
            .shown
            .iter()
            .filter(|(_, mirrored)| mirrored.device == device)
            .map(|(shown, mirrored)| (mirrored.seq, *shown))
            .collect();
        if on_show.len() <= MAX_SHOWN {
            return;
        }
        on_show.sort_unstable();
        for (_, shown) in on_show.drain(..on_show.len() - MAX_SHOWN) {
            let Some(mirrored) = self.forget(shown).await else { continue };
            self.close(shown).await;
            self.folded.entry(device.to_owned()).or_default().0.push(mirrored.id);
        }
        self.update_summary(device).await;
    }

    async fn update_summary(&mut self, device: &str) {
        let Some((ids, summary)) = self.folded.get(device).map(|(ids, summary)| (ids.len(), *summary)) else { return };
        if ids == 0 {
            self.folded.remove(device);
            return self.close(summary).await;
        }
        let text = format!("{ids} more from {}", self.device_name(device).await);
        if let Some(shown) = self.notify_own(summary, &text, "", false).await
            && let Some(entry) = self.folded.get_mut(device)
        {
            entry.1 = shown;
        }
    }

    async fn update_count(&mut self) {
        if self.held.is_empty() {
            let count = std::mem::take(&mut self.count);
            return self.close(count).await;
        }
        let summary = match self.held.len() {
            1 => "1 phone notification".to_owned(),
            n => format!("{n} phone notifications"),
        };
        if let Some(shown) = self.notify_own(self.count, &summary, "Unlock to see them.", false).await {
            self.count = shown;
        }
    }

    /// Link's own notice, not a phone's; `replaces` 0 for a new one.
    async fn notify_own(&self, replaces: u32, summary: &str, body: &str, alert: bool) -> Option<u32> {
        let mut hints: HashMap<&str, Variant<'_>> = HashMap::new();
        hints.insert("desktop-entry", Variant::from("org.andromeda.Link"));
        if !alert {
            hints.insert("suppress-sound", Variant::from(true));
        }
        let actions: Vec<String> = Vec::new();
        let result: zbus::Result<u32> =
            self.proxy.call("Notify", &("Link", replaces, "phone", summary, body, actions, hints, -1i32)).await;
        result.inspect_err(|error| log::warn!("Notify: {error}")).ok()
    }

    async fn close(&self, shown: u32) {
        if shown == 0 {
            return;
        }
        let result: zbus::Result<()> = self.proxy.call("CloseNotification", &(shown,)).await;
        if let Err(error) = result {
            log::debug!("CloseNotification: {error}");
        }
    }

    async fn device_name(&self, device: &str) -> String {
        let devices = self.link.proxy.devices().await.unwrap_or_default();
        devices.into_iter().find(|(id, ..)| id == device).map_or_else(|| "your phone".to_owned(), |(_, name, _)| name)
    }

    pub async fn removed(&mut self, device: &str, id: &str) {
        if self.held.iter().any(|held| held.device == device && held.id == id) {
            self.held.retain(|held| held.device != device || held.id != id);
            self.update_count().await;
        }
        if let Some((ids, _)) = self.folded.get_mut(device)
            && let Some(at) = ids.iter().position(|folded| folded == id)
        {
            ids.remove(at);
            self.update_summary(device).await;
        }
        let Some(shown) = self.find(device, id) else { return };
        // Forgotten first, so the server's NotificationClosed does not dismiss it on the phone again.
        self.forget(shown).await;
        let result: zbus::Result<()> = self.proxy.call("CloseNotification", &(shown,)).await;
        if let Err(error) = result {
            log::debug!("CloseNotification: {error}");
        }
    }

    pub async fn action_invoked(&mut self, shown: u32, key: &str) {
        let Some(mirrored) = self.shown.get_mut(&shown) else { return };
        if let Some(text) = key.strip_prefix(INLINE_REPLY_PREFIX) {
            self.reply(shown, text.to_owned()).await;
        } else if let Some(action) = key.strip_prefix(ACTION_PREFIX) {
            mirrored.acted = true;
            self.link.notification_action(&mirrored.device, &mirrored.id, action, "").await;
        }
    }

    pub async fn replied(&mut self, shown: u32, text: String) {
        self.reply(shown, text).await;
    }

    pub async fn closed(&mut self, shown: u32, reason: u32) {
        // Link's own notices: a dismissed one is shown again only when its count or state changes.
        self.folded.values_mut().filter(|(_, summary)| *summary == shown).for_each(|(_, summary)| *summary = 0);
        self.access_off.values_mut().filter(|notice| **notice == shown).for_each(|notice| *notice = 0);
        if self.count == shown {
            self.count = 0;
        }
        let Some(mirrored) = self.forget(shown).await else { return };
        if reason == DISMISSED && !mirrored.acted {
            self.link.notification_dismiss(&mirrored.device, &mirrored.id).await;
        }
    }

    async fn reply(&mut self, shown: u32, text: String) {
        let Some(mirrored) = self.shown.get_mut(&shown) else { return };
        let Some(action) = mirrored.reply_action.clone() else { return };
        mirrored.acted = true;
        self.link.notification_action(&mirrored.device, &mirrored.id, &action, &text).await;
    }

    fn find(&self, device: &str, id: &str) -> Option<u32> {
        self.shown.iter().find(|(_, mirrored)| mirrored.device == device && mirrored.id == id).map(|(shown, _)| *shown)
    }

    async fn forget(&mut self, shown: u32) -> Option<Mirrored> {
        let mirrored = self.shown.remove(&shown)?;
        if let Some(icon) = &mirrored.icon {
            let _ = tokio::fs::remove_file(icon).await;
        }
        Some(mirrored)
    }

    /// The icon as a file for the `image-path` hint, which every server reads, unlike raw `image-data`.
    async fn write_icon(&self, posted: &Posted) -> Option<PathBuf> {
        let (width, height) = png_size(&posted.icon)?;
        if !(1..=MAX_ICON_SIDE).contains(&width) || !(1..=MAX_ICON_SIDE).contains(&height) {
            log::info!("phone notification icon of {width}x{height} ignored");
            return None;
        }
        let mut hasher = DefaultHasher::new();
        (&posted.device, &posted.id, &posted.icon).hash(&mut hasher);
        let path = self.icons.join(format!("{:016x}.png", hasher.finish()));
        let written = async {
            tokio::fs::create_dir_all(&self.icons).await?;
            tokio::fs::write(&path, &posted.icon).await
        };
        match written.await {
            Ok(()) => Some(path),
            Err(error) => {
                log::warn!("writing a notification icon: {error}");
                None
            }
        }
    }
}

/// The width and height in a PNG's header, read before any server decodes it.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    const HEADER: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR";
    let rest = png.strip_prefix(HEADER)?;
    let side = |at: usize| Some(u32::from_be_bytes(rest.get(at..at + 4)?.try_into().ok()?));
    Some((side(0)?, side(4)?))
}

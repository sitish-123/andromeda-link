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
        }
    }
}

struct Mirrored {
    device: String,
    id: String,
    reply_action: Option<String>,
    acted: bool,
    icon: Option<PathBuf>,
}

pub struct Notifications {
    pub proxy: zbus::Proxy<'static>,
    link: Link,
    inline_reply: bool,
    icons: PathBuf,
    shown: HashMap<u32, Mirrored>,
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
        })
    }

    pub async fn posted(&mut self, posted: Posted) {
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
        self.shown.insert(shown, Mirrored { device: posted.device, id: posted.id, reply_action, acted: false, icon });
    }

    pub async fn removed(&mut self, device: &str, id: &str) {
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

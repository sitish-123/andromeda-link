//! The phone notifications on show, per device, kept on disk so that neither a reconnect nor a restart shows one again.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use link_core::identity::DeviceId;
use link_core::proto::message::{NotificationDigest, NotificationPosted};

pub struct Mirror {
    /// One `<device id>.json` per device: its notifications, oldest first.
    dir: PathBuf,
    live: HashMap<DeviceId, Vec<NotificationPosted>>,
}

impl Mirror {
    /// Loads what earlier runs left in `dir`; a file it cannot read counts as empty.
    pub fn load(dir: PathBuf) -> Self {
        let mut live = HashMap::new();
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|stem| DeviceId::parse(&stem.to_string_lossy()).ok()) else {
                continue;
            };
            match fs::read(&path).map_err(anyhow::Error::from).and_then(|bytes| Ok(serde_json::from_slice(&bytes)?)) {
                Ok(shown) => drop(live.insert(id, shown)),
                Err(error) => log::warn!("reading {}: {error}", path.display()),
            }
        }
        Self { dir, live }
    }

    /// None for an identical re-post; else whether the desktop alerts: for a new one, or a changed title or text
    /// without `alert_once`.
    pub fn post(&mut self, device: &DeviceId, posted: &NotificationPosted) -> Option<bool> {
        let live = self.live.entry(device.clone()).or_default();
        let alert = match live.iter().position(|shown| shown.id == posted.id) {
            Some(index) if live[index].digest() == posted.digest() => return None,
            Some(index) => {
                let old = live.remove(index);
                !posted.alert_once && (old.title != posted.title || old.text != posted.text)
            }
            None => true,
        };
        live.push(posted.clone());
        self.save(device);
        Some(alert)
    }

    /// Whether `id` was on show.
    pub fn remove(&mut self, device: &DeviceId, id: &str) -> bool {
        let Some(live) = self.live.get_mut(device) else { return false };
        let before = live.len();
        live.retain(|shown| shown.id != id);
        let removed = live.len() != before;
        if removed {
            self.save(device);
        }
        removed
    }

    /// Removes what the phone no longer shows and returns those ids, then the ids to pull: new or changed ones.
    pub fn snapshot(&mut self, device: &DeviceId, digests: &[NotificationDigest]) -> (Vec<String>, Vec<String>) {
        let live = self.live.entry(device.clone()).or_default();
        let (kept, gone): (Vec<_>, Vec<_>) =
            live.drain(..).partition(|shown| digests.iter().any(|digest| digest.id == shown.id));
        *live = kept;
        let pull = digests
            .iter()
            .filter(|digest| !live.iter().any(|shown| shown.id == digest.id && shown.digest() == digest.hash))
            .map(|digest| digest.id.clone())
            .collect();
        let gone: Vec<String> = gone.into_iter().map(|shown| shown.id).collect();
        if !gone.is_empty() {
            self.save(device);
        }
        (gone, pull)
    }

    /// Drops every notification of `device` and returns their ids.
    pub fn forget(&mut self, device: &DeviceId) -> Vec<String> {
        if let Err(error) = fs::remove_file(self.path(device))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("removing {}: {error}", self.path(device).display());
        }
        self.live.remove(device).unwrap_or_default().into_iter().map(|shown| shown.id).collect()
    }

    fn path(&self, device: &DeviceId) -> PathBuf {
        self.dir.join(format!("{device}.json"))
    }

    fn save(&self, device: &DeviceId) {
        let path = self.path(device);
        let tmp = path.with_extension("tmp");
        let shown = self.live.get(device).map(Vec::as_slice).unwrap_or_default();
        let written = fs::create_dir_all(&self.dir)
            .and_then(|()| {
                let mut file = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
                file.write_all(&serde_json::to_vec(shown)?)
            })
            .and_then(|()| fs::rename(&tmp, &path));
        if let Err(error) = written {
            log::error!("saving {}: {error}", path.display());
        }
    }
}

//! Media both ways, owned by the hub: the desktop's players as phones see them, and each phone's players, the latest
//! of which is exported over MPRIS.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use link_core::identity::DeviceId;
use link_core::proto::message::{MediaGone, MediaPlayer, Message, PlaybackState};
use link_core::session::SessionHandle;

use crate::desktop_media::{DesktopMediaHandle, Request};
use crate::hub::HubHandle;
use crate::mpris::{Exported, write_art};

/// Players kept per phone; a new one beyond this drops the least recently updated.
const MAX_PHONE_PLAYERS: usize = 8;

/// `(player, app, title, artist, art URL, playing, position ms, length ms or 0, volume or -1, when the position was
/// true in ms since the Unix epoch, commands it takes)`, a phone's lead player in the D-Bus `PhonePlayers` property.
pub type PhonePlayer = (String, String, String, String, String, bool, u64, u64, i32, i64, Vec<String>);

#[derive(Default)]
struct PhonePlayers {
    /// Least recently updated first, each with when it reported in ms since the Unix epoch.
    players: Vec<(MediaPlayer, i64)>,
    exported: Option<Exported>,
    /// The lead player's artwork file and its file:// URL.
    art: Option<(PathBuf, String)>,
}

impl PhonePlayers {
    /// The newest playing player leads; a paused one that reports later does not take its place.
    fn lead(&self) -> Option<&(MediaPlayer, i64)> {
        let mut players = self.players.iter().rev();
        players.clone().find(|(player, _)| player.state == PlaybackState::Playing).or_else(|| players.next())
    }
}

pub struct Media {
    desktop: BTreeMap<String, MediaPlayer>,
    phones: HashMap<DeviceId, PhonePlayers>,
    requests: DesktopMediaHandle,
    art: PathBuf,
}

impl Media {
    pub fn new(requests: DesktopMediaHandle, art: PathBuf) -> Self {
        Self { desktop: BTreeMap::new(), phones: HashMap::new(), requests, art }
    }

    pub fn desktop_player<'a>(&mut self, player: MediaPlayer, sessions: impl Iterator<Item = &'a SessionHandle>) {
        self.desktop.insert(player.player.clone(), player.clone());
        post_all(sessions, &Message::MediaPlayer(player));
    }

    pub fn desktop_gone<'a>(&mut self, player: String, sessions: impl Iterator<Item = &'a SessionHandle>) {
        if self.desktop.remove(&player).is_some() {
            post_all(sessions, &Message::MediaGone(MediaGone { player }));
        }
    }

    /// A new session hears every desktop player.
    pub fn connected(&self, session: &SessionHandle) {
        for player in self.desktop.values() {
            post_all(std::iter::once(session), &Message::MediaPlayer(player.clone()));
        }
    }

    pub async fn request(&self, request: Request) {
        self.requests.request(request).await;
    }

    pub async fn on_phone_message(&mut self, hub: &HubHandle, device: &DeviceId, device_name: &str, message: Message) {
        let phone = self.phones.entry(device.clone()).or_default();
        match message {
            Message::MediaPlayer(player) => {
                phone.players.retain(|(known, _)| known.player != player.player);
                if phone.players.len() >= MAX_PHONE_PLAYERS {
                    phone.players.remove(0);
                }
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis());
                phone.players.push((player, i64::try_from(now).unwrap_or(i64::MAX)));
            }
            Message::MediaGone(gone) => phone.players.retain(|(known, _)| known.player != gone.player),
            Message::MediaCommand(command) => return self.requests.request(Request::Command(command)).await,
            other => return log::warn!("{device}: {} is not media", other.kind()),
        }
        let Some((latest, _)) = phone.lead().cloned() else {
            if let Some((file, _)) = phone.art.take() {
                drop(std::fs::remove_file(file));
            }
            if let Some(exported) = phone.exported.take() {
                exported.close().await;
            }
            return;
        };
        let (file, url) = write_art(&self.art, device, &latest);
        if let Some((old, _)) = phone.art.take_if(|(old, _)| Some(&*old) != file.as_ref()) {
            drop(std::fs::remove_file(old));
        }
        phone.art = file.zip(url);
        // MPRIS needs a session bus; macOS has none, and the app reads `PhonePlayers` there.
        if cfg!(target_os = "macos") {
            return;
        }
        let result = match &mut phone.exported {
            Some(exported) => exported.update(device, &latest).await,
            None => match Exported::start(hub.clone(), device.clone(), device_name, &latest, &self.art).await {
                Ok(exported) => {
                    phone.exported = Some(exported);
                    Ok(())
                }
                Err(error) => Err(error),
            },
        };
        if let Err(error) = result {
            log::warn!("{device}: exporting its player: {error}");
        }
    }

    /// The phone's players go with its session.
    pub async fn disconnected(&mut self, device: &DeviceId) {
        let Some(phone) = self.phones.remove(device) else { return };
        if let Some((file, _)) = phone.art {
            drop(std::fs::remove_file(file));
        }
        if let Some(exported) = phone.exported {
            exported.close().await;
        }
    }

    /// Each phone's lead player, sorted by device id.
    pub fn players(&self) -> Vec<(String, PhonePlayer)> {
        let mut players: Vec<_> = self
            .phones
            .iter()
            .filter_map(|(device, phone)| {
                let (player, at) = phone.lead()?;
                let can =
                    player.can.iter().filter_map(|kind| serde_json::to_value(kind).ok()?.as_str().map(str::to_owned));
                let value = (
                    player.player.clone(),
                    player.name.clone(),
                    player.title.clone(),
                    player.artist.clone(),
                    phone.art.as_ref().map(|(_, url)| url.clone()).unwrap_or_default(),
                    player.state == PlaybackState::Playing,
                    player.position_ms,
                    player.length_ms.unwrap_or(0),
                    player.volume.map_or(-1, i32::from),
                    *at,
                    can.collect(),
                );
                Some((device.to_string(), value))
            })
            .collect();
        players.sort_by(|a, b| a.0.cmp(&b.0));
        players
    }
}

fn post_all<'a>(sessions: impl Iterator<Item = &'a SessionHandle>, message: &Message) {
    for session in sessions {
        if !session.post(message.clone()) {
            log::info!("a session's queue is full; dropped a {}", message.kind());
        }
    }
}

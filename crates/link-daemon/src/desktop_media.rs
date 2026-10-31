//! The desktop's own MPRIS players, reported to phones and commanded by them. Event-driven: a player is read again
//! when it appears, changes a property, or seeks.

use std::collections::HashMap;

use futures_util::StreamExt;
use link_core::proto::message::{
    MAX_ARTWORK_LEN, MAX_METADATA_LEN, MAX_PLAYER_LEN, MediaCommand, MediaCommandKind, MediaPlayer, PlaybackState,
};
use tokio::sync::mpsc;
use zbus::fdo::{DBusProxy, PropertiesProxy};
use zbus::message::Type;
use zbus::names::{BusName, InterfaceName};
use zbus::{MatchRule, MessageStream, Proxy};
use zbus::{ObjectPath, OwnedVariant, Variant};

use crate::hub::HubHandle;
use crate::mpris::NAME_PREFIX;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// Players reported to phones; more are left out.
const MAX_PLAYERS: usize = 8;

/// What the daemon asks of the desktop's players.
#[derive(Clone)]
pub struct DesktopMediaHandle(mpsc::Sender<Request>);

pub enum Request {
    Command(MediaCommand),
    /// Pauses every playing player and remembers which, for a phone call.
    PauseAll,
    /// Resumes the players `PauseAll` paused.
    ResumePaused,
}

struct Watched {
    /// The bus name's unique owner, to match signals by sender.
    owner: String,
    track: Option<String>,
    player: MediaPlayer,
}

/// On macOS, the Mac's own player for phones: what plays, from the now-playing adapter (crates/link-nowplaying) that
/// Apple's perl loads. Media keys when the adapter does not run.
pub async fn now_playing(hub: HubHandle, mut requests: mpsc::Receiver<Request>) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
    const PERL: &str = "use DynaLoader; my $l = DynaLoader::dl_load_file($ARGV[0], 0) or die DynaLoader::dl_error(); \
        my $s = DynaLoader::dl_find_symbol($l, 'link_now_playing') or die DynaLoader::dl_error(); \
        DynaLoader::dl_install_xsub('main::run', $s); run();";
    let library = std::env::current_exe()?.with_file_name("../Frameworks/liblink_nowplaying.dylib");
    let child = tokio::process::Command::new("/usr/bin/perl")
        .args(["-e", PERL])
        .arg(&library)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let (mut child, mut stdin, stdout) = match child {
        Ok(mut child) => match (child.stdin.take(), child.stdout.take()) {
            (Some(stdin), Some(stdout)) => (child, stdin, stdout),
            _ => return keys(hub, requests).await,
        },
        Err(error) => {
            log::warn!("the now-playing adapter did not start: {error}; media keys only");
            return keys(hub, requests).await;
        }
    };
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let (mut shown, mut playing, mut paused_for_call) = (false, false, false);
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else {
                    log::warn!("the now-playing adapter stopped ({:?}); media keys only", child.try_wait());
                    if shown {
                        hub.desktop_player_gone(MAC_PLAYER.to_owned()).await;
                    }
                    return keys(hub, requests).await;
                };
                match mac_player(&line) {
                    Some(player) => {
                        playing = player.state == PlaybackState::Playing;
                        hub.desktop_player(player).await;
                        shown = true;
                    }
                    None if shown => {
                        hub.desktop_player_gone(MAC_PLAYER.to_owned()).await;
                        (shown, playing) = (false, false);
                    }
                    None => {}
                }
            }
            request = requests.recv() => {
                let Some(request) = request else { return Ok(()) };
                let line = match request {
                    Request::Command(command) => match command.command {
                        MediaCommandKind::Play => "play".to_owned(),
                        MediaCommandKind::Pause => "pause".to_owned(),
                        MediaCommandKind::PlayPause => "play-pause".to_owned(),
                        MediaCommandKind::Next => "next".to_owned(),
                        MediaCommandKind::Previous => "previous".to_owned(),
                        MediaCommandKind::Seek => format!("seek {}", command.value.unwrap_or(0)),
                        MediaCommandKind::Volume => continue,
                    },
                    Request::PauseAll if playing => {
                        paused_for_call = true;
                        "pause".to_owned()
                    }
                    Request::ResumePaused if std::mem::take(&mut paused_for_call) => "play".to_owned(),
                    Request::PauseAll | Request::ResumePaused => continue,
                };
                if let Err(error) = stdin.write_all(format!("{line}\n").as_bytes()).await {
                    log::warn!("telling the now-playing adapter {line}: {error}");
                }
            }
        }
    }
}

const MAC_PLAYER: &str = "mac";

/// The adapter's JSON line as the phone's player; None for `{}`, nothing playing.
fn mac_player(line: &str) -> Option<MediaPlayer> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let text =
        |key: &str| truncate(value.get(key).and_then(serde_json::Value::as_str).unwrap_or_default(), MAX_METADATA_LEN);
    let number = |key: &str| value.get(key).and_then(serde_json::Value::as_u64);
    let title = text("title");
    if title.is_empty() {
        return None;
    }
    let artwork = value
        .get("artwork")
        .and_then(serde_json::Value::as_str)
        .and_then(|path| std::fs::read(path).ok())
        .filter(|bytes| bytes.len() <= MAX_ARTWORK_LEN);
    Some(MediaPlayer {
        player: MAC_PLAYER.to_owned(),
        name: truncate(&text("app"), MAX_PLAYER_LEN),
        state: if value.get("playing").and_then(serde_json::Value::as_bool) == Some(true) {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        },
        title,
        artist: text("artist"),
        album: text("album"),
        length_ms: number("duration_ms").filter(|ms| *ms > 0),
        position_ms: number("position_ms").unwrap_or(0),
        volume: None,
        artwork,
        can: vec![
            MediaCommandKind::Play,
            MediaCommandKind::Pause,
            MediaCommandKind::PlayPause,
            MediaCommandKind::Next,
            MediaCommandKind::Previous,
            MediaCommandKind::Seek,
        ],
    })
}

/// On a desktop without MPRIS (macOS), the one player phones see: its commands press the media keys through
/// `link-session`. Its state is unknown, so it shows as paused.
pub async fn keys(hub: HubHandle, mut requests: mpsc::Receiver<Request>) -> anyhow::Result<()> {
    hub.desktop_player(MediaPlayer {
        player: "media-keys".to_owned(),
        name: "Media keys".to_owned(),
        state: PlaybackState::Paused,
        title: "Media keys".to_owned(),
        artist: String::new(),
        album: String::new(),
        length_ms: None,
        position_ms: 0,
        volume: None,
        artwork: None,
        can: vec![MediaCommandKind::PlayPause, MediaCommandKind::Next, MediaCommandKind::Previous],
    })
    .await;
    while let Some(request) = requests.recv().await {
        let Request::Command(command) = request else { continue };
        let key = match command.command {
            MediaCommandKind::Play | MediaCommandKind::Pause | MediaCommandKind::PlayPause => "play-pause",
            MediaCommandKind::Next => "next",
            MediaCommandKind::Previous => "previous",
            MediaCommandKind::Seek | MediaCommandKind::Volume => continue,
        };
        hub.media_key(key).await;
    }
    Ok(())
}

pub fn channel() -> (DesktopMediaHandle, mpsc::Receiver<Request>) {
    let (sender, receiver) = mpsc::channel(16);
    (DesktopMediaHandle(sender), receiver)
}

impl DesktopMediaHandle {
    pub async fn request(&self, request: Request) {
        if self.0.send(request).await.is_err() {
            log::debug!("desktop media stopped");
        }
    }
}

struct DesktopMedia {
    bus: zbus::Connection,
    hub: HubHandle,
    /// By bus name.
    players: HashMap<String, Watched>,
    paused_for_call: Vec<String>,
}

pub async fn run(bus: zbus::Connection, hub: HubHandle, mut requests: mpsc::Receiver<Request>) -> anyhow::Result<()> {
    let owners_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .sender("org.freedesktop.DBus")
        .interface("org.freedesktop.DBus")
        .member("NameOwnerChanged")
        .arg0ns("org.mpris.MediaPlayer2")
        .build()?;
    let changes_rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .interface("org.freedesktop.DBus.Properties")
        .member("PropertiesChanged")
        .path(PATH)
        .build()?;
    let seeks_rule =
        MatchRule::builder().msg_type(Type::Signal).interface(PLAYER).member("Seeked").path(PATH).build()?;
    let mut owners = MessageStream::for_match_rule(owners_rule, &bus, None).await?;
    let mut changes = MessageStream::for_match_rule(changes_rule, &bus, None).await?;
    let mut seeks = MessageStream::for_match_rule(seeks_rule, &bus, None).await?;
    let mut media = DesktopMedia { bus: bus.clone(), hub, players: HashMap::new(), paused_for_call: Vec::new() };
    for name in DBusProxy::new(&bus).await?.list_names().await? {
        media.appeared(name.as_str()).await;
    }
    loop {
        tokio::select! {
            Some(Ok(message)) = owners.next() => {
                let Ok((name, _, owner)) = message.body().deserialize::<(String, String, String)>() else { continue };
                if owner.is_empty() { media.gone(&name).await } else { media.appeared(&name).await }
            }
            Some(Ok(message)) = changes.next() => media.signalled(&message).await,
            Some(Ok(message)) = seeks.next() => media.signalled(&message).await,
            request = requests.recv() => match request {
                Some(request) => media.handle(request).await,
                None => return Ok(()),
            },
        }
    }
}

impl DesktopMedia {
    async fn appeared(&mut self, name: &str) {
        if !name.starts_with(PREFIX) || name.starts_with(NAME_PREFIX) {
            return;
        }
        if !self.players.contains_key(name) && self.players.len() >= MAX_PLAYERS {
            return log::info!("{name}: more than {MAX_PLAYERS} players, left out");
        }
        self.refresh(name).await;
    }

    async fn gone(&mut self, name: &str) {
        if let Some(watched) = self.players.remove(name) {
            self.hub.desktop_player_gone(watched.player.player).await;
        }
    }

    async fn signalled(&mut self, message: &zbus::Message) {
        let header = message.header();
        let Some(sender) = header.sender() else { return };
        let name =
            self.players.iter().find(|(_, watched)| watched.owner == sender.as_str()).map(|(name, _)| name.clone());
        if let Some(name) = name {
            self.refresh(&name).await;
        }
    }

    async fn refresh(&mut self, name: &str) {
        match self.read(name).await {
            Ok(watched) => {
                let changed = self.players.get(name).is_none_or(|old| old.player != watched.player);
                let player = watched.player.clone();
                self.players.insert(name.to_owned(), watched);
                if changed {
                    self.hub.desktop_player(player).await;
                }
            }
            Err(error) => log::debug!("{name}: {error}"),
        }
    }

    async fn read(&self, name: &str) -> anyhow::Result<Watched> {
        let bus_name = BusName::try_from(name)?;
        let owner = DBusProxy::new(&self.bus).await?.get_name_owner(bus_name.clone()).await?.to_string();
        let properties = PropertiesProxy::builder(&self.bus).destination(bus_name).path(PATH).build().await?;
        let identity = properties
            .get(InterfaceName::from_static_str_unchecked("org.mpris.MediaPlayer2"), "Identity")
            .await
            .ok()
            .and_then(|value| String::try_from(value).ok());
        let all = properties.get_all(InterfaceName::from_static_str_unchecked(PLAYER)).await?;
        let id = truncate(&name[PREFIX.len()..], MAX_PLAYER_LEN);
        let (player, track) = describe(id, identity, &all);
        Ok(Watched { owner, track, player })
    }

    async fn handle(&mut self, request: Request) {
        match request {
            Request::Command(command) => self.command(command).await,
            Request::PauseAll => {
                let playing: Vec<String> = self
                    .players
                    .iter()
                    .filter(|(_, watched)| watched.player.state == PlaybackState::Playing)
                    .map(|(name, _)| name.clone())
                    .collect();
                for name in &playing {
                    self.call(name, "Pause", &()).await;
                }
                log::info!("paused {} players for a phone call", playing.len());
                self.paused_for_call = playing;
            }
            Request::ResumePaused => {
                for name in std::mem::take(&mut self.paused_for_call) {
                    self.call(&name, "Play", &()).await;
                }
            }
        }
    }

    async fn command(&mut self, command: MediaCommand) {
        let found = self.players.iter().find(|(_, watched)| watched.player.player == command.player);
        let Some((name, watched)) = found.map(|(name, watched)| (name.clone(), watched)) else { return };
        if !watched.player.can.contains(&command.command) {
            return log::info!("{name}: a phone asked for {:?}, which it does not offer", command.command);
        }
        let value = command.value.unwrap_or(0);
        match command.command {
            MediaCommandKind::Play => self.call(&name, "Play", &()).await,
            MediaCommandKind::Pause => self.call(&name, "Pause", &()).await,
            MediaCommandKind::PlayPause => self.call(&name, "PlayPause", &()).await,
            MediaCommandKind::Next => self.call(&name, "Next", &()).await,
            MediaCommandKind::Previous => self.call(&name, "Previous", &()).await,
            MediaCommandKind::Seek => {
                let position = i64::try_from(value).unwrap_or(i64::MAX).saturating_mul(1000);
                if let Some(track) = watched.track.clone().and_then(|track| ObjectPath::try_from(track).ok()) {
                    self.call(&name, "SetPosition", &(track, position)).await;
                } else {
                    // MPRIS signals no change while a track plays, so the cached position is stale: ask the player.
                    let cached = i64::try_from(watched.player.position_ms).unwrap_or(0).saturating_mul(1000);
                    let now = self.position(&name).await.unwrap_or(cached);
                    self.call(&name, "Seek", &(position - now)).await;
                }
            }
            MediaCommandKind::Volume => {
                #[expect(clippy::cast_precision_loss, reason = "the value is at most 100")]
                let volume = value as f64 / 100.0;
                self.set_volume(&name, volume).await;
            }
        }
    }

    async fn call<B>(&self, name: &str, method: &str, body: &B)
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let result = async {
            let proxy = Proxy::new(&self.bus, name.to_owned(), PATH, PLAYER).await?;
            proxy.call_method(method, body).await
        };
        if let Err(error) = result.await {
            log::info!("{name}: {method}: {error}");
        }
    }

    /// The player's position now, in microseconds.
    async fn position(&self, name: &str) -> Option<i64> {
        let result = async {
            let proxy = Proxy::new(&self.bus, name.to_owned(), PATH, PLAYER).await?;
            proxy.get_property::<i64>("Position").await
        };
        result.await.inspect_err(|error| log::info!("{name}: reading Position: {error}")).ok()
    }

    async fn set_volume(&self, name: &str, volume: f64) {
        let result = async {
            let proxy = Proxy::new(&self.bus, name.to_owned(), PATH, PLAYER).await?;
            proxy.set_property("Volume", volume).await.map_err(zbus::Error::from)
        };
        if let Err(error) = result.await {
            log::info!("{name}: setting Volume: {error}");
        }
    }
}

fn describe(
    id: String,
    identity: Option<String>,
    all: &HashMap<String, OwnedVariant>,
) -> (MediaPlayer, Option<String>) {
    let string = |key: &str| all.get(key).and_then(|value| String::try_from(value.clone()).ok());
    let flag = |key: &str| all.get(key).and_then(|value| bool::try_from(value.clone()).ok()).unwrap_or(false);
    let metadata: HashMap<String, OwnedVariant> = all
        .get("Metadata")
        .and_then(|value| HashMap::<String, OwnedVariant>::try_from(value.clone()).ok())
        .unwrap_or_default();
    let text = |key: &str| metadata.get(key).and_then(|value| String::try_from(value.clone()).ok()).unwrap_or_default();
    let artist = metadata
        .get("xesam:artist")
        .and_then(|value| Vec::<String>::try_from(value.clone()).ok())
        .map(|artists| artists.join(", "))
        .unwrap_or_default();
    let length_us = metadata.get("mpris:length").and_then(micros);
    let track = metadata.get("mpris:trackid").and_then(|value| match &**value {
        Variant::ObjectPath(path) => Some(path.to_string()),
        Variant::Str(path) => Some(path.to_string()),
        _ => None,
    });
    let state = match string("PlaybackStatus").as_deref() {
        Some("Playing") => PlaybackState::Playing,
        Some("Paused") => PlaybackState::Paused,
        _ => PlaybackState::Stopped,
    };
    let volume = all.get("Volume").and_then(|value| f64::try_from(value.clone()).ok());
    let mut can = Vec::new();
    let (play, pause) = (flag("CanPlay"), flag("CanPause"));
    for (offered, command) in [
        (play, MediaCommandKind::Play),
        (pause, MediaCommandKind::Pause),
        (play || pause, MediaCommandKind::PlayPause),
        (flag("CanGoNext"), MediaCommandKind::Next),
        (flag("CanGoPrevious"), MediaCommandKind::Previous),
        (flag("CanSeek"), MediaCommandKind::Seek),
        (volume.is_some() && flag("CanControl"), MediaCommandKind::Volume),
    ] {
        if offered {
            can.push(command);
        }
    }
    let name = identity.filter(|identity| !identity.is_empty()).unwrap_or_else(|| id.clone());
    let player = MediaPlayer {
        name: truncate(&name, MAX_PLAYER_LEN),
        player: id,
        state,
        title: truncate(&text("xesam:title"), MAX_METADATA_LEN),
        artist: truncate(&artist, MAX_METADATA_LEN),
        album: truncate(&text("xesam:album"), MAX_METADATA_LEN),
        length_ms: length_us.map(|us| us / 1000),
        position_ms: all.get("Position").and_then(micros).map_or(0, |us| us / 1000),
        volume: volume.map(percent),
        artwork: artwork(&text("mpris:artUrl")),
        can,
    };
    (player, track)
}

/// MPRIS times are `x` microseconds, though some players send `t`.
fn micros(value: &OwnedVariant) -> Option<u64> {
    match &**value {
        Variant::I64(us) => u64::try_from(*us).ok(),
        Variant::U64(us) => Some(*us),
        _ => None,
    }
}

#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0..=100 first")]
fn percent(volume: f64) -> u8 {
    (volume.clamp(0.0, 1.0) * 100.0).round() as u8
}

/// A local artwork file small enough to send as is; remote and larger images are left out.
fn artwork(url: &str) -> Option<Vec<u8>> {
    let path = url.strip_prefix("file://")?;
    let size = std::fs::metadata(path).ok()?.len();
    if size == 0 || size > u64::try_from(MAX_ARTWORK_LEN).ok()? {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let image = bytes.starts_with(b"\x89PNG") || bytes.starts_with(&[0xFF, 0xD8]);
    image.then_some(bytes)
}

fn truncate(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

//! The app's view of `andromeda-linkd`: a D-Bus client on its own thread, events in and commands out.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Seek, Write};
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use link_api::{Link1Proxy, SERVICE};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub battery: u32,
    pub charging: bool,
    /// wifi, cellular, ethernet, none, or other.
    pub network: String,
    /// Cellular bars, 0 to 4, or -1 without service.
    pub signal: i32,
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub device: String,
    pub id: String,
    pub app: String,
    pub title: String,
    pub text: String,
    pub icon: Vec<u8>,
    /// (id, label, takes text)
    pub actions: Vec<(String, String, bool)>,
}

#[derive(Debug)]
pub enum Event {
    /// The service is reachable (true) or gone (false).
    Service(bool),
    Devices(Vec<Device>),
    Status(HashMap<String, Status>),
    Notification(Notification),
    NotificationRemoved {
        device: String,
        id: String,
    },
    Call {
        device: String,
        state: String,
        number: String,
        name: String,
    },
    Offered {
        transfer: String,
        device: String,
        files: Vec<(String, u64)>,
    },
    Sending {
        transfer: String,
        files: Vec<(String, u64)>,
    },
    Progress {
        transfer: String,
        bytes: u64,
        total: u64,
    },
    Finished {
        transfer: String,
        status: String,
    },
    Shared(String),
    Received {
        device: String,
        text: String,
    },
    /// A pairing window opened: the code the phone shows and the URI its QR code holds.
    Pairing {
        code: String,
        uri: String,
    },
    Paired(String),
    /// The phone's player changed; None when it has none.
    Player {
        device: String,
        player: Option<Player>,
    },
    ClipSent(String),
    /// Device id to the features it holds.
    Grants(HashMap<String, Vec<String>>),
    AutoAccept(Vec<String>),
    LocalSend(bool),
    /// A folder on the phone, as `ListFiles` gives it.
    Listed {
        device: String,
        path: String,
        entries: Vec<Entry>,
    },
    Thumbnail {
        device: String,
        path: String,
        png: Vec<u8>,
    },
    /// A phone file copied to this path.
    Saved(PathBuf),
    /// Device id to its texts, newest first.
    Texts(HashMap<String, Vec<Text>>),
    /// Device id to its contacts as (name, numbers).
    Contacts(HashMap<String, Vec<(String, Vec<String>)>>),
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub folder: bool,
    pub size: u64,
    /// Unix seconds.
    pub mtime: u64,
}

#[derive(Clone, Debug)]
pub struct Text {
    pub thread: u64,
    pub address: String,
    pub body: String,
    /// Unix milliseconds.
    pub date: u64,
    pub sent: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Player {
    pub title: String,
    pub artist: String,
    pub art: Option<PathBuf>,
    pub playing: bool,
    /// Microseconds, as MPRIS gives them.
    pub position: i64,
    pub length: i64,
}

#[derive(Debug)]
pub enum Command {
    Ring(String),
    Reply {
        device: String,
        id: String,
        action: String,
        text: String,
    },
    Accept(String),
    Decline(String),
    Cancel(String),
    Share {
        device: String,
        text: String,
    },
    SendFiles {
        device: String,
        paths: Vec<PathBuf>,
    },
    /// mute or decline the ringing call.
    CallAction {
        device: String,
        action: String,
    },
    Pair,
    CancelPairing,
    PlayPause(String),
    /// Offers this text as the desktop's clipboard to the phones holding the clipboard grant.
    OfferClipboard(String),
    SetGrant {
        device: String,
        feature: String,
        granted: bool,
    },
    SetAutoAccept {
        device: String,
        enabled: bool,
    },
    SetLocalSend(bool),
    Unpair(String),
    List {
        device: String,
        path: String,
    },
    /// A thumbnail at most this many pixels on its longer side.
    Thumbnail {
        device: String,
        path: String,
        size: u16,
    },
    /// Copies a phone file into the Downloads folder.
    Save {
        device: String,
        path: String,
    },
    SendText {
        device: String,
        to: String,
        body: String,
    },
}

/// The `ReadFile` chunk, its largest allowed length.
const CHUNK: u32 = 1 << 20;

/// The prefix of the MPRIS names the service exports, one per phone, followed by the device id.
const PLAYER_PREFIX: &str = "org.mpris.MediaPlayer2.andromeda_link_";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";

/// Starts the client thread; it reconnects on its own when the service restarts.
pub fn start() -> (UnboundedSender<Command>, UnboundedReceiver<Event>) {
    let (commands, command_rx) = unbounded_channel();
    let (event_tx, events) = unbounded_channel();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => {
                log::error!("no tokio runtime: {error}");
                return;
            }
        };
        runtime.block_on(run(command_rx, event_tx));
    });
    (commands, events)
}

async fn run(mut commands: UnboundedReceiver<Command>, events: UnboundedSender<Event>) {
    loop {
        match session(&mut commands, &events).await {
            Ok(()) => return,
            Err(error) => log::info!("andromeda-linkd: {error}"),
        }
        let _ = events.send(Event::Service(false));
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// One connection to the service; returns Ok when the app closes, Err when the service is gone.
async fn session(commands: &mut UnboundedReceiver<Command>, events: &UnboundedSender<Event>) -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    let proxy = Link1Proxy::new(&bus).await?;
    let names = zbus::fdo::DBusProxy::new(&bus).await?;
    let mut owner = names.receive_name_owner_changed().await?;
    for name in names.list_names().await? {
        if name.starts_with(PLAYER_PREFIX) {
            tokio::spawn(watch_player(bus.clone(), name.to_string(), events.clone()));
        }
    }
    let devices = proxy.devices().await?;
    let _ = events.send(Event::Service(true));
    send_devices(events, devices);
    send_status(events, proxy.device_status().await.unwrap_or_default());
    let _ = events.send(Event::Grants(proxy.grants().await?));
    let _ = events.send(Event::AutoAccept(proxy.auto_accept().await?));
    let _ = events.send(Event::LocalSend(proxy.local_send_visible().await?));
    send_texts(events, proxy.texts().await?);
    let _ = events.send(Event::Contacts(proxy.contacts().await?));

    let mut devices = proxy.receive_devices_changed().await;
    let mut status = proxy.receive_device_status_changed().await;
    let mut grants = proxy.receive_grants_changed().await;
    let mut auto_accept = proxy.receive_auto_accept_changed().await;
    let mut localsend = proxy.receive_local_send_visible_changed().await;
    let mut texts = proxy.receive_texts_changed().await;
    let mut contacts = proxy.receive_contacts_changed().await;
    let mut signals = proxy.inner().receive_all_signals().await?;
    loop {
        tokio::select! {
            Some(change) = devices.next() => send_devices(events, change.get().await?),
            Some(change) = status.next() => send_status(events, change.get().await?),
            Some(change) = grants.next() => { let _ = events.send(Event::Grants(change.get().await?)); }
            Some(change) = auto_accept.next() => { let _ = events.send(Event::AutoAccept(change.get().await?)); }
            Some(change) = localsend.next() => { let _ = events.send(Event::LocalSend(change.get().await?)); }
            Some(change) = texts.next() => send_texts(events, change.get().await?),
            Some(change) = contacts.next() => { let _ = events.send(Event::Contacts(change.get().await?)); }
            Some(message) = signals.next() => {
                if let Some(event) = signal(message) {
                    let _ = events.send(event);
                }
            }
            Some(change) = owner.next() => {
                let args = change.args()?;
                if args.name() == SERVICE && args.new_owner().is_none() {
                    return Err(zbus::Error::Failure("the service stopped".into()));
                }
                if let Some(device) = args.name().strip_prefix(PLAYER_PREFIX) {
                    if args.new_owner().is_some() {
                        tokio::spawn(watch_player(bus.clone(), args.name().to_string(), events.clone()));
                    } else {
                        let _ = events.send(Event::Player { device: device.to_owned(), player: None });
                    }
                }
            }
            command = commands.recv() => match command {
                Some(command) => {
                    if let Err(error) = execute(&proxy, events, command).await {
                        let _ = events.send(Event::Failed(error.to_string()));
                    }
                }
                None => return Ok(()),
            },
        }
    }
}

/// Follows one phone's MPRIS player until its name goes away.
async fn watch_player(bus: zbus::Connection, name: String, events: UnboundedSender<Event>) {
    let Some(device) = name.strip_prefix(PLAYER_PREFIX).map(str::to_owned) else { return };
    let result: zbus::Result<()> = async {
        let properties =
            zbus::fdo::PropertiesProxy::builder(&bus).destination(name.as_str()).path(PLAYER_PATH).build().await?;
        let mut changes = properties.receive_properties_changed().await?;
        loop {
            let all =
                properties.get_all(zbus::names::InterfaceName::from_static_str_unchecked(PLAYER_INTERFACE)).await?;
            let _ = events.send(Event::Player { device: device.clone(), player: Some(player(&all)) });
            if changes.next().await.is_none() {
                return Ok(());
            }
        }
    }
    .await;
    if let Err(error) = result {
        log::debug!("{name}: {error}");
    }
}

fn player(all: &HashMap<String, zbus::OwnedVariant>) -> Player {
    use zbus::OwnedVariant as OwnedValue;
    let text = |value: Option<&OwnedValue>| value.and_then(|value| String::try_from(value.clone()).ok());
    let metadata: HashMap<String, OwnedValue> = all
        .get("Metadata")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| value.try_into().ok())
        .unwrap_or_default();
    let artist = metadata
        .get("xesam:artist")
        .and_then(|value| <Vec<String>>::try_from(value.clone()).ok())
        .map(|artists| artists.join(", "))
        .unwrap_or_default();
    let number = |value: Option<&OwnedValue>| {
        value.and_then(|value| {
            i64::try_from(value.clone())
                .ok()
                .or_else(|| u64::try_from(value.clone()).ok().and_then(|v| i64::try_from(v).ok()))
        })
    };
    Player {
        title: text(metadata.get("xesam:title")).unwrap_or_default(),
        artist,
        art: text(metadata.get("mpris:artUrl")).and_then(|url| url.strip_prefix("file://").map(PathBuf::from)),
        playing: text(all.get("PlaybackStatus")).as_deref() == Some("Playing"),
        position: number(all.get("Position")).unwrap_or(0),
        length: number(metadata.get("mpris:length")).unwrap_or(0),
    }
}

fn send_devices(events: &UnboundedSender<Event>, devices: Vec<(String, String, bool)>) {
    let devices = devices.into_iter().map(|(id, name, connected)| Device { id, name, connected }).collect();
    let _ = events.send(Event::Devices(devices));
}

fn send_status(events: &UnboundedSender<Event>, status: HashMap<String, (u32, bool, String, i32)>) {
    let status = status
        .into_iter()
        .map(|(id, (battery, charging, network, signal))| (id, Status { battery, charging, network, signal }))
        .collect();
    let _ = events.send(Event::Status(status));
}

#[allow(clippy::type_complexity)]
fn send_texts(events: &UnboundedSender<Event>, texts: HashMap<String, Vec<(u64, u64, String, String, u64, bool)>>) {
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

/// Copies a phone file into Downloads under a free name, a chunk at a time.
async fn save(proxy: &Link1Proxy<'_>, device: &str, path: &str) -> zbus::Result<PathBuf> {
    static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let failure = |error: std::io::Error| zbus::Error::Failure(format!("saving {path}: {error}"));
    let folder = gtk4::glib::user_special_dir(gtk4::glib::UserDirectory::Downloads)
        .unwrap_or_else(|| gtk4::glib::home_dir().join("Downloads"));
    std::fs::create_dir_all(&folder).map_err(failure)?;
    let name = path.rsplit('/').next().filter(|name| !name.is_empty()).unwrap_or("file");
    let save = SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = folder.join(format!(".{name}.{}-{save}.part", std::process::id()));
    let mut file = File::options().write(true).create_new(true).open(&partial).map_err(failure)?;
    let copied = async {
        let mut offset = 0;
        loop {
            let data = proxy.read_file(device, path, offset, CHUNK).await?;
            if data.is_empty() {
                return Ok(());
            }
            file.write_all(&data).map_err(failure)?;
            offset += data.len() as u64;
        }
    }
    .await
    .and_then(|()| claim(&folder, name, &partial).map_err(failure));
    let _ = std::fs::remove_file(&partial);
    copied
}

/// Links `partial` to the first free name of "name", "name (2)", ...; a link fails rather than replace a file.
fn claim(folder: &std::path::Path, name: &str, partial: &std::path::Path) -> std::io::Result<PathBuf> {
    let (stem, extension) =
        name.rsplit_once('.').map_or((name, String::new()), |(stem, ext)| (stem, format!(".{ext}")));
    for copy in 1.. {
        let target = if copy == 1 { folder.join(name) } else { folder.join(format!("{stem} ({copy}){extension}")) };
        match std::fs::hard_link(partial, &target) {
            Ok(()) => return Ok(target),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    unreachable!("an unbounded range")
}

fn signal(message: zbus::Message) -> Option<Event> {
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
            icon: args.icon.clone(),
            actions,
        }));
    }
    if let Some(signal) = link_api::NotificationRemoved::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::NotificationRemoved { device: owned(args.device_id), id: owned(args.id) });
    }
    if let Some(signal) = link_api::Call::from_message(message.clone()) {
        let args = signal.args().ok()?;
        let (device, state, number, name) = (args.device_id, args.state, args.number, args.name);
        return Some(Event::Call {
            device: owned(device),
            state: owned(state),
            number: owned(number),
            name: owned(name),
        });
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
        return Some(Event::Finished { transfer: owned(args.transfer_id), status: owned(args.status) });
    }
    if let Some(signal) = link_api::PairingFinished::from_message(message.clone()) {
        return Some(Event::Paired(owned(signal.args().ok()?.name)));
    }
    if let Some(signal) = link_api::PairingFailed::from_message(message.clone()) {
        return Some(Event::Failed(format!("Pairing failed: {}", signal.args().ok()?.reason)));
    }
    if let Some(signal) = link_api::Received::from_message(message) {
        let args = signal.args().ok()?;
        return Some(Event::Received { device: owned(args.device_id), text: owned(args.text) });
    }
    None
}

async fn execute(proxy: &Link1Proxy<'static>, events: &UnboundedSender<Event>, command: Command) -> zbus::Result<()> {
    match command {
        Command::Ring(device) => proxy.ring(&device, true).await,
        Command::Reply { device, id, action, text } => proxy.notification_action(&device, &id, &action, &text).await,
        Command::Accept(transfer) => proxy.accept_transfer(&transfer).await,
        Command::Decline(transfer) => proxy.decline_transfer(&transfer).await,
        Command::Cancel(transfer) => proxy.cancel_transfer(&transfer).await,
        Command::Share { device, text } => {
            proxy.share(&device, "text", &text).await?;
            let _ = events.send(Event::Shared(text));
            Ok(())
        }
        Command::SendFiles { device, paths } => {
            let mut files = Vec::new();
            let mut listed = Vec::new();
            for path in paths {
                let file =
                    File::open(&path).map_err(|error| zbus::Error::Failure(format!("{}: {error}", path.display())))?;
                let size = file.metadata().map_or(0, |meta| meta.len());
                let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                listed.push((name.clone(), size));
                files.push((file, name));
            }
            let pairs: Vec<(zbus::Fd<'_>, &str)> =
                files.iter().map(|(file, name)| (file.as_fd().into(), name.as_str())).collect();
            let transfer = proxy.send_files(&device, &pairs.iter().collect::<Vec<_>>()).await?;
            let _ = events.send(Event::Sending { transfer, files: listed });
            Ok(())
        }
        Command::CallAction { device, action } => proxy.call_action(&device, &action).await,
        Command::CancelPairing => proxy.cancel_pairing().await,
        Command::PlayPause(device) => {
            let name = format!("{PLAYER_PREFIX}{device}");
            let player = zbus::Proxy::new(proxy.inner().connection(), name, PLAYER_PATH, PLAYER_INTERFACE).await?;
            player.call("PlayPause", &()).await
        }
        Command::OfferClipboard(text) => {
            let failure = |error: std::io::Error| zbus::Error::Failure(format!("clipboard: {error}"));
            let fd = rustix::fs::memfd_create("link-clip", rustix::fs::MemfdFlags::CLOEXEC)
                .map_err(|error| failure(error.into()))?;
            let mut file = File::from(fd);
            file.write_all(text.as_bytes()).and_then(|()| file.rewind()).map_err(failure)?;
            proxy.offer_clipboard(&["text/plain;charset=utf-8", "text/plain"], file.as_fd().into()).await?;
            let _ = events.send(Event::ClipSent(text));
            Ok(())
        }
        Command::SetGrant { device, feature, granted } => proxy.set_grant(&device, &feature, granted).await,
        Command::SetAutoAccept { device, enabled } => proxy.set_auto_accept(&device, enabled).await,
        Command::SetLocalSend(visible) => proxy.set_local_send_visible(visible).await,
        Command::Unpair(device) => proxy.unpair(&device).await,
        // Phone reads wait on the phone, so they run beside the loop instead of holding the commands behind them.
        Command::List { device, path } => {
            let (proxy, events) = (proxy.clone(), events.clone());
            tokio::spawn(async move {
                let _ = events.send(match proxy.list_files(&device, &path).await {
                    Ok(entries) => {
                        let entries = entries
                            .into_iter()
                            .map(|(name, folder, size, mtime)| Entry { name, folder, size, mtime })
                            .collect();
                        Event::Listed { device, path, entries }
                    }
                    Err(error) => Event::Failed(error.to_string()),
                });
            });
            Ok(())
        }
        Command::Thumbnail { device, path, size } => {
            let (proxy, events) = (proxy.clone(), events.clone());
            tokio::spawn(async move {
                match proxy.thumbnail(&device, &path, size).await {
                    Ok(png) => drop(events.send(Event::Thumbnail { device, path, png })),
                    Err(error) => log::debug!("thumbnail {path}: {error}"),
                }
            });
            Ok(())
        }
        Command::Save { device, path } => {
            let (proxy, events) = (proxy.clone(), events.clone());
            tokio::spawn(async move {
                let _ = events.send(match save(&proxy, &device, &path).await {
                    Ok(saved) => Event::Saved(saved),
                    Err(error) => Event::Failed(error.to_string()),
                });
            });
            Ok(())
        }
        Command::SendText { device, to, body } => proxy.send_text(&device, &to, &body).await,
        Command::Pair => {
            let (code, uri) = proxy.start_pairing().await?;
            let _ = events.send(Event::Pairing { code, uri });
            Ok(())
        }
    }
}

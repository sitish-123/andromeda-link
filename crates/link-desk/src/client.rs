//! The app's view of `andromeda-linkd`: a D-Bus client on its own thread, events in and commands out.

use std::collections::HashMap;
use std::fs::File;
use std::hash::{Hash, Hasher};
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
pub struct Controls {
    /// Percent, or -1.
    pub media_volume: i32,
    /// normal, vibrate, silent, or empty.
    pub ringer: String,
    /// 1, 0, or -1 when the phone does not say.
    pub dnd: i32,
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub device: String,
    pub id: String,
    pub app: String,
    pub title: String,
    pub text: String,
    pub icon: Option<PathBuf>,
    /// (id, label, takes text)
    pub actions: Vec<(String, String, bool)>,
}

#[derive(Clone, Debug, Default)]
pub struct Player {
    /// The phone app, from the MPRIS Identity "<app> on <phone>".
    pub app: String,
    pub title: String,
    pub artist: String,
    /// A file:// URL the service wrote.
    pub art: String,
    pub playing: bool,
    /// Microseconds, as MPRIS gives them.
    pub position: i64,
    pub length: i64,
    /// 0.0 to 1.0.
    pub volume: f64,
}

#[derive(Debug)]
pub enum Event {
    /// The service is reachable (true) or gone (false).
    Service(bool),
    Devices(Vec<Device>),
    Status(HashMap<String, Status>),
    /// Device id to (last seen unix seconds, wifi, bluetooth, or empty).
    Info(HashMap<String, (i64, String)>),
    Controls(HashMap<String, Controls>),
    Notification(Notification),
    NotificationRemoved {
        device: String,
        id: String,
    },
    Offered {
        transfer: String,
        device: String,
    },
    Sending {
        transfer: String,
        device: String,
        files: Vec<String>,
    },
    Finished {
        transfer: String,
        status: String,
        paths: Vec<String>,
    },
    Received {
        device: String,
        text: String,
    },
    /// The phone's player changed; None when it has none.
    Player {
        device: String,
        player: Option<Player>,
    },
    ClipSent(String),
    /// Ages shown in the lists need a refresh.
    Tick,
    Failed(String),
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
    Dismiss {
        device: String,
        id: String,
    },
    SendFiles {
        device: String,
        paths: Vec<PathBuf>,
    },
    /// Offers this text as the desktop's clipboard to the phones holding the clipboard grant.
    OfferClipboard(String),
    /// set-media-volume, set-ringer, or set-dnd.
    System {
        device: String,
        command: &'static str,
        value: u32,
    },
    Player {
        device: String,
        action: PlayerAction,
    },
    StartService,
}

#[derive(Debug)]
pub enum PlayerAction {
    PlayPause,
    Next,
    Previous,
    /// Microseconds.
    SetPosition(i64),
    /// 0.0 to 1.0.
    SetVolume(f64),
}

/// The prefix of the MPRIS names the service exports, one per phone, followed by the device id.
const PLAYER_PREFIX: &str = "org.mpris.MediaPlayer2.andromeda_link_";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const ROOT_INTERFACE: &str = "org.mpris.MediaPlayer2";

/// Starts the client thread; `sink` gets every event on that thread. It reconnects on its own when the service
/// restarts, but never starts the service without `Command::StartService`.
pub fn start(sink: impl Fn(Event) + Send + 'static) -> UnboundedSender<Command> {
    let (commands, command_rx) = unbounded_channel();
    let (event_tx, mut events) = unbounded_channel();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => {
                log::error!("no tokio runtime: {error}");
                return;
            }
        };
        runtime.spawn(async move {
            while let Some(event) = events.recv().await {
                sink(event);
            }
        });
        let ticks = event_tx.clone();
        runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            while ticks.send(Event::Tick).is_ok() {
                interval.tick().await;
            }
        });
        runtime.block_on(run(command_rx, event_tx));
    });
    commands
}

async fn run(mut commands: UnboundedReceiver<Command>, events: UnboundedSender<Event>) {
    loop {
        match session(&mut commands, &events).await {
            Ok(()) => return,
            Err(error) => log::info!("andromeda-linkd: {error}"),
        }
        let _ = events.send(Event::Service(false));
        let retry = tokio::time::sleep(Duration::from_secs(2));
        tokio::pin!(retry);
        loop {
            tokio::select! {
                () = &mut retry => break,
                command = commands.recv() => match command {
                    Some(Command::StartService) => {
                        if let Err(error) = start_service().await {
                            let _ = events.send(Event::Failed(format!("Could not start the service: {error}")));
                        }
                        break;
                    }
                    Some(_) => { let _ = events.send(Event::Failed("The Link service is not running".into())); }
                    None => return,
                },
            }
        }
    }
}

async fn start_service() -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    zbus::fdo::DBusProxy::new(&bus).await?.start_service_by_name(SERVICE.try_into()?, 0).await?;
    Ok(())
}

/// One connection to the service; returns Ok when the app closes, Err when the service is gone.
async fn session(commands: &mut UnboundedReceiver<Command>, events: &UnboundedSender<Event>) -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    let names = zbus::fdo::DBusProxy::new(&bus).await?;
    // A call to an absent service would start it through D-Bus activation; only StartService may do that.
    if !names.name_has_owner(SERVICE.try_into()?).await? {
        return Err(zbus::Error::Failure("the service is not running".into()));
    }
    let proxy = Link1Proxy::new(&bus).await?;
    let mut owner = names.receive_name_owner_changed().await?;
    for name in names.list_names().await? {
        if name.starts_with(PLAYER_PREFIX) {
            tokio::spawn(watch_player(bus.clone(), name.to_string(), events.clone()));
        }
    }
    let devices = proxy.devices().await?;
    let _ = events.send(Event::Service(true));
    send_devices(events, devices);
    send_info(events, proxy.device_info().await.unwrap_or_default());
    send_status(events, proxy.device_status().await.unwrap_or_default());
    send_controls(events, proxy.device_controls().await.unwrap_or_default());

    let mut devices = proxy.receive_devices_changed().await;
    let mut info = proxy.receive_device_info_changed().await;
    let mut status = proxy.receive_device_status_changed().await;
    let mut controls = proxy.receive_device_controls_changed().await;
    let mut signals = proxy.inner().receive_all_signals().await?;
    loop {
        tokio::select! {
            Some(change) = devices.next() => send_devices(events, change.get().await?),
            Some(change) = info.next() => send_info(events, change.get().await?),
            Some(change) = status.next() => send_status(events, change.get().await?),
            Some(change) = controls.next() => send_controls(events, change.get().await?),
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

/// Follows one phone's MPRIS player until its name goes away; reads the position each second while it plays,
/// since MPRIS does not signal it.
async fn watch_player(bus: zbus::Connection, name: String, events: UnboundedSender<Event>) {
    let Some(device) = name.strip_prefix(PLAYER_PREFIX).map(str::to_owned) else { return };
    let result: zbus::Result<()> = async {
        let properties =
            zbus::fdo::PropertiesProxy::builder(&bus).destination(name.as_str()).path(PLAYER_PATH).build().await?;
        let interface = zbus::names::InterfaceName::from_static_str_unchecked(PLAYER_INTERFACE);
        let mut changes = properties.receive_properties_changed().await?;
        let mut current = None;
        loop {
            match current.as_mut() {
                Some(player @ &mut Player { playing: true, .. }) => {
                    tokio::select! {
                        change = changes.next() => if change.is_none() { return Ok(()) } else { current = None },
                        () = tokio::time::sleep(Duration::from_secs(1)) => {
                            let position = properties.get(interface.clone(), "Position").await?;
                            player.position = number(Some(&position)).unwrap_or(player.position);
                        }
                    }
                }
                Some(_) => {
                    if changes.next().await.is_none() {
                        return Ok(());
                    }
                    current = None;
                }
                None => {
                    let all = properties.get_all(interface.clone()).await?;
                    let identity = properties
                        .get(zbus::names::InterfaceName::from_static_str_unchecked(ROOT_INTERFACE), "Identity")
                        .await
                        .ok()
                        .and_then(|value| String::try_from(value).ok())
                        .unwrap_or_default();
                    current = Some(player(&all, &identity));
                }
            }
            let _ = events.send(Event::Player { device: device.clone(), player: current.clone() });
        }
    }
    .await;
    if let Err(error) = result {
        log::debug!("{name}: {error}");
    }
}

fn number(value: Option<&zbus::OwnedVariant>) -> Option<i64> {
    value.and_then(|value| {
        i64::try_from(value.clone())
            .ok()
            .or_else(|| u64::try_from(value.clone()).ok().and_then(|v| i64::try_from(v).ok()))
    })
}

fn player(all: &HashMap<String, zbus::OwnedVariant>, identity: &str) -> Player {
    let text = |value: Option<&zbus::OwnedVariant>| value.and_then(|value| String::try_from(value.clone()).ok());
    let metadata: HashMap<String, zbus::OwnedVariant> = all
        .get("Metadata")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| value.try_into().ok())
        .unwrap_or_default();
    let artist = metadata
        .get("xesam:artist")
        .and_then(|value| <Vec<String>>::try_from(value.clone()).ok())
        .map(|artists| artists.join(", "))
        .unwrap_or_default();
    Player {
        app: identity.split_once(" on ").map_or(identity, |(app, _)| app).to_owned(),
        title: text(metadata.get("xesam:title")).unwrap_or_default(),
        artist,
        art: text(metadata.get("mpris:artUrl")).unwrap_or_default(),
        playing: text(all.get("PlaybackStatus")).as_deref() == Some("Playing"),
        position: number(all.get("Position")).unwrap_or(0),
        length: number(metadata.get("mpris:length")).unwrap_or(0),
        volume: all.get("Volume").and_then(|value| f64::try_from(value.clone()).ok()).unwrap_or(1.0),
    }
}

fn send_devices(events: &UnboundedSender<Event>, devices: Vec<(String, String, bool)>) {
    let devices = devices.into_iter().map(|(id, name, connected)| Device { id, name, connected }).collect();
    let _ = events.send(Event::Devices(devices));
}

fn send_info(events: &UnboundedSender<Event>, info: HashMap<String, HashMap<String, zbus::OwnedVariant>>) {
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

fn send_status(events: &UnboundedSender<Event>, status: HashMap<String, (u32, bool, String, i32)>) {
    let status = status
        .into_iter()
        .map(|(id, (battery, charging, network, signal))| (id, Status { battery, charging, network, signal }))
        .collect();
    let _ = events.send(Event::Status(status));
}

fn send_controls(events: &UnboundedSender<Event>, controls: HashMap<String, (i32, String, i32, i32)>) {
    let controls = controls
        .into_iter()
        .map(|(id, (media_volume, ringer, dnd, _locked))| (id, Controls { media_volume, ringer, dnd }))
        .collect();
    let _ = events.send(Event::Controls(controls));
}

/// Writes a notification icon to the cache once per content, for QML to load by URL.
fn cache_icon(icon: &[u8]) -> Option<PathBuf> {
    if icon.is_empty() {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    icon.hash(&mut hasher);
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    let folder = cache.join("andromeda-link/icons");
    let path = folder.join(format!("{:016x}.png", hasher.finish()));
    if path.exists() {
        return Some(path);
    }
    match std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&path, icon)) {
        Ok(()) => Some(path),
        Err(error) => {
            log::warn!("{}: {error}", path.display());
            None
        }
    }
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
            icon: cache_icon(&args.icon),
            actions,
        }));
    }
    if let Some(signal) = link_api::NotificationRemoved::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::NotificationRemoved { device: owned(args.device_id), id: owned(args.id) });
    }
    if let Some(signal) = link_api::TransferOffered::from_message(message.clone()) {
        let args = signal.args().ok()?;
        return Some(Event::Offered { transfer: owned(args.transfer_id), device: owned(args.device_id) });
    }
    if let Some(signal) = link_api::TransferFinished::from_message(message.clone()) {
        let args = signal.args().ok()?;
        let paths = args.paths.iter().map(|&path| owned(path)).collect();
        return Some(Event::Finished { transfer: owned(args.transfer_id), status: owned(args.status), paths });
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
        Command::Dismiss { device, id } => proxy.notification_dismiss(&device, &id).await,
        Command::SendFiles { device, paths } => {
            let mut files = Vec::new();
            for path in paths {
                let file =
                    File::open(&path).map_err(|error| zbus::Error::Failure(format!("{}: {error}", path.display())))?;
                let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                files.push((file, name));
            }
            let pairs: Vec<(zbus::Fd<'_>, &str)> =
                files.iter().map(|(file, name)| (file.as_fd().into(), name.as_str())).collect();
            let transfer = proxy.send_files(&device, &pairs.iter().collect::<Vec<_>>()).await?;
            let files = files.into_iter().map(|(_, name)| name).collect();
            let _ = events.send(Event::Sending { transfer, device, files });
            Ok(())
        }
        Command::OfferClipboard(text) => {
            let failure = |error: std::io::Error| zbus::Error::Failure(format!("clipboard: {error}"));
            let fd = rustix::fs::memfd_create("link-clip", rustix::fs::MemfdFlags::CLOEXEC)
                .map_err(|error| failure(error.into()))?;
            let mut file = File::from(fd);
            file.write_all(text.as_bytes()).and_then(|()| file.rewind()).map_err(failure)?;
            let types = ["text/plain;charset=utf-8", "text/plain"].map(|mime| (mime, file.as_fd().into()));
            proxy.offer_clipboard(&types.iter().collect::<Vec<_>>()).await?;
            let _ = events.send(Event::ClipSent(text));
            Ok(())
        }
        Command::System { device, command, value } => proxy.send_system_command(&device, command, value).await,
        Command::Player { device, action } => {
            let name = format!("{PLAYER_PREFIX}{device}");
            let player = zbus::Proxy::new(proxy.inner().connection(), name, PLAYER_PATH, PLAYER_INTERFACE).await?;
            match action {
                PlayerAction::PlayPause => player.call("PlayPause", &()).await,
                PlayerAction::Next => player.call("Next", &()).await,
                PlayerAction::Previous => player.call("Previous", &()).await,
                PlayerAction::SetPosition(target) => {
                    let current = number(Some(&player.get_property::<zbus::OwnedVariant>("Position").await?));
                    player.call("Seek", &(target - current.unwrap_or(0))).await
                }
                PlayerAction::SetVolume(volume) => player.set_property("Volume", volume).await.map_err(Into::into),
            }
        }
        Command::StartService => Ok(()),
    }
}

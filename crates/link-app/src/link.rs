//! The app's view of `andromeda-linkd`: a D-Bus client on its own thread, events in and commands out.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Seek, Write};
use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::OwnedFd;

const SERVICE: &str = "org.andromeda.Link1";
const PATH: &str = "/org/andromeda/Link1";
const INTERFACE: &str = "org.andromeda.Link1";

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
    Failed(String),
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
}

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
    let proxy = zbus::Proxy::new_owned(bus.clone(), SERVICE, PATH, INTERFACE).await?;
    let names = zbus::fdo::DBusProxy::new(&bus).await?;
    let mut owner = names.receive_name_owner_changed().await?;
    for name in names.list_names().await? {
        if name.starts_with(PLAYER_PREFIX) {
            tokio::spawn(watch_player(bus.clone(), name.to_string(), events.clone()));
        }
    }
    let devices: Vec<(String, String, bool)> = proxy.get_property("Devices").await?;
    let _ = events.send(Event::Service(true));
    send_devices(events, devices);
    send_status(events, proxy.get_property("DeviceStatus").await.unwrap_or_default());

    let mut devices = proxy.receive_property_changed::<Vec<(String, String, bool)>>("Devices").await;
    let mut status = proxy.receive_property_changed::<HashMap<String, (u32, bool, String, i32)>>("DeviceStatus").await;
    let mut signals = proxy.receive_all_signals().await?;
    loop {
        tokio::select! {
            Some(change) = devices.next() => send_devices(events, change.get().await?),
            Some(change) = status.next() => send_status(events, change.get().await?),
            Some(message) = signals.next() => {
                if let Some(event) = signal(&message) {
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

fn signal(message: &zbus::Message) -> Option<Event> {
    let header = message.header();
    let body = message.body();
    match header.member()?.as_str() {
        "NotificationPosted" => {
            let (device, id, app, title, text, icon, actions) = body.deserialize().ok()?;
            Some(Event::Notification(Notification { device, id, app, title, text, icon, actions }))
        }
        "NotificationRemoved" => {
            let (device, id) = body.deserialize().ok()?;
            Some(Event::NotificationRemoved { device, id })
        }
        "Call" => {
            let (device, state, number, name) = body.deserialize().ok()?;
            Some(Event::Call { device, state, number, name })
        }
        "TransferOffered" => {
            let (transfer, device, files) = body.deserialize().ok()?;
            Some(Event::Offered { transfer, device, files })
        }
        "TransferProgress" => {
            let (transfer, bytes, total) = body.deserialize().ok()?;
            Some(Event::Progress { transfer, bytes, total })
        }
        "TransferFinished" => {
            let (transfer, status, _paths): (String, String, Vec<String>) = body.deserialize().ok()?;
            Some(Event::Finished { transfer, status })
        }
        "PairingFinished" => {
            let (_device, name): (String, String) = body.deserialize().ok()?;
            Some(Event::Paired(name))
        }
        "PairingFailed" => {
            let (reason,): (String,) = body.deserialize().ok()?;
            Some(Event::Failed(format!("Pairing failed: {reason}")))
        }
        "Received" => {
            let (device, _kind, text): (String, String, String) = body.deserialize().ok()?;
            Some(Event::Received { device, text })
        }
        _ => None,
    }
}

async fn execute(proxy: &zbus::Proxy<'_>, events: &UnboundedSender<Event>, command: Command) -> zbus::Result<()> {
    match command {
        Command::Ring(device) => proxy.call("Ring", &(device, true)).await,
        Command::Reply { device, id, action, text } => {
            proxy.call("NotificationAction", &(device, id, action, text)).await
        }
        Command::Accept(transfer) => proxy.call("AcceptTransfer", &(transfer,)).await,
        Command::Decline(transfer) => proxy.call("DeclineTransfer", &(transfer,)).await,
        Command::Cancel(transfer) => proxy.call("CancelTransfer", &(transfer,)).await,
        Command::Share { device, text } => {
            proxy.call::<_, _, ()>("Share", &(&device, "text", &text)).await?;
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
                files.push((OwnedFd::from(std::os::fd::OwnedFd::from(file)), name));
            }
            let transfer: String = proxy.call("SendFiles", &(&device, files)).await?;
            let _ = events.send(Event::Sending { transfer, files: listed });
            Ok(())
        }
        Command::CallAction { device, action } => proxy.call("CallAction", &(device, action)).await,
        Command::CancelPairing => proxy.call("CancelPairing", &()).await,
        Command::PlayPause(device) => {
            let name = format!("{PLAYER_PREFIX}{device}");
            let player = zbus::Proxy::new(proxy.connection(), name, PLAYER_PATH, PLAYER_INTERFACE).await?;
            player.call("PlayPause", &()).await
        }
        Command::OfferClipboard(text) => {
            let failure = |error: std::io::Error| zbus::Error::Failure(format!("clipboard: {error}"));
            let fd = rustix::fs::memfd_create("link-clip", rustix::fs::MemfdFlags::CLOEXEC)
                .map_err(|error| failure(error.into()))?;
            let mut file = File::from(fd);
            file.write_all(text.as_bytes()).and_then(|()| file.rewind()).map_err(failure)?;
            let mimes = vec!["text/plain;charset=utf-8", "text/plain"];
            proxy.call::<_, _, ()>("OfferClipboard", &(mimes, OwnedFd::from(std::os::fd::OwnedFd::from(file)))).await?;
            let _ = events.send(Event::ClipSent(text));
            Ok(())
        }
        Command::Pair => {
            let (code, uri): (String, String) = proxy.call("StartPairing", &()).await?;
            let _ = events.send(Event::Pairing { code, uri });
            Ok(())
        }
    }
}

//! The hub actor: sole owner of the device store, the pairing window, and the set of live sessions.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use link_core::discovery::Advertiser;
use link_core::identity::{DeviceId, Spki};
use link_core::net;
use link_core::proto::CloseCode;
use link_core::proto::message::{
    CALLS, CAMERA, CAN_CONTROL, CAN_INPUT, CAPABILITIES, CLIP_ITEMS, CONTROL, Call, CallState, Capabilities,
    DesktopStatus, FEATURE, FS_STAT, FS_WRITE, FsRefusal, Hello, Hotspot, HotspotEnd, HotspotJoined,
    MAX_SYSTEM_ERROR_LEN, MESSAGES, MediaPlayer, Message, NotificationPosted, NotificationPull, PhotoRefusal, Ping,
    RemoteInput, Ringer, SYSTEM, SYSTEM_CONTROL, Share, Status, SystemCommand, SystemCommandKind, SystemResult, Text,
    TransferId, WIFI_DIRECT_BY_ADDRESS, WifiDirect, WifiDirectReady,
};
use link_core::proto::pairing::{Secret, Secrets};
use link_core::session::{Route, SessionEvent, SessionHandle};
use link_core::store::{Feature, Peer, Store, StoredCommand, feature_of};
use link_core::transfer::{ClipPeer, Source, Status as TransferStatus, TransferEvent, TransferHandle};
use link_core::transport::Puncher;
use link_core::uri::{PairingUri, QR_SECRET_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use tokio::sync::{mpsc, oneshot, watch};

/// Android shows its pairing code for 30 s; the window covers it and the user's tap.
const BT_PAIRING_WINDOW: Duration = Duration::from_secs(60);
/// A slider drag on the desktop moves the phone's slider live, at most this often.
const DESKTOP_STATUS_EVERY: Duration = Duration::from_millis(150);
/// How long a desktop's command waits for the phone's `system-result`.
const SYSTEM_TIMEOUT: Duration = Duration::from_secs(5);
use tokio::task::JoinSet;
use tokio::time::Instant;
use zbus::OwnedObjectPath;

use crate::browse;
use crate::desktop_media::{DesktopMediaHandle, Request};
use crate::hotspot;
use crate::localsend::LocalSendHandle;
use crate::media::Media;
use crate::notifications::Mirror;
use crate::paths::Paths;

const WINDOW: Duration = Duration::from_secs(120);

pub enum Admission {
    Session,
    Pair(Attempt),
    Reject(CloseCode),
}

/// One pairing attempt: the secrets of the window that admitted it, and that window's id.
pub struct Attempt {
    pub secrets: Arc<Secrets>,
    pub window: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(clippy::struct_excessive_bools, reason = "one field per D-Bus property")]
pub struct Snapshot {
    /// `(device_id, name, connected)`, the D-Bus `Devices` property.
    pub devices: Vec<(String, String, bool)>,
    /// `(device_id, last_seen, transport)`, the D-Bus `DeviceInfo` property; transport is wifi, bluetooth, or empty.
    pub info: Vec<(String, i64, &'static str)>,
    pub pairing: bool,
    /// Devices whose file offers are accepted without asking, the D-Bus `AutoAccept` property.
    pub auto_accept: Vec<String>,
    /// Device id to granted features, the D-Bus `Grants` property.
    pub grants: Vec<(String, Vec<String>)>,
    /// Connected device id to `(battery, charging, network, signal)`, the D-Bus `DeviceStatus` property.
    pub status: Vec<(String, (u32, bool, String, i32))>,
    /// Connected device id to `(media volume, ringer, dnd, locked)`, the D-Bus `DeviceControls` property.
    pub controls: Vec<(String, (i32, String, i32, i32))>,
    /// Connected devices whose notification access is off, the D-Bus `NotificationsOff` property.
    pub notifications_off: Vec<String>,
    /// The D-Bus `NotificationCounts` property.
    pub notification_counts: Vec<(String, u32)>,
    /// The D-Bus `NotificationsWhileLocked` property.
    pub notify_locked: bool,
    /// "Visible to LocalSend", the D-Bus `LocalSendVisible` property.
    pub localsend: bool,
    /// Device id to its contacts as `(name, numbers)`, the D-Bus `Contacts` property.
    pub contacts: Vec<(String, crate::messages::Contacts)>,
    /// Device id to its newest texts, the D-Bus `Texts` property.
    pub texts: Vec<(String, crate::messages::Texts)>,
    /// Device id to its newest calls, the D-Bus `Calls` property.
    pub calls: Vec<(String, crate::messages::Calls)>,
    /// Device id to why its phone refused the call log, the D-Bus `CallsRefused` property.
    pub calls_refused: Vec<(String, String)>,
    /// A phone mirrors here or presents: the D-Bus `KeepAwake` property.
    pub keep_awake: bool,
    /// `(name, command)`, the D-Bus `Commands` property.
    pub commands: Vec<(String, String)>,
    /// What `link-session` can do now, the D-Bus `SessionCapabilities` property.
    pub capabilities: Vec<String>,
}

#[derive(Debug)]
pub enum Event {
    PairingFinished {
        id: DeviceId,
        name: String,
    },
    PairingFailed {
        reason: String,
    },
    Received {
        id: DeviceId,
        share: Share,
    },
    /// A transfer in either direction, from any backend.
    Transfer(Signal),
    ClipboardOffered {
        from: DeviceId,
        id: u64,
        mimes: Vec<String>,
        size: u64,
    },
    NotificationPosted {
        id: DeviceId,
        posted: NotificationPosted,
        /// Whether the desktop sounds and pops it up, or only replaces the shown one.
        alert: bool,
    },
    NotificationRemoved {
        id: DeviceId,
        notification: String,
    },
    /// The phone asks this desktop to ring, or to stop.
    RingRequested {
        id: DeviceId,
        on: bool,
    },
    PhoneRinging {
        id: DeviceId,
        on: bool,
    },
    Call {
        id: DeviceId,
        call: Call,
    },
    TextReceived {
        id: DeviceId,
        text: Text,
    },
    /// The desktop is joining a phone's hotspot (`ssid`), or left it or failed to join (None).
    Hotspot {
        id: DeviceId,
        ssid: Option<String>,
    },
    /// The phone's touchpad or keyboard.
    RemoteInput {
        id: DeviceId,
        input: RemoteInput,
    },
    /// The phone asks for the desktop's screen, or to switch to `source`.
    RemoteRequested {
        id: DeviceId,
        source: Option<u32>,
    },
    /// Remote control ended on the phone, or the phone left.
    RemoteStopped {
        id: DeviceId,
    },
    /// A command for the desktop from a phone with the system grant; `serial` is its message id, which
    /// `system_result` answers, or 0 for one nobody waits on, as from Do Not Disturb sync.
    SystemCommand {
        id: DeviceId,
        serial: u64,
        command: SystemCommand,
    },
    Pinged {
        id: DeviceId,
        message: Option<String>,
    },
}

/// What any transfer backend reports, as the shell sees it: the Link transfer actor and LocalSend alike.
#[derive(Debug, Clone)]
pub enum Signal {
    /// An offer waiting for consent; `device` is a Link device id or `localsend:<fingerprint>`.
    Offered {
        id: TransferId,
        device: String,
        files: Vec<(String, u64)>,
    },
    Progress {
        id: TransferId,
        bytes: u64,
        total: u64,
    },
    Finished {
        id: TransferId,
        status: TransferStatus,
        paths: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Link,
    LocalSend,
}

enum Command {
    StartPairing { reply: oneshot::Sender<anyhow::Result<(String, String)>> },
    CancelPairing,
    Unpair { id: DeviceId, reply: oneshot::Sender<bool> },
    ConfirmBtPairing { passkey: u32, reply: oneshot::Sender<bool> },
    Admit { spki: Spki, reply: oneshot::Sender<Admission> },
    Paired { spki: Spki, name: String, window: u64 },
    PairingFailed { reason: String, window: u64 },
    ReleaseWindow { window: u64 },
    Connected { id: DeviceId, hello: Hello, session: SessionHandle },
    Disconnected { id: DeviceId, stable_id: usize },
    Session { id: DeviceId, reply: oneshot::Sender<Option<SessionHandle>> },
    SetAutoAccept { id: DeviceId, enabled: bool, reply: oneshot::Sender<bool> },
    SetGrant { id: DeviceId, feature: String, granted: bool, reply: oneshot::Sender<bool> },
    SendFiles { device: String, sources: Vec<Source>, reply: oneshot::Sender<Result<TransferId, link_core::Error>> },
    Decide { id: TransferId, accept: bool, reply: oneshot::Sender<bool> },
    CancelTransfer { id: TransferId, reply: oneshot::Sender<bool> },
    SetLocalSend { visible: bool },
    SetNotifyLocked { on: bool },
    ClipboardPeers { reply: oneshot::Sender<Vec<DeviceId>> },
    DesktopPlayer { player: MediaPlayer },
    DesktopPlayerGone { player: String },
    Browse { id: DeviceId, request: browse::Request, reply: browse::Reply },
    SendText { id: DeviceId, to: String, body: String, reply: crate::messages::SendReply },
    Dial { id: DeviceId, number: String, reply: crate::messages::SendReply },
    TakePhoto { id: DeviceId, reply: crate::messages::PhotoReply },
    OpenMirror { id: DeviceId, reply: crate::mirror::Reply },
    ServeRemote { id: DeviceId, stream: RemoteStream, reply: oneshot::Sender<Result<(), String>> },
    StopRemote { id: DeviceId, reason: String },
    SetCommands { commands: Vec<StoredCommand>, reply: oneshot::Sender<bool> },
    SetCapabilities { owner: String, can: Vec<String> },
    CapabilitiesGone { owner: String },
    SetDesktopStatus { status: DesktopStatus },
    SystemResult { id: DeviceId, serial: u64, error: Option<String> },
    SystemToPhone { id: DeviceId, command: SystemCommand, reply: oneshot::Sender<Result<(), String>> },
    Ping { id: DeviceId, message: Option<String>, reply: oneshot::Sender<Result<(), link_core::Error>> },
}

#[derive(Clone)]
pub struct HubHandle {
    commands: mpsc::Sender<Command>,
    session_events: mpsc::Sender<SessionEvent>,
    transfers: TransferHandle,
}

struct Window {
    id: u64,
    secrets: Arc<Secrets>,
    deadline: Instant,
    taken: bool,
}

pub struct Hub {
    own: Spki,
    store: Store,
    paths: Paths,
    /// Present only while someone could be looking: a paired device or an open pairing window.
    advertiser: Option<Advertiser>,
    window: Option<Window>,
    windows_opened: u64,
    sessions: HashMap<DeviceId, SessionHandle>,
    status: HashMap<DeviceId, Status>,
    notifications: Mirror,
    media: Media,
    /// Phones with a call ringing or active, during which the desktop's players stay paused.
    in_call: HashSet<DeviceId>,
    /// Its own handle, for the exported MPRIS players to reach sessions.
    me: HubHandle,
    commands: mpsc::Receiver<Command>,
    session_events: mpsc::Receiver<SessionEvent>,
    snapshots: watch::Sender<Snapshot>,
    events: mpsc::Sender<Event>,
    transfers: TransferHandle,
    transfer_events: mpsc::UnboundedReceiver<TransferEvent>,
    localsend: LocalSendHandle,
    localsend_signals: mpsc::UnboundedReceiver<Signal>,
    /// Which backend each open transfer belongs to, for consent and cancelling.
    open: HashMap<TransferId, Backend>,
    /// Phones whose hotspot this desktop joined, and the `NetworkManager` connection it joined with.
    hotspots: HashMap<DeviceId, OwnedObjectPath>,
    /// Each join's phone, the hotspot's SSID (None for a Wi-Fi Direct group), and the outcome.
    joining: JoinSet<(DeviceId, Option<String>, Result<hotspot::Joined, String>)>,
    /// Phones whose hotspot this desktop is joining now.
    joining_hotspots: HashSet<DeviceId>,
    /// How phones find this desktop over Wi-Fi Direct, when it can join a group.
    wifi_direct_peer: Option<WifiDirectReady>,
    /// The code a phone with a session reported while Android pairs Bluetooth with it, until when it counts.
    bt_pairing: Option<(u32, Instant)>,
    /// This desktop's Bluetooth adapter, named in the QR code, and the deadline that keeps it open to pairing.
    bluetooth: Option<(String, watch::Sender<Option<Instant>>)>,
    /// Work too large for Bluetooth, waiting for the phone's session to move to its hotspot.
    waiting: Vec<Waiting>,
    /// Phones on Bluetooth already asked for their hotspot by browsing, which does not wait for it.
    hotspot_asked: HashSet<DeviceId>,
    browsing: browse::Browsing,
    /// Each connected phone's `feature:` entries from its hello.
    features: HashMap<DeviceId, Vec<String>>,
    /// The D-Bus unique name of the `link-session` that last reported, and what it can do.
    capabilities: Option<(String, Vec<String>)>,
    /// The desktop's state as `link-session` last reported it.
    desktop_status: Option<DesktopStatus>,
    /// When `desktop_status` last went to phones, and when a change held back by [`DESKTOP_STATUS_EVERY`] goes.
    status_sent: Option<Instant>,
    status_due: Option<Instant>,
    /// `system-command`s sent to phones, by phone and message id, waiting for their `system-result`.
    system_waiting: HashMap<(DeviceId, u64), oneshot::Sender<Result<(), String>>>,
    messages: crate::messages::Messages,
    mirrors: crate::mirror::Mirrors,
    /// Phones whose presenter for this desktop is open.
    presenting: HashSet<DeviceId>,
    remotes: crate::remote::Remotes,
    puncher: Puncher,
}

/// `link-session`'s socket of access units for a phone's remote control, and what it shows.
pub struct RemoteStream {
    pub socket: std::os::fd::OwnedFd,
    pub width: u32,
    pub height: u32,
    pub started: link_core::proto::message::RemoteStarted,
}

struct Waiting {
    peer: DeviceId,
    work: Parked,
    deadline: Instant,
}

enum Parked {
    Send { sources: Vec<Source>, reply: oneshot::Sender<Result<TransferId, link_core::Error>> },
    Mirror(crate::mirror::Reply),
}

/// The transfer backends the hub consents for: the Link transfer actor, which it also attaches sessions to, and
/// LocalSend.
pub struct Transfers {
    pub handle: TransferHandle,
    pub events: mpsc::UnboundedReceiver<TransferEvent>,
    pub localsend: LocalSendHandle,
    pub localsend_signals: mpsc::UnboundedReceiver<Signal>,
}

impl Hub {
    pub fn new(
        own: Spki,
        store: Store,
        paths: Paths,
        desktop_media: DesktopMediaHandle,
        transfers: Transfers,
        puncher: Puncher,
    ) -> (Self, HubHandle, watch::Receiver<Snapshot>, mpsc::Receiver<Event>) {
        let (commands_tx, commands) = mpsc::channel(32);
        let (session_events_tx, session_events) = mpsc::channel(16);
        let handle =
            HubHandle { commands: commands_tx, session_events: session_events_tx, transfers: transfers.handle.clone() };
        let media = Media::new(desktop_media, paths.art.clone());
        let notifications = Mirror::load(paths.notifications.clone());
        let (snapshots, snapshots_rx) = watch::channel(Snapshot::default());
        let (events, events_rx) = mpsc::channel(16);
        let mut hub = Self {
            own,
            store,
            paths,
            advertiser: None,
            window: None,
            windows_opened: 0,
            sessions: HashMap::new(),
            status: HashMap::new(),
            notifications,
            media,
            in_call: HashSet::new(),
            me: handle.clone(),
            commands,
            session_events,
            snapshots,
            events,
            transfers: transfers.handle.clone(),
            transfer_events: transfers.events,
            localsend: transfers.localsend,
            localsend_signals: transfers.localsend_signals,
            open: HashMap::new(),
            hotspots: HashMap::new(),
            joining: JoinSet::new(),
            joining_hotspots: HashSet::new(),
            wifi_direct_peer: None,
            bt_pairing: None,
            bluetooth: None,
            waiting: Vec::new(),
            hotspot_asked: HashSet::new(),
            browsing: browse::Browsing::default(),
            features: HashMap::new(),
            capabilities: None,
            desktop_status: None,
            status_sent: None,
            status_due: None,
            system_waiting: HashMap::new(),
            messages: crate::messages::Messages::default(),
            mirrors: crate::mirror::Mirrors::default(),
            presenting: HashSet::new(),
            remotes: crate::remote::Remotes::default(),
            puncher,
        };
        hub.refresh();
        (hub, handle, snapshots_rx, events_rx)
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        self.wifi_direct_peer = crate::wifi_direct::local_peer().await;
        if self.store.localsend {
            self.localsend.set_visible(true).await;
        }
        loop {
            let deadline = self.window.as_ref().map(|window| window.deadline);
            let give_up = self.waiting.iter().map(|waiting| waiting.deadline).min();
            let browse_deadline = self.browsing.deadline();
            let contacts_deadline = self.messages.deadline();
            let mirror_deadline = self.mirrors.deadline();
            let status_due = self.status_due;
            tokio::select! {
                () = sleep_until(status_due) => self.flush_desktop_status().await,
                () = sleep_until(mirror_deadline) => {
                    for id in self.mirrors.expire() {
                        self.tell_mirror_stop(&id, "nobody allowed it on the phone in time").await;
                    }
                }
                Some(id) = self.mirrors.ended() => {
                    self.tell_mirror_stop(&id, "the viewer closed").await;
                    self.publish();
                }
                Some(id) = self.remotes.ended() => log::debug!("{id}: a remote control stream ended"),
                () = sleep_until(browse_deadline) => self.browsing.expire(),
                () = sleep_until(contacts_deadline) => self.messages.expire(),
                Some(joined) = self.joining.join_next() => match joined {
                    Ok((id, ssid, joined)) => self.hotspot_joined(id, ssid, joined).await,
                    Err(join) => log::warn!("hotspot join task: {join}"),
                },
                () = sleep_until(give_up) => self.expire_waiting().await,
                command = self.commands.recv() => match command {
                    Some(command) => self.handle(command).await,
                    None => return Ok(()),
                },
                Some(event) = self.session_events.recv() => self.on_session_event(event).await,
                Some(event) = self.transfer_events.recv() => self.on_transfer(event).await,
                Some(signal) = self.localsend_signals.recv() => self.on_signal(Backend::LocalSend, signal).await,
                () = sleep_until(deadline) => {
                    log::info!("pairing window expired");
                    self.close_window();
                }
            }
        }
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::StartPairing { reply } => drop(reply.send(self.start_pairing())),
            Command::CancelPairing => self.close_window(),
            Command::ConfirmBtPairing { passkey, reply } => {
                let confirmed =
                    self.bt_pairing.take().is_some_and(|(code, until)| code == passkey && Instant::now() < until);
                let _ = reply.send(confirmed);
            }
            Command::Unpair { id, reply } => {
                let unpaired = self.unpair(&id);
                self.forget_notifications(&id).await;
                self.media.disconnected(&id).await;
                let _ = reply.send(unpaired);
            }
            Command::Admit { spki, reply } => drop(reply.send(self.admit(&spki))),
            Command::Paired { spki, name, window } => self.paired(&spki, name, window).await,
            Command::PairingFailed { reason, window } => {
                if self.close_window_if(window) {
                    self.emit(Event::PairingFailed { reason }).await;
                }
            }
            Command::ReleaseWindow { window } => self.release_window(window),
            Command::Connected { id, hello, session } => self.connected(id, hello, session).await,
            Command::Disconnected { id, stable_id } => self.disconnected(id, stable_id).await,
            Command::SendText { id, to, body, reply } => self.send_text(id, to, body, reply).await,
            Command::Dial { id, number, reply } => self.dial(id, number, reply).await,
            Command::TakePhoto { id, reply } => self.take_photo(id, reply).await,
            Command::Browse { id, request, reply } => self.browse(id, request, reply).await,
            Command::OpenMirror { id, reply } => self.open_mirror(id, reply).await,
            Command::ServeRemote { id, stream, reply } => drop(reply.send(self.serve_remote(id, stream).await)),
            Command::StopRemote { id, reason } => self.stop_remote(&id, &reason).await,
            Command::SetCommands { commands, reply } => drop(reply.send(self.set_commands(commands).await)),
            Command::SetCapabilities { owner, can } => self.set_capabilities(Some((owner, can))).await,
            Command::SetDesktopStatus { status } => self.set_desktop_status(status).await,
            Command::SystemResult { id, serial, error } => self.answer_system(&id, serial, error).await,
            Command::SystemToPhone { id, command, reply } => self.send_system_command(id, command, reply).await,
            Command::CapabilitiesGone { owner } => {
                if self.capabilities.as_ref().is_some_and(|(current, _)| *current == owner) {
                    self.set_capabilities(None).await;
                    // The session that captured the screen is gone.
                    for id in self.remotes.stop_all() {
                        self.tell_remote_stop(&id, "the desktop stopped sharing its screen").await;
                    }
                }
            }
            Command::Ping { id, message, reply } => drop(reply.send(self.ping(&id, message).await)),
            Command::DesktopPlayer { player } => self.media.desktop_player(player, self.sessions.values()),
            Command::DesktopPlayerGone { player } => self.media.desktop_gone(player, self.sessions.values()),
            Command::Session { id, reply } => {
                drop(reply.send(self.sessions.get(&id).filter(|session| session.is_live()).cloned()));
            }
            Command::SetAutoAccept { id, enabled, reply } => {
                let Some(peer) = self.store.peer_mut(&id) else {
                    let _ = reply.send(false);
                    return;
                };
                peer.auto_accept = enabled;
                self.save();
                self.publish();
                let _ = reply.send(true);
            }
            Command::SetGrant { id, feature, granted, reply } => {
                let _ = reply.send(self.set_grant(&id, &feature, granted).await);
            }
            Command::SendFiles { device, sources, reply } => match self.needs_hotspot(&device, &sources) {
                Some(peer) => self.wait_for_hotspot(peer, Parked::Send { sources, reply }).await,
                None => drop(reply.send(self.send_files(&device, sources).await)),
            },
            Command::Decide { id, accept, reply } => {
                let answered = match self.open.get(&id) {
                    Some(Backend::LocalSend) => self.localsend.decide(id, accept).await,
                    _ => self.transfers.decide(id, accept).await.unwrap_or(false),
                };
                let _ = reply.send(answered);
            }
            Command::CancelTransfer { id, reply } => {
                let cancelled = match self.open.get(&id) {
                    Some(Backend::LocalSend) => self.localsend.cancel(id).await,
                    _ => self.transfers.cancel(id).await.unwrap_or(false),
                };
                let _ = reply.send(cancelled);
            }
            Command::SetLocalSend { visible } => {
                self.store.localsend = visible;
                self.save();
                self.publish();
                self.localsend.set_visible(visible).await;
            }
            Command::SetNotifyLocked { on } => {
                self.store.notify_locked = on;
                self.save();
                self.publish();
            }
            Command::ClipboardPeers { reply } => {
                let granted = |id: &DeviceId| self.store.peer(id).is_some_and(|peer| peer.grants.clipboard);
                let peers = self.sessions.iter().filter(|(id, session)| session.is_live() && granted(id));
                drop(reply.send(peers.map(|(id, _)| id.clone()).collect()));
            }
        }
    }

    /// Consent and grants are the hub's: an offer from a device without the files grant is declined, one from an
    /// auto-accept device accepted, any other goes to the shell; a clip from a device without the clipboard grant is
    /// dropped.
    async fn on_transfer(&mut self, event: TransferEvent) {
        let grants = |id: &DeviceId| self.store.peer(id).map(|peer| peer.grants).unwrap_or_default();
        match &event {
            TransferEvent::Busy { .. } => return,
            TransferEvent::Offered { id, from, .. } if !grants(from).files => {
                log::info!("{from}: declining transfer {id}: no files grant");
                if let Err(error) = self.transfers.decide(*id, false).await {
                    log::error!("declining {id}: {error}");
                }
                return;
            }
            TransferEvent::ClipOffered { from, .. } if !grants(from).clipboard => {
                log::info!("{from}: dropping a clipboard offer: no clipboard grant");
                return;
            }
            TransferEvent::Offered { id, from, .. } if self.store.peer(from).is_some_and(|peer| peer.auto_accept) => {
                log::info!("{from}: auto-accepting transfer {id}");
                if let Err(error) = self.transfers.decide(*id, true).await {
                    log::error!("accepting {id}: {error}");
                }
                return;
            }
            _ => {}
        }
        let signal = match event {
            TransferEvent::Offered { id, from, files } => {
                let files = files.into_iter().map(|file| (file.name, file.size)).collect();
                Signal::Offered { id, device: from.to_string(), files }
            }
            TransferEvent::Progress { id, bytes, total } => Signal::Progress { id, bytes, total },
            TransferEvent::Finished { id, status, files, .. } => {
                Signal::Finished { id, status, paths: files.into_iter().map(|file| file.path).collect() }
            }
            TransferEvent::ClipOffered { from, id, mimes, size, .. } => {
                return self.emit(Event::ClipboardOffered { from, id, mimes, size }).await;
            }
            TransferEvent::Busy { .. } => return,
            TransferEvent::Mirror { from, stream } => {
                self.mirrors.stream(from, &stream);
                return self.publish();
            }
        };
        self.on_signal(Backend::Link, signal).await;
    }

    /// Tracks which backend owns each open transfer and passes the signal to D-Bus.
    async fn on_signal(&mut self, backend: Backend, signal: Signal) {
        match &signal {
            Signal::Offered { id, .. } | Signal::Progress { id, .. } => {
                self.open.entry(*id).or_insert(backend);
            }
            Signal::Finished { id, .. } => {
                self.open.remove(id);
            }
        }
        self.emit(Event::Transfer(signal)).await;
    }

    /// A `localsend:<fingerprint>` device goes to LocalSend, anything else to Link.
    async fn send_files(&mut self, device: &str, sources: Vec<Source>) -> Result<TransferId, link_core::Error> {
        if let Some(fingerprint) = device.strip_prefix(crate::localsend::PREFIX) {
            let id = self
                .localsend
                .send(fingerprint.to_owned(), sources)
                .await
                .map_err(|_| link_core::Error::NotConnected)?;
            self.open.insert(id, Backend::LocalSend);
            return Ok(id);
        }
        let id = self.transfers.send(DeviceId::parse(device)?, sources).await?;
        self.open.insert(id, Backend::Link);
        Ok(id)
    }

    async fn on_session_event(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Received { from, share } => {
                if self.sessions.contains_key(&from) {
                    self.emit(Event::Received { id: from, share }).await;
                }
            }
            SessionEvent::Status { from, status } if self.sessions.contains_key(&from) => {
                self.on_status(from, status).await;
            }
            SessionEvent::SystemCommand { from, id, command } if self.sessions.contains_key(&from) => {
                self.on_system_command(from, id, command).await;
            }
            SessionEvent::Status { .. } | SessionEvent::SystemCommand { .. } => {}
            SessionEvent::Unpaired { from } => {
                log::info!("{from} unpaired itself");
                self.sessions.remove(&from);
                self.store.remove(&from);
                self.messages.forget(&from);
                self.save();
                self.refresh();
                self.forget_notifications(&from).await;
                self.media.disconnected(&from).await;
            }
            SessionEvent::Message {
                from,
                message: answer @ (Message::FsEntries(_) | Message::FsData(_) | Message::FsError(_)),
            } => {
                // Answers to this desktop's own requests, which need no grant of the phone's here.
                if let Some(next) = self.browsing.on_answer(&from, answer)
                    && let Some(session) = self.sessions.get(&from)
                    && session.send(next).await.is_err()
                {
                    log::info!("{from}: asking for the next page of a listing failed");
                }
            }
            // The phone's own switch governs its contacts and texts; this desktop asked for them.
            SessionEvent::Message { from, message: Message::Contacts(page) } => {
                let changed = self.messages.on_contacts(&from, page);
                self.messages_changed(from, changed).await;
            }
            SessionEvent::Message { from, message: Message::Texts(page) } => {
                let changed = self.messages.on_texts(&from, page);
                self.messages_changed(from, changed).await;
            }
            SessionEvent::Message { from, message: Message::TextSent(sent) } => self.messages.on_sent(&from, &sent),
            SessionEvent::Message { from, message: Message::Calls(page) } => {
                let changed = self.messages.on_calls(&from, page);
                self.messages_changed(from, changed).await;
            }
            SessionEvent::Message { from, message: Message::Dialed(dialed) } => self.messages.on_dialed(&from, &dialed),
            SessionEvent::Message { from, message: Message::PhotoTaken(taken) } => self.messages.on_photo(&from, taken),
            SessionEvent::Message { from, message: Message::CallsChanged } if self.phone_has(&from, CALLS) => {
                let pull = self.messages.pull_calls(from.clone());
                self.send_to(&from, pull).await;
            }
            SessionEvent::Message { from, message: Message::ContactsChanged } if self.phone_has(&from, MESSAGES) => {
                let pull = self.messages.pull_contacts(from.clone());
                self.send_to(&from, pull).await;
            }
            SessionEvent::Message { from, message: Message::TextsChanged } if self.phone_has(&from, MESSAGES) => {
                let pull = self.messages.pull_texts(from.clone());
                self.send_to(&from, pull).await;
            }
            // Answers to this desktop's own mirror request, which needs no grant of the phone's here.
            SessionEvent::Message { from, message: Message::MirrorStarted(started) } => {
                log::info!("{from}: the phone allowed mirroring at {}x{}", started.width, started.height);
            }
            SessionEvent::Message { from, message: Message::MirrorStop(stop) } => {
                log::info!("{from}: the phone stopped mirroring: {:?}", stop.reason);
                self.mirrors.stopped(&from, stop.reason);
                self.publish();
            }
            SessionEvent::Message { from, message: Message::Punch(punch) } if self.sessions.contains_key(&from) => {
                let addresses: Vec<std::net::SocketAddr> =
                    punch.addresses.iter().filter_map(|text| text.parse().ok()).collect();
                log::info!("{from}: punching toward {addresses:?}");
                self.puncher.punch(&addresses);
            }
            // Connection housekeeping, not a shared feature: any paired phone with a session may pair Bluetooth.
            SessionEvent::Message { from, message: Message::BtPairing(pairing) }
                if self.sessions.contains_key(&from) =>
            {
                log::info!("{from}: pairing Bluetooth");
                let until = Instant::now() + BT_PAIRING_WINDOW;
                self.open_bluetooth_until(until);
                if let Some(passkey) = pairing.passkey {
                    self.bt_pairing = Some((passkey, until));
                }
            }
            SessionEvent::Message { from, message: Message::Ping(ping) } if self.sessions.contains_key(&from) => {
                self.emit(Event::Pinged { id: from, message: ping.message }).await;
            }
            SessionEvent::Message { from, message } => {
                let granted = feature_of(&message)
                    .is_some_and(|feature| self.store.peer(&from).is_some_and(|peer| peer.grants.allows(feature)));
                if !granted {
                    log::info!("{from}: dropped a {} its grant does not allow", message.kind());
                } else if self.sessions.contains_key(&from) {
                    self.on_message(from, message).await;
                }
            }
        }
    }

    async fn on_message(&mut self, from: DeviceId, message: Message) {
        match message {
            Message::NotificationPosted(posted) => {
                if let Some(alert) = self.notifications.post(&from, &posted) {
                    for notification in self.notifications.trim(&from) {
                        self.emit(Event::NotificationRemoved { id: from.clone(), notification }).await;
                    }
                    self.emit(Event::NotificationPosted { id: from, posted, alert }).await;
                    self.publish();
                }
            }
            Message::NotificationSnapshot(snapshot) => {
                let (gone, ids) = self.notifications.snapshot(&from, &snapshot.notifications);
                for notification in gone {
                    self.emit(Event::NotificationRemoved { id: from.clone(), notification }).await;
                }
                if !ids.is_empty() {
                    self.send_to(&from, Message::NotificationPull(NotificationPull { ids })).await;
                }
                self.publish();
            }
            Message::NotificationRemoved(removed) => {
                if self.notifications.remove(&from, &removed.id) {
                    self.emit(Event::NotificationRemoved { id: from, notification: removed.id }).await;
                    self.publish();
                }
            }
            media @ (Message::MediaPlayer(_) | Message::MediaGone(_) | Message::MediaCommand(_)) => {
                let name = self.store.peer(&from).map(|peer| peer.name.clone()).unwrap_or_default();
                self.media.on_phone_message(&self.me, &from, &name, media).await;
            }
            Message::Ring(ring) => self.emit(Event::RingRequested { id: from, on: ring.on }).await,
            Message::Call(call) => {
                self.set_in_call(&from, call.state != CallState::Idle).await;
                self.emit(Event::Call { id: from, call }).await;
            }
            Message::Ringing(ringing) => self.emit(Event::PhoneRinging { id: from, on: ringing.on }).await,
            Message::Hotspot(offered) => self.join_hotspot(from, offered).await,
            Message::WifiDirect(group) => self.join_wifi_direct(from, group),
            Message::HotspotEnd(end) => {
                log::info!("{from}: the phone ended its hotspot: {:?}", end.reason);
                let reason = end.reason.unwrap_or_else(|| "the phone ended its hotspot".to_owned());
                self.fail_waiting(&from, &reason).await;
                self.leave_hotspot(&from).await;
            }
            Message::RemoteInput(input) => self.emit(Event::RemoteInput { id: from, input }).await,
            Message::Presenting(presenting) => {
                if presenting.on {
                    self.presenting.insert(from);
                } else {
                    self.presenting.remove(&from);
                }
                self.publish();
            }
            Message::RemoteRequest(request) => self.remote_request(from, request.source).await,
            Message::RemoteStop(stop) => {
                log::info!("{from}: the phone ended remote control: {:?}", stop.reason);
                self.remotes.stop(&from);
                self.emit(Event::RemoteStopped { id: from }).await;
            }
            Message::SystemResult(result) => {
                if let Some(waiting) = self.system_waiting.remove(&(from, result.command)) {
                    drop(waiting.send(result.error.map_or(Ok(()), Err)));
                }
            }
            Message::RunCommand(run) => {
                match self.store.commands.iter().find(|stored| crate::control::id(stored) == run.id) {
                    Some(stored) => match crate::control::run(stored) {
                        Ok(()) => log::info!("{from}: ran the command {:?}", stored.name),
                        Err(error) => log::warn!("{from}: running the command {:?}: {error}", stored.name),
                    },
                    None => log::info!("{from}: no command {}", run.id),
                }
            }
            other => log::warn!("{from}: a desktop session delivered {}", other.kind()),
        }
    }

    /// Pauses the desktop's players when a first phone's call starts, and resumes them when the last one ends.
    async fn set_in_call(&mut self, id: &DeviceId, calling: bool) {
        let before = self.in_call.is_empty();
        if calling {
            self.in_call.insert(id.clone());
        } else {
            self.in_call.remove(id);
        }
        match (before, self.in_call.is_empty()) {
            (true, false) => self.media.request(Request::PauseAll).await,
            (false, true) => self.media.request(Request::ResumePaused).await,
            _ => {}
        }
    }

    async fn forget_notifications(&mut self, id: &DeviceId) {
        for notification in self.notifications.forget(id) {
            self.emit(Event::NotificationRemoved { id: id.clone(), notification }).await;
        }
        self.publish();
    }

    /// Names the adapter in pairing QR codes and opens it to pairing during each window; before [`Self::run`].
    pub fn set_bluetooth(&mut self, address: String, open: watch::Sender<Option<Instant>>) {
        self.bluetooth = Some((address, open));
    }

    /// Keeps the adapter accepting unpaired devices until at least `until`.
    fn open_bluetooth_until(&self, until: Instant) {
        if let Some((_, open)) = &self.bluetooth {
            open.send_modify(|deadline| *deadline = Some(deadline.map_or(until, |deadline| deadline.max(until))));
        }
    }

    fn start_pairing(&mut self) -> anyhow::Result<(String, String)> {
        let (code, qr) = generate_secrets()?;
        let uri = PairingUri {
            fingerprint: self.own.fingerprint(),
            secret: qr,
            addresses: net::local_addresses(self.store.port),
            bluetooth: self.bluetooth.as_ref().map(|(address, _)| address.clone()),
        };
        self.open_bluetooth_until(Instant::now() + WINDOW);
        let secrets = Secrets { code: Secret::new(code.clone().into_bytes()), qr: Secret::new(qr.to_vec()) };
        self.windows_opened += 1;
        self.window = Some(Window {
            id: self.windows_opened,
            secrets: Arc::new(secrets),
            deadline: Instant::now() + WINDOW,
            taken: false,
        });
        self.refresh();
        log::info!("pairing window open");
        Ok((code, uri.to_string()))
    }

    fn close_window(&mut self) {
        if self.window.take().is_some() {
            self.refresh();
        }
    }

    /// Closes the window only if it is still `id`, so an attempt on a replaced window leaves the new one open.
    fn close_window_if(&mut self, id: u64) -> bool {
        if self.window.as_ref().is_none_or(|window| window.id != id) {
            return false;
        }
        self.close_window();
        true
    }

    fn release_window(&mut self, id: u64) {
        if let Some(window) = self.window.as_mut().filter(|window| window.id == id) {
            window.taken = false;
        }
    }

    fn admit(&mut self, spki: &Spki) -> Admission {
        let id = spki.device_id();
        if let Some(index) = self.store.revoked.iter().position(|revoked| *revoked == id) {
            self.store.revoked.remove(index);
            self.save();
            return Admission::Reject(CloseCode::Unpaired);
        }
        if self.store.peer(&id).is_some() {
            return Admission::Session;
        }
        match &mut self.window {
            Some(window) if window.taken => Admission::Reject(CloseCode::Busy),
            Some(window) => {
                log::info!("{id}: pairing attempt on window {}", window.id);
                window.taken = true;
                Admission::Pair(Attempt { secrets: window.secrets.clone(), window: window.id })
            }
            None => Admission::Reject(CloseCode::NotPaired),
        }
    }

    async fn paired(&mut self, spki: &Spki, name: String, window: u64) {
        let mut peer = Peer::new(spki, name.clone());
        peer.touch();
        let id = peer.id.clone();
        self.store.upsert(peer);
        self.save();
        log::info!("paired {id} ({name:?})");
        if self.close_window_if(window) {
            self.emit(Event::PairingFinished { id, name }).await;
        } else {
            self.refresh();
        }
    }

    /// One session per device: a newer one replaces the older, which a phone that changed networks leaves behind.
    async fn connected(&mut self, id: DeviceId, hello: Hello, session: SessionHandle) {
        let Some(peer) = self.store.peer_mut(&id) else {
            return session.close(CloseCode::NotPaired);
        };
        peer.name = hello.name;
        peer.touch();
        self.save();
        self.media.connected(&session);
        let on_ip = session.connection().quic().is_some();
        match session.connection().quic() {
            Some(quic) => log::info!("{id}: connected over IP from {}", quic.remote_address()),
            None => log::info!("{id}: connected over Bluetooth"),
        }
        if let Some(older) = self.sessions.insert(id.clone(), session.clone()) {
            older.close(CloseCode::Done);
        }
        self.hotspot_asked.remove(&id);
        self.publish();
        let features: Vec<String> =
            hello.addresses.iter().filter(|entry| entry.starts_with(FEATURE)).cloned().collect();
        let clip = ClipPeer {
            granted: self.store.peer(&id).is_some_and(|peer| peer.grants.clipboard),
            items: features.iter().any(|feature| feature == CLIP_ITEMS),
        };
        self.transfers.attach(id.clone(), session.clone(), clip).await;
        self.features.insert(id.clone(), features);
        if self.phone_has(&id, MESSAGES) {
            let contacts = self.messages.pull_contacts(id.clone());
            self.send_to(&id, contacts).await;
            let texts = self.messages.pull_texts(id.clone());
            self.send_to(&id, texts).await;
        }
        if self.phone_has(&id, CALLS) {
            let calls = self.messages.pull_calls(id.clone());
            self.send_to(&id, calls).await;
        }
        self.send_commands(&id).await;
        self.send_capabilities(&id).await;
        self.send_desktop_status(&id).await;
        let by_address = hello.addresses.iter().any(|entry| entry == WIFI_DIRECT_BY_ADDRESS);
        let ready = self
            .wifi_direct_peer
            .clone()
            .map(|peer| WifiDirectReady { address: peer.address.filter(|_| by_address), ..peer });
        if !on_ip
            && let Some(ready) = ready
            && let Err(error) = session.send(Message::WifiDirectReady(ready)).await
        {
            log::info!("{id}: offering Wi-Fi Direct: {error}");
        }
        if on_ip {
            self.send_waiting(&id).await;
        } else {
            // Back on Bluetooth: the phone's hotspot is gone even if its hotspot-end was lost with the old session.
            self.leave_hotspot(&id).await;
        }
    }

    async fn browse(&mut self, id: DeviceId, request: browse::Request, reply: browse::Reply) {
        let Some(session) = self.sessions.get(&id).filter(|session| session.is_live()).cloned() else {
            return drop(reply.send(Err(browse::Failure::NotConnected)));
        };
        if matches!(request, browse::Request::Read { .. }) && session.connection().quic().is_none() {
            self.ask_hotspot_once(&id, &session).await;
        }
        if request.writes() {
            let granted = self.store.peer(&id).is_some_and(|peer| peer.grants.files);
            if !granted || !self.phone_has(&id, FS_WRITE) {
                let reason = if granted { FsRefusal::Denied } else { FsRefusal::NotAllowed };
                return drop(reply.send(Err(browse::Failure::Refused(reason))));
            }
        }
        let request = if self.phone_has(&id, FS_STAT) { Ok(request) } else { request.for_older_phone() };
        let request = match request {
            Ok(request) => request,
            Err(failure) => return drop(reply.send(Err(failure))),
        };
        let (req, message) = self.browsing.start(id, request, reply);
        if session.send(message).await.is_err() {
            self.browsing.fail(req, browse::Failure::NotConnected);
        }
    }

    async fn send_text(&mut self, id: DeviceId, to: String, body: String, reply: crate::messages::SendReply) {
        let Some(session) = self.sessions.get(&id).filter(|session| session.is_live()).cloned() else {
            let _ = reply.send(Err(crate::messages::SendFailure::NotConnected));
            return;
        };
        let (req, message) = self.messages.send_text(id, to, body, reply);
        if session.send(message).await.is_err() {
            self.messages.answer_send(req, Some(crate::messages::SendFailure::NotConnected));
        }
    }

    async fn dial(&mut self, id: DeviceId, number: String, reply: crate::messages::SendReply) {
        let Some(session) = self.sessions.get(&id).filter(|session| session.is_live()).cloned() else {
            let _ = reply.send(Err(crate::messages::SendFailure::NotConnected));
            return;
        };
        let (req, message) = self.messages.dial(id, number, reply);
        if session.send(message).await.is_err() {
            self.messages.answer_send(req, Some(crate::messages::SendFailure::NotConnected));
        }
    }

    async fn take_photo(&mut self, id: DeviceId, reply: crate::messages::PhotoReply) {
        use crate::messages::PhotoFailure;
        let Some(session) = self.sessions.get(&id).filter(|session| session.is_live()).cloned() else {
            let _ = reply.send(Err(PhotoFailure::NotConnected));
            return;
        };
        if !self.phone_has(&id, CAMERA) {
            let _ = reply.send(Err(PhotoFailure::Refused(PhotoRefusal::Denied)));
            return;
        }
        let (req, message) = self.messages.take_photo(id, reply);
        if session.send(message).await.is_err() {
            self.messages.answer_photo(req, Err(PhotoFailure::NotConnected));
        }
    }

    async fn messages_changed(&mut self, from: DeviceId, changed: crate::messages::Changed) {
        if let Some(next) = changed.next {
            self.send_to(&from, next).await;
        }
        if changed.publish {
            self.publish();
        }
        for text in changed.received {
            self.emit(Event::TextReceived { id: from.clone(), text }).await;
        }
    }

    async fn send_to(&self, id: &DeviceId, message: Message) {
        let kind = message.kind();
        if let Some(session) = self.sessions.get(id)
            && let Err(error) = session.send(message).await
        {
            log::info!("{id}: sending {kind}: {error}");
        }
    }

    fn phone_has(&self, id: &DeviceId, feature: &str) -> bool {
        self.features.get(id).is_some_and(|features| features.iter().any(|entry| entry == feature))
    }

    async fn set_grant(&mut self, id: &DeviceId, feature: &str, granted: bool) -> bool {
        let (Some(peer), Some(feature)) = (self.store.peer_mut(id), Feature::parse(feature)) else { return false };
        peer.grants.set(feature, granted);
        self.save();
        self.publish();
        match feature {
            Feature::Commands => self.send_commands(id).await,
            // Tells the phone whether it obeys this desktop's commands, or may send it input.
            Feature::System | Feature::Input => self.send_capabilities(id).await,
            _ => {}
        }
        true
    }

    async fn set_commands(&mut self, commands: Vec<StoredCommand>) -> bool {
        if !crate::control::valid(&commands) {
            return false;
        }
        self.store.commands = commands;
        self.save();
        self.publish();
        for id in self.sessions.keys().cloned().collect::<Vec<_>>() {
            self.send_commands(&id).await;
        }
        true
    }

    async fn ping(&self, id: &DeviceId, message: Option<String>) -> Result<(), link_core::Error> {
        match self.sessions.get(id) {
            Some(_) if !self.phone_has(id, CONTROL) => Err(link_core::Error::Unexpected("ping")),
            Some(session) => session.send(Message::Ping(Ping { message })).await,
            None => Err(link_core::Error::NotConnected),
        }
    }

    fn capabilities(&self) -> &[String] {
        self.capabilities.as_ref().map_or(&[], |(_, can)| can.as_slice())
    }

    async fn set_capabilities(&mut self, capabilities: Option<(String, Vec<String>)>) {
        let changed = self.capabilities().to_vec() != capabilities.as_ref().map_or(&[][..], |(_, can)| can.as_slice());
        self.capabilities = capabilities;
        // A state no session reports any more is not sent to phones that connect later.
        if self.capabilities.is_none() {
            self.desktop_status = None;
        }
        if !changed {
            return;
        }
        self.publish();
        for id in self.sessions.keys().cloned().collect::<Vec<_>>() {
            self.send_capabilities(&id).await;
        }
    }

    async fn send_capabilities(&self, id: &DeviceId) {
        if self.phone_has(id, CAPABILITIES) {
            let mut can = self.capabilities().to_vec();
            // "input" means this phone may use it: the session takes input and the phone has the Input grant.
            if !self.store.peer(id).is_some_and(|peer| peer.grants.input) {
                can.retain(|entry| entry != CAN_INPUT);
            }
            if self.phone_has(id, SYSTEM_CONTROL) && self.store.peer(id).is_some_and(|peer| peer.grants.system) {
                can.push(CAN_CONTROL.to_owned());
            }
            self.send_to(id, Message::Capabilities(Capabilities { can })).await;
        }
    }

    async fn send_desktop_status(&self, id: &DeviceId) {
        let Some(status) = self.desktop_status.clone().filter(|_| self.phone_has(id, SYSTEM)) else { return };
        let status = if self.phone_has(id, SYSTEM_CONTROL) { Some(status) } else { status.without_control() };
        if let Some(status) = status {
            self.send_to(id, Message::DesktopStatus(status)).await;
        }
    }

    async fn set_desktop_status(&mut self, status: DesktopStatus) {
        if self.desktop_status.as_ref() == Some(&status) {
            return;
        }
        let dnd_changed = self.desktop_status.as_ref().and_then(|old| old.dnd) != status.dnd;
        self.desktop_status = Some(status);
        let now = Instant::now();
        match self.status_sent.map(|sent| sent + DESKTOP_STATUS_EVERY).filter(|next| *next > now) {
            Some(next) => self.status_due = Some(next),
            None => self.flush_desktop_status().await,
        }
        if dnd_changed {
            self.sync_dnd_to_phones().await;
        }
    }

    /// Records a phone's status; with Do Not Disturb sync, the phone's new DND goes to `link-session`. On connect the
    /// desktop's DND wins, so a DND set here while the phone was away stays.
    async fn on_status(&mut self, from: DeviceId, status: Status) {
        let Some(before) = self.status.insert(from.clone(), status).map(|old| old.dnd) else {
            self.publish();
            self.sync_dnd_to_phones().await;
            return;
        };
        self.publish();
        if let Some(dnd) = status.dnd
            && before != Some(dnd)
            && self.syncs_dnd(&from)
            && self.desktop_status.as_ref().and_then(|desktop| desktop.dnd) != Some(dnd)
        {
            let command = SystemCommand { command: SystemCommandKind::SetDnd, value: Some(u8::from(dnd)) };
            self.emit(Event::SystemCommand { id: from, serial: 0, command }).await;
        }
    }

    async fn on_system_command(&self, from: DeviceId, id: u64, command: SystemCommand) {
        if !command.command.to_desktop() {
            log::info!("{from}: dropped a {:?} meant for a phone", command.command);
        } else if self.store.peer(&from).is_some_and(|peer| peer.grants.system) {
            self.emit(Event::SystemCommand { id: from, serial: id, command }).await;
        } else {
            log::info!("{from}: dropped a system-command its grant does not allow");
            let error = "Control system is off for this phone on the desktop".to_owned();
            self.answer_system(&from, id, Some(error)).await;
        }
    }

    async fn flush_desktop_status(&mut self) {
        self.status_due = None;
        self.status_sent = Some(Instant::now());
        for id in self.sessions.keys().cloned().collect::<Vec<_>>() {
            self.send_desktop_status(&id).await;
        }
    }

    fn syncs_dnd(&self, id: &DeviceId) -> bool {
        self.phone_has(id, SYSTEM_CONTROL)
            && self.store.peer(id).is_some_and(|peer| peer.grants.system && peer.grants.dnd_sync)
    }

    /// Sends the desktop's Do Not Disturb to the phones that sync it and differ.
    async fn sync_dnd_to_phones(&self) {
        let Some(dnd) = self.desktop_status.as_ref().and_then(|status| status.dnd) else { return };
        for id in self.sessions.keys() {
            if self.syncs_dnd(id) && self.status.get(id).and_then(|status| status.dnd) != Some(dnd) {
                let command = SystemCommand { command: SystemCommandKind::SetDnd, value: Some(u8::from(dnd)) };
                self.send_to(id, Message::SystemCommand(command)).await;
            }
        }
    }

    /// Answers `system-command` `serial` from `id`; 0 is one nobody waits on.
    async fn answer_system(&self, id: &DeviceId, serial: u64, error: Option<String>) {
        if serial != 0 && self.phone_has(id, SYSTEM_CONTROL) {
            let error = error.map(|error| error.chars().take(MAX_SYSTEM_ERROR_LEN).collect());
            self.send_to(id, Message::SystemResult(SystemResult { command: serial, error })).await;
        }
    }

    async fn send_system_command(
        &mut self,
        id: DeviceId,
        command: SystemCommand,
        reply: oneshot::Sender<Result<(), String>>,
    ) {
        let refused = if !self.sessions.contains_key(&id) {
            Some("the phone is not connected")
        } else if !self.phone_has(&id, SYSTEM_CONTROL) {
            Some("the phone's Link app is too old")
        } else if !self.store.peer(&id).is_some_and(|peer| peer.grants.system) {
            Some("Control system is off for this phone")
        } else if !command.command.to_phone() {
            Some("not a command for a phone")
        } else {
            None
        };
        if let Some(refused) = refused {
            drop(reply.send(Err(refused.to_owned())));
            return;
        }
        let Some(session) = self.sessions.get(&id) else { return };
        match session.send_numbered(Message::SystemCommand(command)).await {
            Ok(serial) => {
                self.system_waiting.retain(|_, waiting| !waiting.is_closed());
                self.system_waiting.insert((id, serial), reply);
            }
            Err(error) => drop(reply.send(Err(error.to_string()))),
        }
    }

    /// The commands `id` may run: all of them with its grant, none without.
    async fn send_commands(&self, id: &DeviceId) {
        let (Some(session), Some(peer)) = (self.sessions.get(id), self.store.peer(id)) else { return };
        if !self.phone_has(id, CONTROL) {
            return;
        }
        let granted = if peer.grants.commands { self.store.commands.as_slice() } else { &[] };
        if let Err(error) = session.send(Message::Commands(crate::control::listing(granted))).await {
            log::info!("{id}: sending the commands: {error}");
        }
    }

    /// The Link device a send is for, when its session runs over Bluetooth and the files are worth the hotspot.
    fn needs_hotspot(&self, device: &str, sources: &[Source]) -> Option<DeviceId> {
        let peer = DeviceId::parse(device).ok()?;
        let on_bluetooth = self.sessions.get(&peer)?.connection().quic().is_none();
        (on_bluetooth && total_size(sources) > link_core::stream::BLUETOOTH_UPGRADE_ABOVE).then_some(peer)
    }

    /// Asks the phone for its hotspot and parks the work until the session moves there.
    async fn wait_for_hotspot(&mut self, peer: DeviceId, work: Parked) {
        let asked = match self.sessions.get(&peer) {
            Some(session) => session.send(Message::HotspotRequest).await,
            None => Err(link_core::Error::NotConnected),
        };
        match (asked, work) {
            (Ok(()), work) => {
                let deadline = Instant::now() + link_core::hotspot::UPGRADE_TIMEOUT;
                self.waiting.push(Waiting { peer, work, deadline });
            }
            (Err(error), Parked::Send { reply, .. }) => drop(reply.send(Err(error))),
            (Err(_), Parked::Mirror(reply)) => drop(reply.send(Err(crate::mirror::Failure::NotConnected))),
        }
    }

    /// Browsing goes on over Bluetooth while the phone starts its hotspot, so it asks once per session.
    async fn ask_hotspot_once(&mut self, id: &DeviceId, session: &SessionHandle) {
        if self.hotspots.contains_key(id) || !self.hotspot_asked.insert(id.clone()) {
            return;
        }
        log::info!("{id}: asking for the phone's hotspot to browse");
        if let Err(error) = session.send(Message::HotspotRequest).await {
            log::info!("{id}: asking for the hotspot: {error}");
        }
    }

    async fn send_waiting(&mut self, id: &DeviceId) {
        let (ready, rest) = std::mem::take(&mut self.waiting).into_iter().partition(|waiting| waiting.peer == *id);
        self.waiting = rest;
        for Waiting { peer, work, .. } in ready {
            match work {
                Parked::Send { sources, reply } => drop(reply.send(self.send_files(peer.as_str(), sources).await)),
                Parked::Mirror(reply) => self.open_mirror(peer, reply).await,
            }
        }
    }

    async fn fail_waiting(&mut self, id: &DeviceId, reason: &str) {
        let (failed, rest) = std::mem::take(&mut self.waiting).into_iter().partition(|waiting| waiting.peer == *id);
        self.waiting = rest;
        for waiting in failed {
            self.without_hotspot(waiting, link_core::Error::Hotspot(reason.to_owned())).await;
        }
    }

    async fn expire_waiting(&mut self) {
        let now = Instant::now();
        let (expired, rest) =
            std::mem::take(&mut self.waiting).into_iter().partition(|waiting| waiting.deadline <= now);
        self.waiting = rest;
        for waiting in expired {
            log::info!("{}: the session did not move to the phone's hotspot in time", waiting.peer);
            self.without_hotspot(waiting, link_core::Error::TooLargeForBluetooth).await;
        }
    }

    /// Sends over Bluetooth what fits there, and fails the rest with `error`; mirroring never fits.
    async fn without_hotspot(&mut self, waiting: Waiting, error: link_core::Error) {
        let Waiting { peer, work, .. } = waiting;
        let (sources, reply) = match work {
            Parked::Mirror(reply) => return drop(reply.send(Err(crate::mirror::Failure::NeedsWifi))),
            Parked::Send { sources, reply } => (sources, reply),
        };
        if total_size(&sources) > link_core::stream::BLUETOOTH_FILE_LIMIT {
            return drop(reply.send(Err(error)));
        }
        log::info!("{peer}: sending over Bluetooth: {error}");
        let sent = self.send_files(peer.as_str(), sources).await;
        drop(reply.send(sent));
    }

    /// A session ended; what hung on it goes only if no newer session replaced it.
    async fn disconnected(&mut self, id: DeviceId, stable_id: usize) {
        self.transfers.detach(id.clone(), stable_id).await;
        if self.sessions.get(&id).is_some_and(|live| live.stable_id() == stable_id) {
            self.sessions.remove(&id);
            self.hotspot_asked.remove(&id);
            self.browsing.disconnected(&id);
            self.messages.disconnected(&id);
            self.mirrors.fail(&id, crate::mirror::Failure::NotConnected);
            if self.remotes.running(&id) {
                self.remotes.stop(&id);
                self.emit(Event::RemoteStopped { id: id.clone() }).await;
            }
            self.leave_hotspot(&id).await;
            self.status.remove(&id);
            self.presenting.remove(&id);
            self.publish();
            self.media.disconnected(&id).await;
            self.set_in_call(&id, false).await;
        }
    }

    /// Asks the phone to mirror, only over IP: on Bluetooth, which video does not fit through, the request waits for
    /// the session to move to the phone's hotspot.
    async fn open_mirror(&mut self, id: DeviceId, reply: crate::mirror::Reply) {
        use crate::mirror::Failure;
        let Some(session) = self.sessions.get(&id).filter(|session| session.is_live()).cloned() else {
            return drop(reply.send(Err(Failure::NotConnected)));
        };
        if session.connection().quic().is_none() {
            log::info!("{id}: mirroring waits for the phone's hotspot");
            return self.wait_for_hotspot(id, Parked::Mirror(reply)).await;
        }
        if self.mirrors.busy(&id) {
            return drop(reply.send(Err(Failure::Busy)));
        }
        log::info!("{id}: asking the phone to mirror its screen");
        match session.send(Message::MirrorRequest).await {
            Ok(()) => self.mirrors.wait(id, reply),
            Err(error) => drop(reply.send(Err(Failure::Failed(error.to_string())))),
        }
    }

    async fn tell_mirror_stop(&self, id: &DeviceId, reason: &str) {
        let stop = Message::MirrorStop(link_core::proto::message::MirrorStop { reason: Some(reason.to_owned()) });
        if let Some(session) = self.sessions.get(id)
            && let Err(error) = session.send(stop).await
        {
            log::info!("{id}: telling the phone to stop mirroring: {error}");
        }
    }

    /// Passes a phone's request for the desktop's screen to `link-session`, only over the same network: a Bluetooth
    /// session is too slow, and an overlay network may be far away, where nobody sees the desktop being controlled.
    async fn remote_request(&mut self, id: DeviceId, source: Option<u32>) {
        use link_core::proto::message::CAN_SCREEN;
        let quic = self.sessions.get(&id).and_then(|session| session.connection().quic().cloned());
        let refusal = match quic {
            None => Some("remote control needs the same Wi-Fi as the desktop"),
            Some(quic) if net::is_overlay(quic.remote_address().ip()) => {
                Some("remote control works only on the same Wi-Fi as the desktop")
            }
            Some(_) if !self.capabilities().iter().any(|can| can == CAN_SCREEN) => {
                Some("this desktop cannot share its screen")
            }
            Some(_) => None,
        };
        if let Some(reason) = refusal {
            log::info!("{id}: refusing remote control: {reason}");
            return self.tell_remote_stop(&id, reason).await;
        }
        log::info!("{id}: the phone asks for the screen (source {source:?})");
        self.emit(Event::RemoteRequested { id, source }).await;
    }

    /// Tells the phone what it shows, then opens the video stream and pumps `link-session`'s socket into it.
    async fn serve_remote(&mut self, id: DeviceId, stream: RemoteStream) -> Result<(), String> {
        let session = self.sessions.get(&id).filter(|session| session.is_live()).cloned();
        let session = session.ok_or_else(|| format!("{id} is not connected"))?;
        session.send(Message::RemoteStarted(stream.started)).await.map_err(|error| error.to_string())?;
        let sender = link_core::mirror::MirrorSender::open(&session, stream.width, stream.height)
            .await
            .map_err(|error| error.to_string())?;
        log::info!("{id}: showing the screen at {}x{}", stream.width, stream.height);
        self.remotes.serve(id, stream.socket, sender).map_err(|error| error.to_string())
    }

    async fn stop_remote(&mut self, id: &DeviceId, reason: &str) {
        self.remotes.stop(id);
        self.tell_remote_stop(id, reason).await;
    }

    async fn tell_remote_stop(&self, id: &DeviceId, reason: &str) {
        let stop = Message::RemoteStop(link_core::proto::message::MirrorStop { reason: Some(reason.to_owned()) });
        if let Some(session) = self.sessions.get(id)
            && let Err(error) = session.send(stop).await
        {
            log::info!("{id}: telling the phone to stop remote control: {error}");
        }
    }

    /// Joins the phone's Wi-Fi Direct group off the actor, like a hotspot.
    fn join_wifi_direct(&mut self, id: DeviceId, group: WifiDirect) {
        let on_bluetooth = self.sessions.get(&id).is_some_and(|session| session.connection().quic().is_none());
        if !on_bluetooth || self.hotspots.contains_key(&id) || self.wifi_direct_peer.is_none() {
            return log::info!("{id}: ignoring a Wi-Fi Direct group this session does not need");
        }
        log::info!("{id}: joining the phone's Wi-Fi Direct group as {:?}", group.name);
        self.joining.spawn(async move {
            let joined = crate::wifi_direct::join(&group.name).await;
            (id, None, joined)
        });
    }

    /// Joins the phone's hotspot off the actor; only a Bluetooth session needs one. The shell says so first, since the
    /// laptop leaves its own Wi-Fi.
    async fn join_hotspot(&mut self, id: DeviceId, offered: Hotspot) {
        let on_bluetooth = self.sessions.get(&id).is_some_and(|session| session.connection().quic().is_none());
        if !on_bluetooth {
            return log::info!("{id}: ignoring a hotspot this session does not need");
        }
        // The phone offers its hotspot only after giving up on a group, which this desktop may have joined anyway.
        let abandoned = self.hotspots.remove(&id);
        log::info!("{id}: joining the phone's hotspot {:?}", offered.ssid);
        self.joining_hotspots.insert(id.clone());
        self.emit(Event::Hotspot { id: id.clone(), ssid: Some(offered.ssid.clone()) }).await;
        self.joining.spawn(async move {
            if let Some(group) = abandoned {
                hotspot::leave(&group).await;
            }
            let joined = hotspot::join(&offered.ssid, &offered.passphrase).await;
            (id, Some(offered.ssid), joined)
        });
    }

    /// Answers the phone with where the desktop listens on its hotspot or group. A failed hotspot fails the sends
    /// waiting for it; a failed group does not, since the phone falls back to its hotspot.
    async fn hotspot_joined(&mut self, id: DeviceId, ssid: Option<String>, joined: Result<hotspot::Joined, String>) {
        if ssid.is_some() {
            self.joining_hotspots.remove(&id);
        } else if self.joining_hotspots.contains(&id) || self.hotspots.contains_key(&id) {
            // The phone gave up on its group and runs a hotspot now, which a HotspotEnd would stop.
            log::info!("{id}: dropping the Wi-Fi Direct group the phone gave up on");
            if let Ok(joined) = joined {
                hotspot::leave(&joined.active).await;
            }
            return;
        }
        let Some(session) = self.sessions.get(&id).cloned() else {
            if let Ok(joined) = joined {
                hotspot::leave(&joined.active).await;
            }
            if ssid.is_some() {
                self.emit(Event::Hotspot { id, ssid: None }).await;
            }
            return;
        };
        let reply = match joined {
            Ok(joined) => {
                let address = std::net::SocketAddr::new(joined.address, self.store.port).to_string();
                log::info!("{id}: joined the hotspot at {address}");
                self.hotspots.insert(id.clone(), joined.active);
                Message::HotspotJoined(HotspotJoined { address })
            }
            Err(reason) => {
                log::info!("{id}: joining the hotspot failed: {reason}");
                if ssid.is_some() {
                    self.emit(Event::Hotspot { id: id.clone(), ssid: None }).await;
                    self.fail_waiting(&id, &reason).await;
                }
                let reason = reason.chars().take(link_core::proto::message::MAX_REASON_LEN).collect();
                Message::HotspotEnd(HotspotEnd { reason: Some(reason) })
            }
        };
        if let Err(error) = session.send(reply).await {
            log::info!("{id}: answering the hotspot: {error}");
        }
    }

    async fn leave_hotspot(&mut self, id: &DeviceId) {
        if let Some(active) = self.hotspots.remove(id) {
            hotspot::leave(&active).await;
            self.emit(Event::Hotspot { id: id.clone(), ssid: None }).await;
        }
    }

    fn unpair(&mut self, id: &DeviceId) -> bool {
        if self.store.remove(id).is_none() {
            return false;
        }
        if let Some(session) = self.sessions.remove(id) {
            session.close(CloseCode::Unpaired);
        }
        self.store.revoked.push(id.clone());
        self.messages.forget(id);
        self.save();
        self.refresh();
        true
    }

    /// Publishes the D-Bus snapshot and starts, updates, or stops the mDNS advertisement to match.
    fn refresh(&mut self) {
        self.publish();
        let pairing = self.window.is_some();
        if !pairing && self.store.peers.is_empty() {
            self.advertiser = None;
            return;
        }
        if let Some(advertiser) = &self.advertiser {
            return log_mdns(advertiser.set_pairing(pairing));
        }
        match Advertiser::start(&self.own.device_id(), self.store.port) {
            Ok(advertiser) => {
                log_mdns(advertiser.set_pairing(pairing));
                self.advertiser = Some(advertiser);
            }
            Err(error) => log::warn!("mdns: {error}"),
        }
    }

    fn save(&self) {
        if let Err(error) = self.store.save(&self.paths.devices) {
            log::error!("saving the device store: {error}");
        }
    }

    fn publish(&self) {
        let devices = self
            .store
            .peers
            .iter()
            .map(|peer| (peer.id.to_string(), peer.name.clone(), self.sessions.contains_key(&peer.id)))
            .collect();
        let info = self
            .store
            .peers
            .iter()
            .map(|peer| {
                let transport = self
                    .sessions
                    .get(&peer.id)
                    .map_or("", |session| if session.connection().quic().is_some() { "wifi" } else { "bluetooth" });
                (peer.id.to_string(), i64::try_from(peer.last_seen).unwrap_or(0), transport)
            })
            .collect();
        let auto_accept =
            self.store.peers.iter().filter(|peer| peer.auto_accept).map(|peer| peer.id.to_string()).collect();
        let grants = self.store.peers.iter().map(|peer| (peer.id.to_string(), peer.grants.names())).collect();
        let mut status: Vec<_> = self
            .status
            .iter()
            .map(|(id, status)| {
                let signal = status.signal.map_or(-1, i32::from);
                let value = (u32::from(status.battery), status.charging, status.network.as_str().to_owned(), signal);
                (id.to_string(), value)
            })
            .collect();
        status.sort();
        let flag = |value: Option<bool>| value.map_or(-1, i32::from);
        let mut controls: Vec<_> = self
            .status
            .iter()
            .map(|(id, status)| {
                let ringer = status.ringer.map_or("", Ringer::as_str).to_owned();
                let value = (status.media_volume.map_or(-1, i32::from), ringer, flag(status.dnd), flag(status.locked));
                (id.to_string(), value)
            })
            .collect();
        controls.sort();
        let mut notifications_off: Vec<String> =
            self.status.iter().filter(|(_, status)| status.notifications_off).map(|(id, _)| id.to_string()).collect();
        notifications_off.sort();
        self.snapshots.send_if_modified(|current| {
            let localsend = self.store.localsend;
            let (contacts, texts) = (self.messages.contacts(), self.messages.texts());
            let (calls, calls_refused) = (self.messages.calls(), self.messages.calls_refused());
            let commands =
                self.store.commands.iter().map(|stored| (stored.name.clone(), stored.command.clone())).collect();
            let pairing = self.window.is_some();
            let capabilities = self.capabilities().to_vec();
            let next = Snapshot {
                devices,
                info,
                pairing,
                auto_accept,
                grants,
                status,
                controls,
                notifications_off,
                notification_counts: self.notifications.counts(),
                notify_locked: self.store.notify_locked,
                localsend,
                contacts,
                texts,
                calls,
                calls_refused,
                keep_awake: self.mirrors.active() || !self.presenting.is_empty(),
                commands,
                capabilities,
            };
            let changed = *current != next;
            *current = next;
            changed
        });
    }

    async fn emit(&self, event: Event) {
        if self.events.send(event).await.is_err() {
            log::debug!("no D-Bus forwarder for hub events");
        }
    }
}

fn log_mdns(result: Result<(), link_core::Error>) {
    if let Err(error) = result {
        log::warn!("mdns: {error}");
    }
}

fn total_size(sources: &[Source]) -> u64 {
    sources.iter().map(Source::size).sum()
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// A uniform 6-digit code and a 128-bit QR secret.
fn generate_secrets() -> anyhow::Result<(String, [u8; QR_SECRET_LEN])> {
    const CODES: u32 = 1_000_000;
    let rng = SystemRandom::new();
    let fill = |bytes: &mut [u8]| rng.fill(bytes).map_err(|_| anyhow::anyhow!("system randomness unavailable"));
    let code = loop {
        let mut bytes = [0; 4];
        fill(&mut bytes)?;
        let value = u32::from_be_bytes(bytes);
        if value < u32::MAX - u32::MAX % CODES {
            break value % CODES;
        }
    };
    let mut qr = [0; QR_SECRET_LEN];
    fill(&mut qr)?;
    Ok((format!("{code:06}"), qr))
}

impl HubHandle {
    async fn request<T>(&self, make: impl FnOnce(oneshot::Sender<T>) -> Command) -> Option<T> {
        let (reply, response) = oneshot::channel();
        self.commands.send(make(reply)).await.ok()?;
        response.await.ok()
    }

    async fn tell(&self, command: Command) {
        if self.commands.send(command).await.is_err() {
            log::debug!("hub stopped");
        }
    }

    pub async fn start_pairing(&self) -> anyhow::Result<(String, String)> {
        self.request(|reply| Command::StartPairing { reply })
            .await
            .unwrap_or_else(|| Err(anyhow::anyhow!("hub stopped")))
    }

    pub async fn cancel_pairing(&self) {
        self.tell(Command::CancelPairing).await;
    }

    /// Whether a phone with a session just reported `passkey` for the Bluetooth pairing under way; it counts once.
    pub async fn confirm_bt_pairing(&self, passkey: u32) -> bool {
        self.request(|reply| Command::ConfirmBtPairing { passkey, reply }).await.unwrap_or(false)
    }

    pub async fn unpair(&self, id: DeviceId) -> bool {
        self.request(|reply| Command::Unpair { id, reply }).await.unwrap_or(false)
    }

    pub async fn admit(&self, spki: Spki) -> Admission {
        self.request(|reply| Command::Admit { spki, reply }).await.unwrap_or(Admission::Reject(CloseCode::Busy))
    }

    pub async fn paired(&self, spki: Spki, name: String, window: u64) {
        self.tell(Command::Paired { spki, name, window }).await;
    }

    pub async fn pairing_failed(&self, reason: String, window: u64) {
        self.tell(Command::PairingFailed { reason, window }).await;
    }

    /// Frees window `window` for the next attempt: the last one failed before it could guess the code.
    pub async fn release_window(&self, window: u64) {
        self.tell(Command::ReleaseWindow { window }).await;
    }

    pub async fn connected(&self, id: DeviceId, hello: Hello, session: SessionHandle) {
        self.tell(Command::Connected { id, hello, session }).await;
    }

    /// Asks the phone to mirror its screen; answered once the user allowed it there and the video stream arrived.
    pub async fn open_mirror(&self, id: DeviceId) -> Result<crate::mirror::Opened, crate::mirror::Failure> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Command::OpenMirror { id, reply }).await.is_err() {
            return Err(crate::mirror::Failure::NotConnected);
        }
        answer.await.unwrap_or(Err(crate::mirror::Failure::NotConnected))
    }

    /// Starts or switches a phone's remote control to `stream`.
    pub async fn serve_remote(&self, id: DeviceId, stream: RemoteStream) -> Result<(), String> {
        let (reply, answer) = oneshot::channel();
        self.tell(Command::ServeRemote { id, stream, reply }).await;
        answer.await.unwrap_or_else(|_| Err("the daemon is stopping".to_owned()))
    }

    pub async fn stop_remote(&self, id: DeviceId, reason: String) {
        self.tell(Command::StopRemote { id, reason }).await;
    }

    /// Sends a text through the phone; answered once the phone reports it sent or failed.
    pub async fn send_text(&self, id: DeviceId, to: String, body: String) -> Result<(), crate::messages::SendFailure> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Command::SendText { id, to, body, reply }).await.is_err() {
            return Err(crate::messages::SendFailure::NotConnected);
        }
        answer.await.unwrap_or(Err(crate::messages::SendFailure::NotConnected))
    }

    /// Places a call on the phone; answered once Android took it, or with why not.
    pub async fn dial(&self, id: DeviceId, number: String) -> Result<(), crate::messages::SendFailure> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Command::Dial { id, number, reply }).await.is_err() {
            return Err(crate::messages::SendFailure::NotConnected);
        }
        answer.await.unwrap_or(Err(crate::messages::SendFailure::NotConnected))
    }

    /// Has the phone take a photo; answered with its browse path once its user took it, or with why not.
    pub async fn take_photo(&self, id: DeviceId) -> Result<String, crate::messages::PhotoFailure> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Command::TakePhoto { id, reply }).await.is_err() {
            return Err(crate::messages::PhotoFailure::NotConnected);
        }
        answer.await.unwrap_or(Err(crate::messages::PhotoFailure::NotConnected))
    }

    /// The device's live session, if it has one.
    pub async fn browse(&self, id: DeviceId, request: browse::Request) -> Result<browse::Answer, browse::Failure> {
        let (reply, answer) = oneshot::channel();
        if self.commands.send(Command::Browse { id, request, reply }).await.is_err() {
            return Err(browse::Failure::NotConnected);
        }
        answer.await.unwrap_or(Err(browse::Failure::NotConnected))
    }

    pub async fn session(&self, id: DeviceId) -> Option<SessionHandle> {
        self.request(|reply| Command::Session { id, reply }).await.flatten()
    }

    /// Records what the `link-session` at D-Bus name `owner` can do, and tells connected phones.
    pub async fn set_capabilities(&self, owner: String, can: Vec<String>) {
        self.tell(Command::SetCapabilities { owner, can }).await;
    }

    /// Records the desktop's state and sends it to connected phones when it changed.
    pub async fn set_desktop_status(&self, status: DesktopStatus) {
        self.tell(Command::SetDesktopStatus { status }).await;
    }

    /// Answers phone `id`'s `system-command` `serial`: done, or the error for the user.
    pub async fn system_result(&self, id: DeviceId, serial: u64, error: Option<String>) {
        self.tell(Command::SystemResult { id, serial, error }).await;
    }

    /// Sends phone `id` a command and waits up to [`SYSTEM_TIMEOUT`] for its answer.
    pub async fn send_system_command(&self, id: DeviceId, command: SystemCommand) -> Result<(), String> {
        let answer = self.request(|reply| Command::SystemToPhone { id, command, reply });
        match tokio::time::timeout(SYSTEM_TIMEOUT, answer).await {
            Ok(Some(result)) => result,
            Ok(None) => Err("the phone did not answer".to_owned()),
            Err(_) => Err("the phone did not answer in time".to_owned()),
        }
    }

    /// Forgets the capabilities `owner` reported, when it still holds them.
    pub async fn capabilities_gone(&self, owner: String) {
        self.tell(Command::CapabilitiesGone { owner }).await;
    }

    /// Replaces the commands phones may run; false when the list breaks its limits.
    pub async fn set_commands(&self, commands: Vec<StoredCommand>) -> bool {
        self.request(|reply| Command::SetCommands { commands, reply }).await.unwrap_or(false)
    }

    pub async fn ping(&self, id: DeviceId, message: Option<String>) -> Result<(), link_core::Error> {
        self.request(|reply| Command::Ping { id, message, reply }).await.unwrap_or(Err(link_core::Error::Stopped))
    }

    pub async fn set_grant(&self, id: DeviceId, feature: String, granted: bool) -> bool {
        self.request(|reply| Command::SetGrant { id, feature, granted, reply }).await.unwrap_or(false)
    }

    /// Connected devices holding the clipboard grant.
    pub async fn clipboard_peers(&self) -> Vec<DeviceId> {
        self.request(|reply| Command::ClipboardPeers { reply }).await.unwrap_or_default()
    }

    /// Offers files to a Link device or a `localsend:<fingerprint>` peer.
    pub async fn send_files(&self, device: String, sources: Vec<Source>) -> Result<TransferId, link_core::Error> {
        self.request(|reply| Command::SendFiles { device, sources, reply })
            .await
            .unwrap_or(Err(link_core::Error::Stopped))
    }

    /// Answers an offer from any backend; false when it no longer waits.
    pub async fn decide(&self, id: TransferId, accept: bool) -> bool {
        self.request(|reply| Command::Decide { id, accept, reply }).await.unwrap_or(false)
    }

    pub async fn cancel_transfer(&self, id: TransferId) -> bool {
        self.request(|reply| Command::CancelTransfer { id, reply }).await.unwrap_or(false)
    }

    pub async fn set_localsend(&self, visible: bool) {
        self.tell(Command::SetLocalSend { visible }).await;
    }

    pub async fn set_notify_locked(&self, on: bool) {
        self.tell(Command::SetNotifyLocked { on }).await;
    }

    pub async fn set_auto_accept(&self, id: DeviceId, enabled: bool) -> bool {
        self.request(|reply| Command::SetAutoAccept { id, enabled, reply }).await.unwrap_or(false)
    }

    pub fn transfers(&self) -> &TransferHandle {
        &self.transfers
    }

    /// Where a session delivers what its peer sends.
    pub fn route(&self, peer: DeviceId) -> Route {
        Route { peer, events: self.session_events.clone(), transfers: self.transfers.clone() }
    }

    pub async fn desktop_player(&self, player: MediaPlayer) {
        self.tell(Command::DesktopPlayer { player }).await;
    }

    pub async fn desktop_player_gone(&self, player: String) {
        self.tell(Command::DesktopPlayerGone { player }).await;
    }

    pub async fn disconnected(&self, id: DeviceId, stable_id: usize) {
        self.tell(Command::Disconnected { id, stable_id }).await;
    }
}

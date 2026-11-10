//! Quick Share as a backend of the daemon: findable only while the user makes it visible, and every offer accepted or
//! declined by the user through the shell. D-Bus `org.andromeda.Link1.QuickShare` on the Link object.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt as _;
use link_quickshare::advertise::{Ble, Browser, Mdns, Nearby};
use link_quickshare::endpoint::{DEVICE_LAPTOP, Endpoint};
use link_quickshare::receive::{self, Offer, Outcome, TextKind};
use link_quickshare::send::{self, Hostile, Outgoing, SendOutcome};
use mdns_sd::ServiceDaemon;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;
use zbus::fdo;
use zbus::object_server::SignalEmitter;

use crate::dbus::PATH;

/// Unassigned at IANA, next to Link's 4717/udp; the ufw profile `Andromeda Link` opens it.
const PORT: u16 = 4718;
/// Android gives up on an unanswered offer after about a minute.
const CONSENT_TIMEOUT: Duration = Duration::from_secs(60);
const ADAPTER: &str = "/org/bluez/hci0";
/// An owner that does not call `StartDiscovery` again within this time stops owning discovery.
const DISCOVERY_LIMIT: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub enum Event {
    Offer { id: u64, offer: Offer },
    Finished { id: u64, status: &'static str, files: Vec<PathBuf>, texts: Vec<(TextKind, String)>, error: String },
    SendPin { id: u64, peer: String, pin: String },
    SendFinished { id: u64, status: &'static str, error: String },
}

enum Command {
    SetVisible {
        visible: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Decide {
        id: u64,
        accept: bool,
    },
    /// `owner` is a bus name or a socket client; `on: false` also comes when the owner leaves.
    Discover {
        owner: String,
        on: bool,
    },
    Send {
        peer: String,
        files: Vec<(std::fs::File, String)>,
        reply: oneshot::Sender<Result<u64, String>>,
    },
}

/// While the user picks a device: the mDNS browser, and the BLE hint that makes Android phones advertise.
struct Discovery {
    browser: Browser,
    _ble: Option<Ble>,
}

struct Peer {
    name: String,
    addresses: Vec<std::net::SocketAddr>,
}

#[derive(Clone)]
pub struct Handle(mpsc::Sender<Command>);

struct Visible {
    listener: TcpListener,
    _mdns: Mdns,
    _ble: Option<Ble>,
}

pub struct QuickShare {
    own: Endpoint,
    /// The process's one mDNS responder; absent when it failed to start.
    mdns: Option<ServiceDaemon>,
    downloads: PathBuf,
    marker: PathBuf,
    visible: Option<Visible>,
    visible_tx: watch::Sender<bool>,
    commands: mpsc::Receiver<Command>,
    offers_tx: mpsc::Sender<(u64, Offer, oneshot::Sender<bool>)>,
    offers: mpsc::Receiver<(u64, Offer, oneshot::Sender<bool>)>,
    pending: HashMap<u64, oneshot::Sender<bool>>,
    connections: JoinSet<(u64, link_quickshare::Result<Outcome>)>,
    discovery: Option<Discovery>,
    /// Who wants discovery, and until when; discovery runs while this is not empty.
    owners: HashMap<String, Instant>,
    peers: HashMap<String, Peer>,
    nearby_tx: watch::Sender<Vec<(String, String)>>,
    sends: JoinSet<(u64, link_quickshare::Result<SendOutcome>)>,
    events: mpsc::Sender<Event>,
    next_id: u64,
}

impl QuickShare {
    /// Visibility persists across restarts as a marker file in the state directory.
    pub fn new(
        name: &str,
        state_dir: &Path,
        mdns: Option<ServiceDaemon>,
    ) -> anyhow::Result<(Self, Handle, Watches, mpsc::Receiver<Event>)> {
        let (commands_tx, commands) = mpsc::channel(8);
        let (offers_tx, offers) = mpsc::channel(8);
        let (events, events_rx) = mpsc::channel(16);
        let (visible_tx, visible_rx) = watch::channel(false);
        let (nearby_tx, nearby_rx) = watch::channel(Vec::new());
        let this = Self {
            own: Endpoint::new(name, DEVICE_LAPTOP)?,
            mdns,
            downloads: downloads_dir(),
            marker: state_dir.join("quickshare-visible"),
            visible: None,
            visible_tx,
            commands,
            offers_tx,
            offers,
            pending: HashMap::new(),
            connections: JoinSet::new(),
            discovery: None,
            owners: HashMap::new(),
            peers: HashMap::new(),
            nearby_tx,
            sends: JoinSet::new(),
            events,
            next_id: 1,
        };
        Ok((this, Handle(commands_tx), Watches { visible: visible_rx, nearby: nearby_rx }, events_rx))
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        if self.marker.exists()
            && let Err(error) = self.set_visible(true).await
        {
            log::warn!("quick share: {error}");
        }
        loop {
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(Command::SetVisible { visible, reply }) => drop(reply.send(self.set_visible(visible).await)),
                    Some(Command::Decide { id, accept }) => {
                        if let Some(decide) = self.pending.remove(&id) {
                            // The connection may have ended while the user decided.
                            let _ = decide.send(accept);
                        }
                    }
                    Some(Command::Discover { owner, on }) => self.discover(owner, on).await,
                    Some(Command::Send { peer, files, reply }) => drop(reply.send(self.send(&peer, files))),
                    None => return Ok(()),
                },
                Some(nearby) = next_nearby(self.discovery.as_ref()) => self.nearby(nearby),
                () = until(self.owners.values().min().copied()) => self.expire(),
                Some(joined) = self.sends.join_next() => match joined {
                    Ok((id, outcome)) => {
                        let (status, error) = match outcome {
                            Ok(SendOutcome::Sent) => ("sent", String::new()),
                            Ok(SendOutcome::Rejected(_)) => ("declined", String::new()),
                            Err(error) => ("failed", error.to_string()),
                        };
                        log::info!("quick share: send {id} {status}");
                        self.emit(Event::SendFinished { id, status, error }).await;
                    }
                    Err(error) => log::error!("quick share send task: {error}"),
                },
                accepted = accept(self.visible.as_ref()) => match accepted {
                    Ok((stream, remote)) => self.spawn(stream, remote),
                    Err(error) => log::warn!("quick share accept: {error}"),
                },
                Some((id, offer, decide)) = self.offers.recv() => {
                    self.pending.insert(id, decide);
                    self.emit(Event::Offer { id, offer }).await;
                }
                Some(joined) = self.connections.join_next() => match joined {
                    Ok((id, outcome)) => self.finished(id, outcome).await,
                    Err(error) => log::error!("quick share connection task: {error}"),
                },
            }
        }
    }

    async fn set_visible(&mut self, visible: bool) -> Result<(), String> {
        if visible == self.visible.is_some() {
            return Ok(());
        }
        if visible {
            let listener = match TcpListener::bind(("::", PORT)).await {
                Ok(listener) => listener,
                Err(error) => {
                    log::warn!("quick share port {PORT}: {error}; choosing another");
                    TcpListener::bind(("::", 0)).await.map_err(|e| format!("listening: {e}"))?
                }
            };
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            let daemon = self.mdns.as_ref().ok_or("mDNS is not running")?;
            let mdns = Mdns::advertise(daemon, &self.own, port).map_err(|e| format!("mDNS: {e}"))?;
            // Without the BLE hint Android phones do not look for us, but ones already scanning still find us.
            let ble = Ble::advertise(ADAPTER).await.inspect_err(|e| log::warn!("quick share BLE hint: {e}")).ok();
            log::info!("quick share visible on port {port} as {:?}", self.own.name);
            self.visible = Some(Visible { listener, _mdns: mdns, _ble: ble });
            std::fs::write(&self.marker, b"").map_err(|e| e.to_string())?;
        } else {
            self.visible = None;
            log::info!("quick share hidden");
            if let Err(error) = std::fs::remove_file(&self.marker) {
                log::debug!("quick share marker: {error}");
            }
        }
        self.visible_tx.send_replace(visible);
        Ok(())
    }

    async fn discover(&mut self, owner: String, on: bool) {
        if !on {
            if self.owners.remove(&owner).is_some() && self.owners.is_empty() {
                self.stop_discovery();
            }
            return;
        }
        self.owners.insert(owner, Instant::now() + DISCOVERY_LIMIT);
        if self.discovery.is_some() {
            return;
        }
        let Some(daemon) = &self.mdns else { return log::warn!("quick share discovery: mDNS is not running") };
        match Browser::start(daemon) {
            Ok(browser) => {
                let ble = Ble::advertise(ADAPTER).await.inspect_err(|e| log::warn!("quick share BLE hint: {e}")).ok();
                self.discovery = Some(Discovery { browser, _ble: ble });
            }
            Err(error) => log::warn!("quick share discovery: {error}"),
        }
    }

    fn expire(&mut self) {
        let now = Instant::now();
        self.owners.retain(|_, until| *until > now);
        if self.owners.is_empty() {
            log::info!("quick share discovery: time limit passed");
            self.stop_discovery();
        }
    }

    fn stop_discovery(&mut self) {
        self.discovery = None;
        self.peers.clear();
        self.publish_nearby();
    }

    fn nearby(&mut self, nearby: Nearby) {
        match nearby {
            // Our own advertisement shows up too while visible.
            Nearby::Found { id, .. } if id == self.own.instance_name() => return,
            Nearby::Found { id, name, addresses } => {
                self.peers.insert(id, Peer { name, addresses });
            }
            Nearby::Lost { id } => {
                self.peers.remove(&id);
            }
        }
        self.publish_nearby();
    }

    fn publish_nearby(&self) {
        let mut nearby: Vec<(String, String)> =
            self.peers.iter().map(|(id, peer)| (id.clone(), peer.name.clone())).collect();
        nearby.sort_by(|a, b| a.1.cmp(&b.1));
        self.nearby_tx.send_replace(nearby);
    }

    fn send(&mut self, peer_id: &str, files: Vec<(std::fs::File, String)>) -> Result<u64, String> {
        let peer = self.peers.get(peer_id).ok_or_else(|| "that device is no longer nearby".to_owned())?;
        let addr = peer
            .addresses
            .iter()
            .find(|addr| addr.is_ipv4())
            .or_else(|| peer.addresses.first())
            .copied()
            .ok_or_else(|| "the device has no address".to_owned())?;
        if files.is_empty() {
            return Err("nothing to send".to_owned());
        }
        let id = self.next_id;
        self.next_id += 1;
        let own = self.own.clone();
        let events = self.events.clone();
        let name = peer.name.clone();
        log::info!("quick share: send {id} to {addr}");
        self.sends.spawn(async move {
            let outgoing = files
                .into_iter()
                .map(|(file, name)| Outgoing { name, file: tokio::fs::File::from_std(file) })
                .collect();
            let on_pin = |pin: &str| {
                if events.try_send(Event::SendPin { id, peer: name, pin: pin.to_owned() }).is_err() {
                    log::debug!("quick share: PIN for send {id} dropped");
                }
            };
            (id, send::send(addr, &own, outgoing, &Hostile::default(), on_pin).await)
        });
        Ok(id)
    }

    fn spawn(&mut self, stream: tokio::net::TcpStream, remote: std::net::SocketAddr) {
        let id = self.next_id;
        self.next_id += 1;
        log::info!("quick share: connection {id} from {remote}");
        let offers = self.offers_tx.clone();
        let dir = self.downloads.clone();
        self.connections.spawn(async move {
            let consent = move |offer: Offer| async move {
                let (decide, decided) = oneshot::channel();
                if offers.send((id, offer, decide)).await.is_err() {
                    return false;
                }
                tokio::time::timeout(CONSENT_TIMEOUT, decided).await.ok().and_then(Result::ok).unwrap_or(false)
            };
            (id, receive::receive(stream, &dir, consent).await)
        });
    }

    async fn finished(&mut self, id: u64, outcome: link_quickshare::Result<Outcome>) {
        self.pending.remove(&id);
        let event = match outcome {
            Ok(Outcome::Received { files, texts }) => {
                Event::Finished { id, status: "received", files, texts, error: String::new() }
            }
            Ok(Outcome::Declined) => empty(id, "declined", String::new()),
            Ok(Outcome::Cancelled) => empty(id, "cancelled", String::new()),
            Err(error) => {
                log::info!("quick share: connection {id}: {error}");
                empty(id, "failed", error.to_string())
            }
        };
        self.emit(event).await;
    }

    async fn emit(&self, event: Event) {
        if self.events.send(event).await.is_err() {
            log::debug!("no D-Bus forwarder for quick share events");
        }
    }
}

fn empty(id: u64, status: &'static str, error: String) -> Event {
    Event::Finished { id, status, files: Vec::new(), texts: Vec::new(), error }
}

#[derive(Clone)]
pub struct Watches {
    pub visible: watch::Receiver<bool>,
    pub nearby: watch::Receiver<Vec<(String, String)>>,
}

async fn next_nearby(discovery: Option<&Discovery>) -> Option<Nearby> {
    match discovery {
        Some(discovery) => discovery.browser.next().await,
        None => std::future::pending().await,
    }
}

async fn until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

async fn accept(visible: Option<&Visible>) -> std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)> {
    match visible {
        Some(visible) => visible.listener.accept().await,
        None => std::future::pending().await,
    }
}

/// `XDG_DOWNLOAD_DIR` from user-dirs.dirs, else ~/Downloads.
fn downloads_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/tmp"), PathBuf::from);
    let config = std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
    let configured = std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|dirs| {
        dirs.lines().find_map(|line| {
            let value = line.strip_prefix("XDG_DOWNLOAD_DIR=")?.trim().trim_matches('"');
            Some(PathBuf::from(value.replace("$HOME", &home.to_string_lossy())))
        })
    });
    configured.unwrap_or_else(|| home.join("Downloads"))
}

impl Handle {
    async fn set_visible(&self, visible: bool) -> Result<(), String> {
        let (reply, result) = oneshot::channel();
        self.0.send(Command::SetVisible { visible, reply }).await.map_err(|_| "stopped".to_owned())?;
        result.await.map_err(|_| "stopped".to_owned())?
    }

    pub(crate) async fn discover(&self, owner: String, on: bool) {
        if self.0.send(Command::Discover { owner, on }).await.is_err() {
            log::debug!("quick share stopped");
        }
    }

    async fn send(&self, peer: String, files: Vec<(std::fs::File, String)>) -> Result<u64, String> {
        let (reply, result) = oneshot::channel();
        self.0.send(Command::Send { peer, files, reply }).await.map_err(|_| "stopped".to_owned())?;
        result.await.map_err(|_| "stopped".to_owned())?
    }

    async fn decide(&self, id: u64, accept: bool) {
        if self.0.send(Command::Decide { id, accept }).await.is_err() {
            log::debug!("quick share stopped");
        }
    }
}

pub struct Interface {
    handle: Handle,
    watches: Watches,
    name: String,
    /// The socket client this copy serves, which owns discovery in place of a bus name.
    peer: Option<String>,
    /// Discovery owners whose leaving the bus is watched.
    watched: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

impl Interface {
    pub fn new(handle: Handle, watches: Watches, name: String, peer: Option<String>) -> Self {
        Self { handle, watches, name, peer, watched: std::sync::Arc::default() }
    }
}

#[zbus::interface(name = "org.andromeda.Link1.QuickShare")]
impl Interface {
    async fn accept(&self, id: u64) {
        self.handle.decide(id, true).await;
    }

    async fn decline(&self, id: u64) {
        self.handle.decide(id, false).await;
    }

    /// Looks for nearby receivers, and sends the BLE hint that makes Android phones show themselves, for 2 minutes.
    /// Call again to keep looking; it stops early when every caller calls `StopDiscovery` or leaves the bus.
    async fn start_discovery(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<()> {
        if let Some(peer) = &self.peer {
            self.handle.discover(peer.clone(), true).await;
            return Ok(());
        }
        let owner = header.sender().ok_or_else(|| fdo::Error::Failed("no sender".to_owned()))?.to_owned();
        // One watcher per caller: a caller that renews keeps the one it has.
        if !self.watched.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(owner.to_string()) {
            self.handle.discover(owner.to_string(), true).await;
            return Ok(());
        }
        let bus = fdo::DBusProxy::new(connection).await?;
        // Subscribed before the owner check, so a caller that leaves meanwhile is not missed.
        let mut gone = bus.receive_name_owner_changed_with_args(&[(0, owner.as_str())]).await?;
        self.handle.discover(owner.to_string(), true).await;
        let (handle, watched) = (self.handle.clone(), self.watched.clone());
        let present = bus.name_has_owner(owner.as_ref().into()).await.unwrap_or(false);
        tokio::spawn(async move {
            if present {
                while let Some(changed) = gone.next().await {
                    if changed.args().is_ok_and(|args| args.new_owner().is_none()) {
                        break;
                    }
                }
            }
            watched.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(owner.as_str());
            handle.discover(owner.to_string(), false).await;
        });
        Ok(())
    }

    async fn stop_discovery(&self, #[zbus(header)] header: zbus::message::Header<'_>) {
        let owner = self.peer.clone().or_else(|| header.sender().map(ToString::to_string));
        if let Some(owner) = owner {
            self.handle.discover(owner, false).await;
        }
    }

    /// `files` are (descriptor, name); returns the send's id for `SendPin` and `SendFinished`.
    async fn send(&self, peer: String, files: Vec<(zbus::OwnedFd, String)>) -> fdo::Result<u64> {
        let files =
            files.into_iter().map(|(fd, name)| (std::fs::File::from(std::os::fd::OwnedFd::from(fd)), name)).collect();
        self.handle.send(peer, files).await.map_err(fdo::Error::Failed)
    }

    /// (id, name) of receivers found since `StartDiscovery`.
    #[zbus(property)]
    fn nearby(&self) -> Vec<(String, String)> {
        self.watches.nearby.borrow().clone()
    }

    #[zbus(property)]
    fn visible(&self) -> bool {
        *self.watches.visible.borrow()
    }

    #[zbus(property)]
    async fn set_visible(&mut self, visible: bool) -> fdo::Result<()> {
        self.handle.set_visible(visible).await.map_err(fdo::Error::Failed)
    }

    #[zbus(property)]
    fn name(&self) -> String {
        self.name.clone()
    }

    /// `files` are (name, size); `texts` are the kinds of shared text: text, url, address, phone.
    #[zbus(signal)]
    async fn offer(
        emitter: &SignalEmitter<'_>,
        id: u64,
        sender: &str,
        pin: &str,
        files: Vec<(String, i64)>,
        texts: Vec<String>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn send_pin(emitter: &SignalEmitter<'_>, id: u64, peer: &str, pin: &str) -> zbus::Result<()>;

    /// `status` is sent, declined, or failed.
    #[zbus(signal)]
    async fn send_finished(emitter: &SignalEmitter<'_>, id: u64, status: &str, error: &str) -> zbus::Result<()>;

    /// `status` is received, declined, cancelled, or failed; `texts` are (kind, text).
    #[zbus(signal)]
    async fn finished(
        emitter: &SignalEmitter<'_>,
        id: u64,
        status: &str,
        files: Vec<String>,
        texts: Vec<(String, String)>,
        error: &str,
    ) -> zbus::Result<()>;
}

pub async fn serve(bus: &zbus::Connection, handle: Handle, watches: Watches, name: String) -> anyhow::Result<()> {
    bus.object_server().at(PATH, Interface::new(handle, watches, name, None)).await?;
    Ok(())
}

pub async fn forward(
    bus: &zbus::Connection,
    mut watches: Watches,
    mut events: mpsc::Receiver<Event>,
) -> anyhow::Result<()> {
    let iface = bus.object_server().interface::<_, Interface>(PATH).await?;
    let emitter = iface.signal_emitter();
    loop {
        tokio::select! {
            changed = watches.visible.changed() => {
                changed?;
                iface.get().await.visible_changed(emitter).await?;
            }
            changed = watches.nearby.changed() => {
                changed?;
                iface.get().await.nearby_changed(emitter).await?;
            }
            Some(event) = events.recv() => match event {
                Event::Offer { id, offer } => {
                    let files = offer.files.iter().map(|f| (f.name.clone(), f.size)).collect();
                    let texts = offer.texts.iter().map(|(kind, _)| kind_str(kind).to_owned()).collect();
                    Interface::offer(emitter, id, &offer.sender, &offer.pin, files, texts).await?;
                }
                Event::SendPin { id, peer, pin } => Interface::send_pin(emitter, id, &peer, &pin).await?,
                Event::SendFinished { id, status, error } => {
                    Interface::send_finished(emitter, id, status, &error).await?;
                }
                Event::Finished { id, status, files, texts, error } => {
                    let files = files.iter().map(|p| p.display().to_string()).collect();
                    let texts = texts.into_iter().map(|(kind, text)| (kind_str(&kind).to_owned(), text)).collect();
                    Interface::finished(emitter, id, status, files, texts, &error).await?;
                }
            },
        }
    }
}

fn kind_str(kind: &TextKind) -> &'static str {
    match kind {
        TextKind::Text => "text",
        TextKind::Url => "url",
        TextKind::Address => "address",
        TextKind::Phone => "phone",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Nearby = watch::Receiver<Vec<(String, String)>>;
    const RENEW: Duration = Duration::from_secs(110);

    /// A running Quick Share without mDNS that has found one peer, so a stop shows as an empty `nearby`.
    fn running() -> anyhow::Result<(Handle, Nearby)> {
        let state = std::env::temp_dir().join(format!("link-qs-test-{}", std::process::id()));
        let (mut qs, handle, watches, _events) = QuickShare::new("test", &state, None)?;
        qs.peers.insert("p".to_owned(), Peer { name: "Phone".to_owned(), addresses: Vec::new() });
        qs.publish_nearby();
        tokio::spawn(qs.run());
        Ok((handle, watches.nearby))
    }

    async fn stopped(nearby: &mut Nearby, within: Duration) -> anyhow::Result<()> {
        tokio::time::timeout(within, nearby.wait_for(Vec::is_empty)).await??;
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_stops_at_the_limit_without_renewal() -> anyhow::Result<()> {
        let (handle, mut nearby) = running()?;
        let start = Instant::now();
        handle.discover(":1.7".to_owned(), true).await;
        stopped(&mut nearby, DISCOVERY_LIMIT + Duration::from_secs(1)).await?;
        assert!(start.elapsed() >= DISCOVERY_LIMIT);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_stays_on_while_renewed_and_stops_when_the_owner_leaves() -> anyhow::Result<()> {
        let (handle, mut nearby) = running()?;
        for _ in 0..3 {
            handle.discover(":1.7".to_owned(), true).await;
            tokio::time::sleep(RENEW).await;
            assert!(!nearby.borrow().is_empty());
        }
        handle.discover(":1.7".to_owned(), false).await;
        stopped(&mut nearby, Duration::from_secs(1)).await
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_runs_until_the_last_owner_leaves() -> anyhow::Result<()> {
        let (handle, mut nearby) = running()?;
        handle.discover(":1.7".to_owned(), true).await;
        handle.discover("socket-1".to_owned(), true).await;
        handle.discover(":1.7".to_owned(), false).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!nearby.borrow().is_empty());
        handle.discover("socket-1".to_owned(), false).await;
        stopped(&mut nearby, Duration::from_secs(1)).await
    }
}

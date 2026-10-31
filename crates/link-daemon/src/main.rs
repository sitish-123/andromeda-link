//! `andromeda-linkd`: the desktop side of Link. No UI; clients drive it over `org.andromeda.Link1`, on the session bus
//! or, where there is none, on a Unix socket.

mod bluetooth;
mod browse;
mod control;
mod dbus;
mod desktop_media;
#[cfg_attr(target_os = "macos", path = "hotspot_mac.rs")]
mod hotspot;
mod hub;
mod ifaddrs;
#[cfg(target_os = "linux")]
mod iwd;
mod listener;
mod localsend;
mod low_delay;
mod media;
mod messages;
mod mirror;
mod mpris;
mod notifications;
mod paths;
mod quickshare;
mod remote;
mod socket;
#[cfg_attr(target_os = "macos", path = "wifi_direct_none.rs")]
mod wifi_direct;

use std::net::{Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::Context;
use link_core::identity::Identity;
use link_core::inbox::Inbox;
use link_core::store::Store;
use link_core::stream::StreamAcceptor;
use link_core::transport::Puncher;
use link_core::{transfer, transport};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,zbus=warn,tracing=warn"))
        .format_timestamp(None)
        .init();
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(run())
}

async fn run() -> anyhow::Result<()> {
    let paths = paths::Paths::create()?;
    let identity = Identity::load_or_create(&paths.identity).context("loading the device key")?;
    let mut store = Store::load(&paths.devices).context("loading the device store")?;
    let (endpoint, puncher) = bind(&identity, store.port)?;
    let port = endpoint.local_addr()?.port();
    if store.port != port {
        store.port = port;
        store.save(&paths.devices)?;
    }
    let transport = match link_api::socket_path() {
        Some(path) => Transport::Socket(path),
        None => Transport::Bus(zbus::Connection::session().await.context("connecting to the session bus")?),
    };
    let name = dbus::device_name().await;
    log::info!("{} listening on port {port} as {:?}", identity.device_id(), name);

    let state_dir = paths.state.clone();
    let inbox = Inbox::new(paths.downloads.clone(), &paths.state).context("opening the transfer state")?;
    let (transfers, transfer_actor, transfer_events) = transfer::transfers(inbox);
    let (signals, localsend_signals) = tokio::sync::mpsc::unbounded_channel();
    let (localsend_actor, localsend, nearby) =
        localsend::LocalSend::new(&paths.state, paths.downloads.clone(), name.clone(), signals)?;
    let transfers = hub::Transfers { handle: transfers, events: transfer_events, localsend, localsend_signals };
    let (desktop_media, media_requests) = desktop_media::channel();
    wifi_direct::remove_stale().await;
    let (mut hub, handle, snapshots, events) =
        hub::Hub::new(identity.spki().clone(), store, paths, desktop_media, transfers, puncher);
    let bluetooth = bluetooth::Bluetooth::start().await;
    if let Some(address) = &bluetooth.address {
        hub.set_bluetooth(address.clone(), bluetooth.pairing_window());
    }
    if let Transport::Bus(bus) = &transport {
        dbus::serve(bus, handle.clone(), snapshots.clone(), nearby.clone()).await?;
    }
    let (quick_share, qs_handle, qs_watches, qs_events) = quickshare::QuickShare::new(&name, &state_dir)?;
    if let Transport::Bus(bus) = &transport {
        quickshare::serve(bus, qs_handle.clone(), qs_watches.clone(), name.clone()).await?;
    }
    let api = socket::Api {
        hub: handle.clone(),
        snapshots,
        nearby,
        quick_share: qs_handle,
        quick_share_watches: qs_watches,
        name: name.clone(),
    };
    let acceptor = StreamAcceptor::new(&identity)?;
    let listener =
        listener::Listener::new(endpoint.clone(), bluetooth, acceptor, handle.clone(), identity.spki().clone(), name);
    let mut terminate = signal(SignalKind::terminate())?;
    #[cfg(not(target_os = "linux"))]
    let replaced = std::future::pending::<()>;
    let result = tokio::select! {
        result = hub.run() => result,
        () = replaced() => Err(anyhow::anyhow!("an upgrade replaced andromeda-linkd; exiting to run the new one")),
        () = transfer_actor.run() => Ok(()),
        result = listener.run() => result,
        result = serve_api(transport, api, (events, qs_events), media_requests) => result,
        () = localsend_actor.run() => Ok(()),
        result = quick_share.run() => result,
        _ = terminate.recv() => Ok(()),
        _ = tokio::signal::ctrl_c() => Ok(()),
    };
    endpoint.close(quinn::VarInt::from_u32(0), b"shutdown");
    endpoint.wait_idle().await;
    result
}

enum Transport {
    Bus(zbus::Connection),
    Socket(PathBuf),
}

/// Forwards state and events to the API's clients; desktop media control is MPRIS on the bus, media keys otherwise.
async fn serve_api(
    transport: Transport,
    api: socket::Api,
    (events, qs_events): (mpsc::Receiver<hub::Event>, mpsc::Receiver<quickshare::Event>),
    media_requests: mpsc::Receiver<desktop_media::Request>,
) -> anyhow::Result<()> {
    match transport {
        Transport::Socket(path) => {
            let hub = api.hub.clone();
            tokio::select! {
                result = socket::serve(&path, api, events, qs_events) => result,
                result = desktop_media::now_playing(hub, media_requests) => result,
            }
        }
        Transport::Bus(bus) => tokio::select! {
            result = dbus::forward(&bus, api.snapshots, events, api.nearby) => result,
            result = quickshare::forward(&bus, api.quick_share_watches, qs_events) => result,
            result = desktop_media::run(bus.clone(), api.hub, media_requests) => result,
        },
    }
}

/// Unassigned at IANA; the ufw profile `Andromeda Link` opens it.
const DEFAULT_PORT: u16 = 4717;

/// Binds [`DEFAULT_PORT`], the one the firewall opens; a taken port falls back to a random one for this run only, so
/// the next start returns to the default.
/// [`DEFAULT_PORT`], else the port of the last run, where phones dial first, else any.
fn bind(identity: &Identity, last: u16) -> anyhow::Result<(quinn::Endpoint, Puncher)> {
    let addr = |port| SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    transport::punchable_server_endpoint(identity, addr(DEFAULT_PORT))
        .or_else(|error| {
            log::warn!("port {DEFAULT_PORT}: {error}; choosing another");
            match last {
                0 | DEFAULT_PORT => Err(error),
                last => transport::punchable_server_endpoint(identity, addr(last)),
            }
        })
        .or_else(|_| transport::punchable_server_endpoint(identity, addr(0)))
        .context("binding the QUIC endpoint")
}

/// Resolves when a package upgrade replaced this program's file, so the service manager restarts it into the new one.
#[cfg(target_os = "linux")]
async fn replaced() {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tick.tick().await;
        if std::fs::read_link("/proc/self/exe").is_ok_and(|exe| exe.to_string_lossy().ends_with(" (deleted)")) {
            return;
        }
    }
}

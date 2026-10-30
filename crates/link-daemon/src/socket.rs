//! `org.andromeda.Link1` peer to peer over a Unix socket, where there is no session bus (macOS). Each client gets its
//! own connection with its own copy of the objects, and every signal.

use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::Path;

use anyhow::Context;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::dbus::{self, PATH};
use crate::hub::{self, HubHandle, Snapshot};
use crate::quickshare;

/// Signals queued for one client; a client this far behind loses the newest.
const CLIENT_QUEUE: usize = 256;

/// What each client's connection serves.
#[derive(Clone)]
pub struct Api {
    pub hub: HubHandle,
    pub snapshots: watch::Receiver<Snapshot>,
    pub nearby: watch::Receiver<Vec<(String, String)>>,
    pub quick_share: quickshare::Handle,
    pub quick_share_watches: quickshare::Watches,
    pub name: String,
}

/// Serves `api` on `path` until an error; signals come from `events` and `quick_share_events`.
pub async fn serve(
    path: &Path,
    api: Api,
    mut events: mpsc::Receiver<hub::Event>,
    mut quick_share_events: mpsc::Receiver<quickshare::Event>,
) -> anyhow::Result<()> {
    let listener = bind(path)?;
    log::info!("API on {}", path.display());
    let own_uid = rustix::process::getuid().as_raw();
    let (mut sinks, mut quick_share_sinks) = (Vec::new(), Vec::new());
    let mut clients = JoinSet::new();
    let mut next = 0_u64;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(error) => {
                        log::warn!("accepting an API client: {error}");
                        continue;
                    }
                };
                if !stream.peer_cred().is_ok_and(|cred| cred.uid() == own_uid) {
                    log::warn!("refused an API client of another user");
                    continue;
                }
                next += 1;
                let (sink, client_events) = mpsc::channel(CLIENT_QUEUE);
                let (quick_share_sink, client_quick_share) = mpsc::channel(CLIENT_QUEUE);
                sinks.push(sink);
                quick_share_sinks.push(quick_share_sink);
                clients.spawn(client(stream, format!("socket-{next}"), api.clone(), client_events, client_quick_share));
            }
            Some(event) = events.recv() => broadcast(&mut sinks, &event),
            Some(event) = quick_share_events.recv() => broadcast(&mut quick_share_sinks, &event),
            Some(done) = clients.join_next() => match done {
                Ok(Ok(())) => {}
                Ok(Err(error)) => log::info!("API client: {error:#}"),
                Err(error) => log::warn!("API client task: {error}"),
            },
        }
    }
}

/// Creates the socket's directory owner-only and replaces a socket no daemon answers on.
fn bind(path: &Path) -> anyhow::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    if path.exists() {
        anyhow::ensure!(
            std::os::unix::net::UnixStream::connect(path).is_err(),
            "another andromeda-linkd serves {}",
            path.display()
        );
        std::fs::remove_file(path).with_context(|| format!("removing the stale {}", path.display()))?;
    }
    UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))
}

fn broadcast<E: Clone>(sinks: &mut Vec<mpsc::Sender<E>>, event: &E) {
    sinks.retain(|sink| match sink.try_send(event.clone()) {
        Ok(()) | Err(TrySendError::Full(_)) => true,
        Err(TrySendError::Closed(_)) => false,
    });
}

/// One client's connection, until it closes; then the capabilities it reported are gone.
async fn client(
    stream: UnixStream,
    peer: String,
    api: Api,
    events: mpsc::Receiver<hub::Event>,
    quick_share_events: mpsc::Receiver<quickshare::Event>,
) -> anyhow::Result<()> {
    let link = dbus::Link::new(api.hub.clone(), api.snapshots.clone(), api.nearby.clone(), Some(peer.clone()));
    let quick_share = quickshare::Interface::new(api.quick_share, api.quick_share_watches.clone(), api.name);
    let connection = zbus::connection::Builder::unix_stream(stream.into_std()?)
        .server(zbus::Guid::generate())
        .p2p()
        .serve_at(PATH, link)
        .serve_at(PATH, quick_share)
        .build()
        .await
        .context("the API handshake")?;
    let result = tokio::select! {
        result = dbus::forward(&connection, api.snapshots, events, api.nearby) => result,
        result = quickshare::forward(&connection, api.quick_share_watches, quick_share_events) => result,
        () = connection.closed() => Ok(()),
    };
    api.hub.capabilities_gone(peer).await;
    result
}

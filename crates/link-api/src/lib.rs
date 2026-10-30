//! The typed client of `org.andromeda.Link1`, generated from `api/org.andromeda.Link1.xml`: [`Link1Proxy`],
//! [`QuickShareProxy`] and their signal streams. Clients use only this, never names copied from the XML.

use std::path::{Path, PathBuf};

use zbus::proxy;

pub const SERVICE: &str = "org.andromeda.Link1";
pub const PATH: &str = "/org/andromeda/Link1";
/// Names a Unix socket that carries the API in place of the session bus.
pub const SOCKET_ENV: &str = "ANDROMEDA_LINK_SOCKET";

include!(concat!(env!("OUT_DIR"), "/link1.rs"));

/// The socket `andromeda-linkd` serves the API on: `$ANDROMEDA_LINK_SOCKET`, else on macOS, which has no session bus,
/// `~/Library/Application Support/Andromeda Link/link.sock`. None means the session bus.
pub fn socket_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(SOCKET_ENV) {
        return Some(path.into());
    }
    #[cfg(target_os = "macos")]
    return std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Andromeda Link/link.sock"));
    #[cfg(not(target_os = "macos"))]
    None
}

/// A connection to `andromeda-linkd`: peer to peer over [`socket_path`] when it names one, else the session bus.
/// Over the socket there is no bus driver: no name owners, no activation; the connection closes when the daemon stops.
pub async fn connect() -> zbus::Result<zbus::Connection> {
    match socket_path() {
        Some(path) => connect_socket(&path).await,
        None => zbus::Connection::session().await,
    }
}

/// A peer-to-peer connection to the daemon's socket at `path`.
pub async fn connect_socket(path: &Path) -> zbus::Result<zbus::Connection> {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    zbus::connection::Builder::unix_stream(stream).p2p().build().await
}

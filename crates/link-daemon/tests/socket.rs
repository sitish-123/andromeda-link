//! The API over the daemon's Unix socket, as on macOS: a real `andromeda-linkd` in its own network namespace serves two
//! clients, which each get a method's reply and the property change it causes.
#![expect(clippy::expect_used, reason = "a failed step fails the test")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use link_api::Link1Proxy;

const WAIT: Duration = Duration::from_secs(20);

struct Daemon(Child, PathBuf);

impl Drop for Daemon {
    fn drop(&mut self) {
        drop(self.0.kill());
        drop(self.0.wait());
        drop(std::fs::remove_dir_all(&self.1));
    }
}

fn start(dir: &Path) -> Daemon {
    std::fs::create_dir_all(dir).expect("creating the test directory");
    let nowhere = format!("unix:path={}", dir.join("no-bus").display());
    let child = Command::new("unshare")
        // The client's uid must map to itself, or the D-Bus handshake refuses it.
        .args(["--user", "--map-current-user", "--net"])
        .arg(env!("CARGO_BIN_EXE_andromeda-linkd"))
        .env(link_api::SOCKET_ENV, dir.join("link.sock"))
        .env("XDG_STATE_HOME", dir.join("state"))
        .env("XDG_DOWNLOAD_DIR", dir.join("downloads"))
        .env("DBUS_SESSION_BUS_ADDRESS", &nowhere)
        .env("DBUS_SYSTEM_BUS_ADDRESS", &nowhere)
        .spawn()
        .expect("starting andromeda-linkd");
    Daemon(child, dir.to_owned())
}

async fn pairing_becomes(proxy: &Link1Proxy<'_>, changes: &mut zbus::proxy::PropertyStream<'_, bool>, on: bool) {
    tokio::time::timeout(WAIT, async {
        while let Some(change) = changes.next().await {
            if change.get().await.expect("reading Pairing") == on {
                return;
            }
        }
        panic!("the connection closed");
    })
    .await
    .unwrap_or_else(|_| panic!("no Pairing change to {on}; property reads {:?}", proxy.cached_pairing()));
}

#[tokio::test]
async fn method_reply_and_signal_cross_the_socket() {
    let dir = std::env::temp_dir().join(format!("link-socket-test-{}", std::process::id()));
    let daemon = start(&dir);
    let socket = dir.join("link.sock");
    let deadline = Instant::now() + WAIT;
    let first = loop {
        if let Ok(connection) = link_api::connect_socket(&socket).await {
            break connection;
        }
        assert!(Instant::now() < deadline, "andromeda-linkd serves no socket at {}", socket.display());
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let second = link_api::connect_socket(&socket).await.expect("a second client");
    let caller = Link1Proxy::new(&first).await.expect("the first client's proxy");
    let watcher = Link1Proxy::new(&second).await.expect("the second client's proxy");
    let mut caller_changes = caller.receive_pairing_changed().await;
    let mut watcher_changes = watcher.receive_pairing_changed().await;

    let (code, uri) = caller.start_pairing().await.expect("StartPairing over the socket");
    assert!(!code.is_empty() && !uri.is_empty(), "StartPairing replied ({code:?}, {uri:?})");
    pairing_becomes(&caller, &mut caller_changes, true).await;
    pairing_becomes(&watcher, &mut watcher_changes, true).await;

    watcher.cancel_pairing().await.expect("CancelPairing from the second client");
    pairing_becomes(&caller, &mut caller_changes, false).await;
    drop(daemon);
}

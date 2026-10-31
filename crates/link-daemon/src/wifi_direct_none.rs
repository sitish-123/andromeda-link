//! Wi-Fi Direct needs `NetworkManager`'s Wi-Fi P2P device, so macOS offers none and phones use their hotspot.

use link_core::proto::message::WifiDirectReady;

use crate::hotspot::Joined;

#[expect(clippy::unused_async, reason = "the Linux module's signature")]
pub async fn local_peer() -> Option<WifiDirectReady> {
    None
}

#[expect(clippy::unused_async, reason = "the Linux module's signature")]
pub async fn join(_peer_name: &str) -> Result<Joined, String> {
    Err("no Wi-Fi Direct on this desktop".to_owned())
}

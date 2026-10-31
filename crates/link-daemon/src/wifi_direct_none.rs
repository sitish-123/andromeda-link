//! macOS joins no Wi-Fi Direct group, so its hello leaves out `feature:wifi-direct-join` and phones use their hotspot.

use crate::hotspot::Joined;

pub struct Spare;

#[expect(clippy::unused_async, reason = "the Linux module's signature")]
pub async fn spare() -> Option<Spare> {
    None
}

#[expect(clippy::unused_async, reason = "the Linux module's signature")]
pub async fn remove_stale() {}

#[expect(clippy::unused_async, reason = "the Linux module's signature")]
pub async fn join(_spare: Spare, _ssid: &str, _passphrase: &str) -> Result<Joined, String> {
    Err("no Wi-Fi Direct on this desktop".to_owned())
}

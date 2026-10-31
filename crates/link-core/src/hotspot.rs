//! The phone's hotspot, for moving a Bluetooth session to full speed. See "Hotspot" in `ARCHITECTURE.md`.

use std::time::Duration;

use link_proto::message::Hotspot;

/// A hotspot nothing moved through for this long is stopped.
pub const IDLE: Duration = Duration::from_secs(60);
/// How long a send over the size limit waits for the session to move to the hotspot.
pub const UPGRADE_TIMEOUT: Duration = Duration::from_secs(60);

/// Starts and stops a local-only hotspot: no tethering, no data use. Blocking; the phone calls it off the runtime.
pub trait HotspotProvider: Send + Sync {
    fn start(&self) -> std::io::Result<Hotspot>;
    fn stop(&self);
}

/// How long a Wi-Fi Direct group may take to carry the session before the phone falls back to its hotspot. The desktop
/// may ask its user before it leaves its own Wi-Fi for the group, as for the hotspot.
pub const WIFI_DIRECT_TIMEOUT: Duration = Duration::from_secs(45);

/// Creates a Wi-Fi Direct group the phone owns, which a desktop joins with its credentials. Blocking, like
/// [`HotspotProvider`].
pub trait WifiDirectProvider: Send + Sync {
    fn start(&self) -> std::io::Result<Hotspot>;
    fn stop(&self);
}

//! Joining a phone's hotspot through `CoreWLAN`, for transfers too large for Bluetooth. Leaving disassociates, and
//! macOS then rejoins its usual network by itself.

#![allow(unsafe_code, reason = "FFI to CoreWLAN and getifaddrs")]

use std::net::IpAddr;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_core_wlan::{CWInterface, CWNetwork, CWWiFiClient};
use objc2_foundation::NSString;

use crate::ifaddrs::ipv4_addresses;

/// Joining includes DHCP.
const JOIN_TIMEOUT: Duration = Duration::from_secs(30);
/// A local-only hotspot beacons about 2 s after the phone reports it, and a directed scan can take 10 s more to see it.
const SCAN_TIMEOUT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(250);

/// The BSD name of the Wi-Fi interface that joined, such as `en0`.
pub type Active = String;

/// An active hotspot connection and the address the desktop got on it.
pub struct Joined {
    pub active: Active,
    pub address: IpAddr,
}

pub async fn join(ssid: &str, passphrase: &str) -> Result<Joined, String> {
    let (ssid, passphrase) = (ssid.to_owned(), passphrase.to_owned());
    tokio::task::spawn_blocking(move || join_blocking(&ssid, &passphrase)).await.map_err(|error| error.to_string())?
}

/// `CoreWLAN` scans and associates synchronously, for seconds each.
fn join_blocking(ssid: &str, passphrase: &str) -> Result<Joined, String> {
    // SAFETY: the shared client and its interface are used on this thread only.
    let interface = unsafe { CWWiFiClient::sharedWiFiClient().interface() }.ok_or("no Wi-Fi interface")?;
    // SAFETY: as above.
    let name = unsafe { interface.interfaceName() }.ok_or("no Wi-Fi interface")?.to_string();
    let network = scanned(&interface, ssid)?;
    // The usual network's address can stay on the interface for a moment after the switch.
    let before = ipv4_addresses(&name);
    // SAFETY: `network` came from this interface's scan.
    unsafe { interface.associateToNetwork_password_error(&network, Some(&NSString::from_str(passphrase))) }
        .map_err(|error| format!("macOS could not join it: {}", error.localizedDescription()))?;
    let deadline = Instant::now() + JOIN_TIMEOUT;
    let target = NSString::from_str(ssid);
    loop {
        // SAFETY: as above. Without Location Services the SSID reads as none, so only a different one is refused.
        let on_hotspot = unsafe { interface.ssid() }.is_none_or(|current| *current == *target);
        if let Some(address) = ipv4_addresses(&name).into_iter().find(|address| on_hotspot && !before.contains(address))
        {
            return Ok(Joined { active: name, address: IpAddr::V4(address) });
        }
        if Instant::now() >= deadline {
            // SAFETY: as above.
            unsafe { interface.disassociate() };
            return Err("joining timed out".to_owned());
        }
        std::thread::sleep(POLL);
    }
}

/// The network named `ssid`, rescanning until a scan shows it.
fn scanned(interface: &CWInterface, ssid: &str) -> Result<Retained<CWNetwork>, String> {
    let name = NSString::from_str(ssid);
    let deadline = Instant::now() + SCAN_TIMEOUT;
    while Instant::now() < deadline {
        // SAFETY: the interface is used on this thread only.
        match unsafe { interface.scanForNetworksWithName_error(Some(&name)) } {
            // Without Location Services macOS hides SSIDs, so a network the named scan found with none is taken.
            Ok(networks) => {
                // SAFETY: CWNetwork's ssid is a plain property read.
                if let Some(network) =
                    networks.iter().find(|network| unsafe { network.ssid() }.is_none_or(|seen| *seen == *name))
                {
                    return Ok(network);
                }
            }
            Err(error) => log::debug!("scanning for the hotspot: {}", error.localizedDescription()),
        }
        std::thread::sleep(POLL);
    }
    Err("the hotspot never showed up in a Wi-Fi scan".to_owned())
}

/// Leaves the hotspot.
pub async fn leave(active: &Active) {
    let name = active.clone();
    let left = tokio::task::spawn_blocking(move || {
        // SAFETY: the shared client and the interface are used on this thread only.
        match unsafe { CWWiFiClient::sharedWiFiClient().interfaceWithName(Some(&NSString::from_str(&name))) } {
            // SAFETY: as above.
            Some(interface) => unsafe { interface.disassociate() },
            None => log::info!("leaving the hotspot: no Wi-Fi interface {name}"),
        }
    });
    if let Err(error) = left.await {
        log::info!("leaving the hotspot: {error}");
    }
}

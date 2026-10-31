//! Joining a phone's hotspot through `CoreWLAN`, for transfers too large for Bluetooth. Leaving disassociates, and
//! macOS then rejoins its usual network by itself.

#![allow(unsafe_code, reason = "FFI to CoreWLAN and getifaddrs")]

use std::ffi::CStr;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2_core_wlan::{CWInterface, CWNetwork, CWWiFiClient};
use objc2_foundation::NSString;

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
    loop {
        if let Some(address) = ipv4_addresses(&name).into_iter().find(|address| !before.contains(address)) {
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

/// The interface's routable IPv4 addresses; a self-assigned 169.254 one means DHCP has not answered yet.
fn ipv4_addresses(interface: &str) -> Vec<Ipv4Addr> {
    let mut list = std::ptr::null_mut();
    // SAFETY: `list` receives a list that is freed below.
    if unsafe { libc::getifaddrs(&raw mut list) } != 0 {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut next = list;
    // SAFETY: each entry and its name and address are valid until `freeifaddrs`.
    while let Some(entry) = unsafe { next.as_ref() } {
        next = entry.ifa_next;
        // SAFETY: as above; an AF_INET address is a `sockaddr_in`.
        let address = unsafe {
            if entry.ifa_addr.is_null()
                || i32::from((*entry.ifa_addr).sa_family) != libc::AF_INET
                || CStr::from_ptr(entry.ifa_name).to_bytes() != interface.as_bytes()
            {
                continue;
            }
            Ipv4Addr::from(u32::from_be((*entry.ifa_addr.cast::<libc::sockaddr_in>()).sin_addr.s_addr))
        };
        if !address.is_link_local() {
            found.push(address);
        }
    }
    // SAFETY: `list` came from `getifaddrs`.
    unsafe { libc::freeifaddrs(list) };
    found
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

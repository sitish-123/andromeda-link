//! Joining a phone's hotspot through `NetworkManager`, or iwd without it, for transfers too large for Bluetooth. The profile is volatile:
//! `NetworkManager` deletes it once it deactivates, and autoconnects the desktop's usual network again by itself.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use zbus::{OwnedObjectPath, OwnedVariant, Variant};

pub const NM: &str = "org.freedesktop.NetworkManager";
pub const NM_PATH: &str = "/org/freedesktop/NetworkManager";
/// Joining includes the Wi-Fi scan and DHCP.
const JOIN_TIMEOUT: Duration = Duration::from_secs(30);
/// A local-only hotspot beacons about 2 s after the phone reports it, and a directed scan can take 10 s more to see it.
const SCAN_TIMEOUT: Duration = Duration::from_secs(30);
const DEVICE_TYPE_WIFI: u32 = 2;
pub const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const ACTIVATED: u32 = 2;
const DEACTIVATED: u32 = 4;

/// The `NetworkManager` connection a join activated.
pub enum Active {
    /// A `NetworkManager` active connection.
    Manager(OwnedObjectPath),
    Iwd(crate::iwd::Left),
    /// A `NetworkManager` active connection on an interface added for it, removed on leaving.
    Spare(OwnedObjectPath, String),
}

/// An active hotspot connection and the address the desktop got on it.
pub struct Joined {
    pub active: Active,
    pub address: IpAddr,
}

pub async fn join(ssid: &str, passphrase: &str) -> Result<Joined, String> {
    let bus = zbus::Connection::system().await.map_err(text)?;
    if !manager_runs(&bus).await {
        let (left, address) = crate::iwd::join(&bus, ssid, passphrase, SCAN_TIMEOUT, JOIN_TIMEOUT).await?;
        return Ok(Joined { active: Active::Iwd(left), address });
    }
    let manager = zbus::Proxy::new(&bus, NM, NM_PATH, NM).await.map_err(text)?;
    let device = wifi_device(&bus, &manager).await?;
    join_on(&bus, &device, ssid, passphrase, false).await
}

/// Joins `ssid` on the Wi-Fi `device`, from a random address with `random_address`.
pub async fn join_on(
    bus: &zbus::Connection,
    device: &OwnedObjectPath,
    ssid: &str,
    passphrase: &str,
    random_address: bool,
) -> Result<Joined, String> {
    let manager = zbus::Proxy::new(bus, NM, NM_PATH, NM).await.map_err(text)?;
    let user = std::env::var("USER").unwrap_or_default();
    let mut wireless =
        HashMap::from([("ssid", Variant::from(ssid.as_bytes().to_vec())), ("mode", Variant::from("infrastructure"))]);
    if random_address {
        wireless.insert("cloned-mac-address", Variant::from("random"));
    }
    let settings: HashMap<&str, HashMap<&str, Variant<'_>>> = HashMap::from([
        (
            "connection",
            HashMap::from([
                ("id", Variant::from(format!("Andromeda Link: {ssid}"))),
                ("type", Variant::from("802-11-wireless")),
                ("autoconnect", Variant::from(false)),
                ("permissions", Variant::from(vec![format!("user:{user}")])),
            ]),
        ),
        ("802-11-wireless", wireless),
        (
            "802-11-wireless-security",
            HashMap::from([("key-mgmt", Variant::from("wpa-psk")), ("psk", Variant::from(passphrase))]),
        ),
        ("ipv4", HashMap::from([("method", Variant::from("auto"))])),
        ("ipv6", HashMap::from([("method", Variant::from("ignore"))])),
    ]);
    // iwd joins only networks its last scan saw, and the hotspot is seconds old, so it is scanned for first.
    let access_point = scanned(bus, device, ssid).await?;
    let options: HashMap<&str, Variant<'_>> = HashMap::from([("persist", Variant::from("volatile"))]);
    let (profile, active, _): (OwnedObjectPath, OwnedObjectPath, HashMap<String, OwnedVariant>) =
        manager.call("AddAndActivateConnection2", &(settings, device, &access_point, options)).await.map_err(text)?;
    let joined = activated(bus, active.clone()).await;
    if joined.is_err() {
        abandon(bus, &manager, &active, &profile).await;
    }
    joined
}

/// Deactivates a join that failed and deletes its profile, so the desktop's usual network comes back.
async fn abandon(
    bus: &zbus::Connection,
    manager: &zbus::Proxy<'_>,
    active: &OwnedObjectPath,
    profile: &OwnedObjectPath,
) {
    // Gone already when `NetworkManager` itself gave up on the join.
    if let Err(error) = manager.call::<_, _, ()>("DeactivateConnection", &(active,)).await {
        log::debug!("deactivating the failed join: {error}");
    }
    let deleted = async {
        let settings =
            zbus::Proxy::new(bus, NM, profile.as_str(), "org.freedesktop.NetworkManager.Settings.Connection").await?;
        settings.call::<_, _, ()>("Delete", &()).await
    };
    if let Err(error) = deleted.await {
        log::debug!("deleting the failed join's profile: {error}");
    }
}

/// The access point for `ssid` on `device`, rescanning until a scan shows it.
async fn scanned(bus: &zbus::Connection, device: &OwnedObjectPath, ssid: &str) -> Result<OwnedObjectPath, String> {
    let wireless = zbus::Proxy::new(bus, NM, device.as_str(), WIRELESS).await.map_err(text)?;
    let find = async {
        loop {
            let options: HashMap<&str, Variant<'_>> =
                HashMap::from([("ssids", Variant::from(vec![ssid.as_bytes().to_vec()]))]);
            // A scan already running answers with an error; the next round asks again.
            if let Err(error) = wireless.call::<_, _, ()>("RequestScan", &(options,)).await {
                log::debug!("requesting a Wi-Fi scan: {error}");
            }
            for _ in 0..6 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let points: Vec<OwnedObjectPath> = wireless.call("GetAllAccessPoints", &()).await.map_err(text)?;
                for path in points {
                    // Access points come and go while a scan runs; one gone since the listing is skipped.
                    let Ok(point) =
                        zbus::Proxy::new(bus, NM, path.as_str(), "org.freedesktop.NetworkManager.AccessPoint").await
                    else {
                        continue;
                    };
                    if point.get_property::<Vec<u8>>("Ssid").await.is_ok_and(|seen| seen == ssid.as_bytes()) {
                        return Ok::<_, String>(path);
                    }
                }
            }
        }
    };
    tokio::time::timeout(SCAN_TIMEOUT, find)
        .await
        .map_err(|_| "the hotspot never showed up in a Wi-Fi scan".to_owned())?
}

pub async fn wifi_device(bus: &zbus::Connection, manager: &zbus::Proxy<'_>) -> Result<OwnedObjectPath, String> {
    let devices: Vec<OwnedObjectPath> = manager.get_property("Devices").await.map_err(text)?;
    for path in devices {
        let device =
            zbus::Proxy::new(bus, NM, path.as_str(), "org.freedesktop.NetworkManager.Device").await.map_err(text)?;
        if device.get_property::<u32>("DeviceType").await.map_err(text)? == DEVICE_TYPE_WIFI {
            return Ok(path);
        }
    }
    Err("no Wi-Fi device".to_owned())
}

/// Waits for `active` to come up and reads the address the desktop got on it.
async fn activated(bus: &zbus::Connection, active: OwnedObjectPath) -> Result<Joined, String> {
    let connection = zbus::Proxy::new(bus, NM, active.as_str(), "org.freedesktop.NetworkManager.Connection.Active")
        .await
        .map_err(text)?;
    let activated = async {
        loop {
            match connection.get_property::<u32>("State").await.map_err(text)? {
                ACTIVATED => return Ok(()),
                DEACTIVATED => return Err("NetworkManager could not join it".to_owned()),
                _ => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
    };
    tokio::time::timeout(JOIN_TIMEOUT, activated).await.map_err(|_| "joining timed out".to_owned())??;
    let config: OwnedObjectPath = connection.get_property("Ip4Config").await.map_err(text)?;
    let ip4 =
        zbus::Proxy::new(bus, NM, config.as_str(), "org.freedesktop.NetworkManager.IP4Config").await.map_err(text)?;
    let addresses: Vec<HashMap<String, OwnedVariant>> = ip4.get_property("AddressData").await.map_err(text)?;
    let address = addresses
        .iter()
        .find_map(|entry| String::try_from(entry.get("address")?.clone()).ok()?.parse().ok())
        .ok_or_else(|| "no address on the hotspot".to_owned())?;
    Ok(Joined { active: Active::Manager(active), address })
}

async fn manager_runs(bus: &zbus::Connection) -> bool {
    let (Ok(dbus), Ok(name)) = (zbus::fdo::DBusProxy::new(bus).await, NM.try_into()) else { return false };
    dbus.name_has_owner(name).await.unwrap_or(false)
}

/// Leaves the hotspot; `NetworkManager` then deletes the profile and returns to the usual network.
pub async fn leave(active: &Active) {
    let left = async {
        let bus = zbus::Connection::system().await?;
        match active {
            Active::Manager(active) => {
                let manager = zbus::Proxy::new(&bus, NM, NM_PATH, NM).await?;
                manager.call::<_, _, ()>("DeactivateConnection", &(active,)).await
            }
            Active::Spare(active, interface) => {
                let manager = zbus::Proxy::new(&bus, NM, NM_PATH, NM).await?;
                let left = manager.call::<_, _, ()>("DeactivateConnection", &(active,)).await;
                crate::wifi_direct::remove(interface).await;
                left
            }
            Active::Iwd(left) => {
                crate::iwd::leave(&bus, left).await;
                Ok(())
            }
        }
    };
    if let Err(error) = left.await {
        log::info!("leaving the hotspot: {error}");
    }
}

pub fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

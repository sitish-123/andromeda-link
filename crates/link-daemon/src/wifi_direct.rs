//! Joining a Wi-Fi Direct group a phone owns, as a plain Wi-Fi client with the credentials it sent. `NetworkManager`
//! joins P2P groups only by negotiating (its `Connect` passes `join: false` and a WPS method), which Android answers
//! with a prompt on the phone. The desktop keeps its own Wi-Fi when its Wi-Fi device is idle or when the card runs a
//! second managed interface; otherwise the hub asks the user before leaving it, as for the hotspot.

use std::time::{Duration, Instant};

use zbus::OwnedObjectPath;

use crate::hotspot::{self, Active, Joined, NM, NM_PATH, text};

/// The second managed interface, beside the one that keeps the desktop's own network.
const SPARE: &str = "linkwifi0";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
/// `NM_DEVICE_STATE_DISCONNECTED`: managed and ready to activate.
const READY: u32 = 30;
const READY_TIMEOUT: Duration = Duration::from_secs(10);
/// `iw` answers at once; a hung one must not hold the hub.
const IW_TIMEOUT: Duration = Duration::from_secs(5);

/// Where to join a group without leaving the desktop's network: its idle Wi-Fi device, or an interface added for it.
pub enum Spare {
    Idle(OwnedObjectPath),
    Added(String),
}

/// Where to join a group while the desktop keeps its network; None when only leaving it would do. Quick, since the
/// hub awaits it: [`join`] waits for `NetworkManager` to take an added interface.
pub async fn spare() -> Option<Spare> {
    spare_device().await.inspect_err(|error| log::info!("no spare Wi-Fi interface: {error}")).ok()
}

async fn spare_device() -> Result<Spare, String> {
    let bus = zbus::Connection::system().await.map_err(text)?;
    let manager = zbus::Proxy::new(&bus, NM, NM_PATH, NM).await.map_err(text)?;
    let wifi = hotspot::wifi_device(&bus, &manager).await?;
    let device = zbus::Proxy::new(&bus, NM, wifi.as_str(), DEVICE).await.map_err(text)?;
    let active: OwnedObjectPath = device.get_property("ActiveConnection").await.map_err(text)?;
    if active.as_str() == "/" {
        log::info!("the Wi-Fi device is idle, so the group goes there");
        return Ok(Spare::Idle(wifi));
    }
    let interface: String = device.get_property("Interface").await.map_err(text)?;
    let phy = std::fs::read_to_string(format!("/sys/class/net/{interface}/phy80211/name")).map_err(text)?;
    let info = iw(&["phy", phy.trim(), "info"]).await?;
    if !two_stations(&info) {
        return Err(format!("{} runs one Wi-Fi connection at a time", phy.trim()));
    }
    remove_stale().await;
    iw(&["dev", &interface, "interface", "add", SPARE, "type", "managed"]).await?;
    log::info!("added {SPARE} beside {interface} for the group");
    Ok(Spare::Added(SPARE.to_owned()))
}

/// Joins the group `ssid` on `spare`; a random address keeps a second interface apart from the first.
pub async fn join(spare: Spare, ssid: &str, passphrase: &str) -> Result<Joined, String> {
    let bus = zbus::Connection::system().await.map_err(text)?;
    let added = match spare {
        Spare::Idle(device) => return hotspot::join_on(&bus, &device, ssid, passphrase, false).await,
        Spare::Added(added) => added,
    };
    let joined = async {
        let manager = zbus::Proxy::new(&bus, NM, NM_PATH, NM).await.map_err(text)?;
        let device = managed(&bus, &manager, &added).await?;
        hotspot::join_on(&bus, &device, ssid, passphrase, true).await
    };
    match joined.await {
        Ok(Joined { active: Active::Manager(active), address }) => {
            Ok(Joined { active: Active::Spare(active, added), address })
        }
        Ok(joined) => Ok(joined),
        Err(error) => {
            remove(&added).await;
            Err(error)
        }
    }
}

pub async fn remove(interface: &str) {
    if let Err(error) = iw(&["dev", interface, "del"]).await {
        log::info!("removing {interface}: {error}");
    }
}

/// Removes the spare interface a daemon that died left behind; the hub calls it only while no group of its own uses it.
pub async fn remove_stale() {
    if std::path::Path::new("/sys/class/net").join(SPARE).exists() {
        log::info!("removing a stale {SPARE}");
        remove(SPARE).await;
    }
}

async fn iw(args: &[&str]) -> Result<String, String> {
    let output = tokio::process::Command::new("iw").args(args).kill_on_drop(true).output();
    let output = tokio::time::timeout(IW_TIMEOUT, output).await.map_err(|_| format!("iw {} hung", args.join(" ")))?;
    let output = output.map_err(text)?;
    if !output.status.success() {
        return Err(format!("iw {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The `NetworkManager` device for `interface`, once it manages it.
async fn managed(
    bus: &zbus::Connection,
    manager: &zbus::Proxy<'_>,
    interface: &str,
) -> Result<OwnedObjectPath, String> {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Ok(path) = manager.call::<_, _, OwnedObjectPath>("GetDeviceByIpIface", &(interface,)).await {
            let device = zbus::Proxy::new(bus, NM, path.as_str(), DEVICE).await.map_err(text)?;
            if device.get_property::<u32>("State").await.is_ok_and(|state| state >= READY) {
                return Ok(path);
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(format!("NetworkManager did not take {interface}"))
}

/// Whether `iw phy <phy> info` lists a combination of two managed interfaces on two channels.
fn two_stations(info: &str) -> bool {
    let Some((_, rest)) = info.split_once("valid interface combinations:") else { return false };
    let section = rest.lines().skip(1).take_while(|line| line.starts_with("\t\t")).collect::<Vec<_>>().join(" ");
    let limit = |text: &str| -> Option<u32> {
        let digits = text.trim_start().strip_prefix("<=")?.trim_start();
        digits[..digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len())].parse().ok()
    };
    section.split("* ").any(|combination| {
        let managed: u32 = combination
            .split("#{")
            .skip(1)
            .filter_map(|group| {
                let (kinds, rest) = group.split_once('}')?;
                kinds.split(',').any(|kind| kind.trim() == "managed").then(|| limit(rest)).flatten()
            })
            .sum();
        let channels = combination.split_once("#channels").and_then(|(_, rest)| limit(rest)).unwrap_or(1);
        managed >= 2 && channels >= 2
    })
}

#[cfg(test)]
mod tests {
    use super::two_stations;

    #[test]
    fn finds_two_managed_interfaces_on_two_channels() {
        let intel = "\tvalid interface combinations:\n\t\t * #{ managed } <= 2, #{ P2P-client, P2P-GO } <= 1, \
                     #{ P2P-device } <= 1,\n\t\t   total <= 4, #channels <= 2\n\t\t * #{ managed } <= 2, #{ AP } <= \
                     1,\n\t\t   total <= 4, #channels <= 1\n\tHT Capability overrides:\n";
        assert!(two_stations(intel));
        let single = "\tvalid interface combinations:\n\t\t * #{ managed } <= 1, #{ AP, P2P-client } <= 1,\n\t\t   \
                      total <= 3, #channels <= 2\n\t\t * #{ managed } <= 2,\n\t\t   total <= 2, #channels <= 1\n";
        assert!(!two_stations(single));
    }
}

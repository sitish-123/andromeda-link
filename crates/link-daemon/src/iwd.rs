//! Joining a phone's hotspot through iwd, on desktops without `NetworkManager`. iwd keeps every network it joins, so
//! leaving forgets the hotspot and joins the network the desktop was on before.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use zbus::{OwnedObjectPath, OwnedVariant};

use crate::hotspot::text;
use crate::ifaddrs::ipv4_addresses;

const IWD: &str = "net.connman.iwd";
const STATION: &str = "net.connman.iwd.Station";
const NETWORK: &str = "net.connman.iwd.Network";
const AGENT_PATH: &str = "/org/andromeda/Link1/IwdAgent";
const POLL: Duration = Duration::from_millis(250);

/// What leaving needs: the station, the hotspot it joined, and the network it was on before.
pub struct Left {
    station: OwnedObjectPath,
    hotspot: OwnedObjectPath,
    previous: Option<OwnedObjectPath>,
}

/// Answers iwd's passphrase request for the hotspot while `Connect` runs.
struct Agent {
    passphrase: String,
}

#[zbus::interface(name = "net.connman.iwd.Agent")]
#[expect(
    clippy::unused_self,
    clippy::needless_pass_by_value,
    reason = "iwd's Agent interface; zbus passes arguments by value"
)]
impl Agent {
    fn release(&self) {}

    fn request_passphrase(&self, network: OwnedObjectPath) -> String {
        log::debug!("iwd asks the passphrase of {network}");
        self.passphrase.clone()
    }

    fn cancel(&self, reason: String) {
        log::info!("iwd cancelled the passphrase request: {reason}");
    }
}

pub async fn join(
    bus: &zbus::Connection,
    ssid: &str,
    passphrase: &str,
    scan_timeout: Duration,
    join_timeout: Duration,
) -> Result<(Left, IpAddr), String> {
    let (station, interface) = station(bus).await?;
    let station_proxy = zbus::Proxy::new(bus, IWD, station.as_str(), STATION).await.map_err(text)?;
    let previous = station_proxy.get_property::<OwnedObjectPath>("ConnectedNetwork").await.ok();
    let hotspot = scanned(bus, &station_proxy, ssid, scan_timeout).await?;
    let before = ipv4_addresses(&interface);

    bus.object_server().at(AGENT_PATH, Agent { passphrase: passphrase.to_owned() }).await.map_err(text)?;
    let agents = zbus::Proxy::new(bus, IWD, "/net/connman/iwd", "net.connman.iwd.AgentManager").await.map_err(text)?;
    let agent = OwnedObjectPath::try_from(AGENT_PATH).map_err(text)?;
    let connected = match agents.call::<_, _, ()>("RegisterAgent", &(&agent,)).await {
        Ok(()) => {
            let network = zbus::Proxy::new(bus, IWD, hotspot.as_str(), NETWORK).await.map_err(text)?;
            let connected = network.call::<_, _, ()>("Connect", &()).await.map_err(text);
            let _ = agents.call::<_, _, ()>("UnregisterAgent", &(&agent,)).await;
            connected
        }
        Err(error) => Err(text(error)),
    };
    let _ = bus.object_server().remove::<Agent, _>(AGENT_PATH).await;
    let left = Left { station, hotspot, previous };
    if let Err(error) = connected {
        leave(bus, &left).await;
        return Err(format!("iwd could not join it: {error}"));
    }

    let deadline = Instant::now() + join_timeout;
    loop {
        if let Some(address) = ipv4_addresses(&interface).into_iter().find(|address| !before.contains(address)) {
            return Ok((left, IpAddr::V4(address)));
        }
        if Instant::now() >= deadline {
            leave(bus, &left).await;
            return Err("joining timed out".to_owned());
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Disconnects, forgets the hotspot, and joins the earlier network again.
pub async fn leave(bus: &zbus::Connection, left: &Left) {
    let result = async {
        let station = zbus::Proxy::new(bus, IWD, left.station.as_str(), STATION).await?;
        station.call::<_, _, ()>("Disconnect", &()).await?;
        let hotspot = zbus::Proxy::new(bus, IWD, left.hotspot.as_str(), NETWORK).await?;
        if let Ok(known) = hotspot.get_property::<OwnedObjectPath>("KnownNetwork").await {
            let known = zbus::Proxy::new(bus, IWD, known.as_str(), "net.connman.iwd.KnownNetwork").await?;
            known.call::<_, _, ()>("Forget", &()).await?;
        }
        if let Some(previous) = &left.previous {
            let previous = zbus::Proxy::new(bus, IWD, previous.as_str(), NETWORK).await?;
            previous.call::<_, _, ()>("Connect", &()).await?;
        }
        Ok::<_, zbus::Error>(())
    };
    if let Err(error) = result.await {
        log::info!("leaving the hotspot: {error}");
    }
}

/// The first station and its interface name, such as `wlan0`.
async fn station(bus: &zbus::Connection) -> Result<(OwnedObjectPath, String), String> {
    let manager = zbus::fdo::ObjectManagerProxy::new(bus, IWD, "/").await.map_err(text)?;
    let objects = manager.get_managed_objects().await.map_err(text)?;
    for (path, interfaces) in objects {
        if !interfaces.keys().any(|name| name.as_str() == STATION) {
            continue;
        }
        let device: Option<&HashMap<String, OwnedVariant>> =
            interfaces.iter().find(|(name, _)| name.as_str() == "net.connman.iwd.Device").map(|(_, props)| props);
        let name = device.and_then(|props| String::try_from(props.get("Name")?.try_clone().ok()?).ok());
        if let Some(name) = name {
            return Ok((path, name));
        }
    }
    Err("no Wi-Fi station in iwd".to_owned())
}

/// The network named `ssid`, rescanning until a scan shows it.
async fn scanned(
    bus: &zbus::Connection,
    station: &zbus::Proxy<'_>,
    ssid: &str,
    timeout: Duration,
) -> Result<OwnedObjectPath, String> {
    let find = async {
        loop {
            // A scan already running answers with an error; the next round asks again.
            if let Err(error) = station.call::<_, _, ()>("Scan", &()).await {
                log::debug!("requesting a Wi-Fi scan: {error}");
            }
            for _ in 0..6 {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let networks: Vec<(OwnedObjectPath, i16)> =
                    station.call("GetOrderedNetworks", &()).await.map_err(text)?;
                for (path, _) in networks {
                    let Ok(network) = zbus::Proxy::new(bus, IWD, path.as_str(), NETWORK).await else { continue };
                    if network.get_property::<String>("Name").await.is_ok_and(|name| name == ssid) {
                        return Ok::<_, String>(path);
                    }
                }
            }
        }
    };
    tokio::time::timeout(timeout, find).await.map_err(|_| "the hotspot never showed up in a Wi-Fi scan".to_owned())?
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "a failed step fails the test")]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    use super::*;

    /// A failed join leaves the desktop where it was: the hotspot forgotten and the earlier network joined again.
    #[tokio::test]
    async fn a_join_that_gets_no_address_forgets_the_hotspot_and_rejoins_home() {
        let mut daemon = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().expect("stdout")).read_line(&mut address).expect("address");
        let address = address.trim().to_owned();
        let mut mock = Command::new("python3")
            .args([concat!(env!("CARGO_MANIFEST_DIR"), "/tests/iwd_mock.py"), &address])
            .stdout(Stdio::piped())
            .spawn()
            .expect("iwd mock");
        let mut calls = BufReader::new(mock.stdout.take().expect("stdout")).lines();
        assert_eq!(calls.next().expect("a line").expect("ready"), "ready");

        let bus = zbus::connection::Builder::address(address.as_str()).build().await.expect("bus");
        let joined = join(&bus, "Phone-AP", "secret", Duration::from_secs(5), Duration::from_secs(1)).await;
        for child in [&mut mock, &mut daemon] {
            drop(child.kill());
            drop(child.wait());
        }

        assert_eq!(joined.err().as_deref(), Some("joining timed out"));
        let calls: Vec<String> = calls.map_while(Result::ok).filter(|line| !line.starts_with("Scan")).collect();
        let phone = "/net/connman/iwd/0/3/50686f6e652d4150_psk";
        assert_eq!(
            calls,
            [
                format!("RegisterAgent {AGENT_PATH}"),
                format!("Connect {phone} secret"),
                format!("UnregisterAgent {AGENT_PATH}"),
                "Disconnect /net/connman/iwd/0/3".to_owned(),
                "Forget /net/connman/iwd/50686f6e652d4150_psk".to_owned(),
                "Connect /net/connman/iwd/0/3/486f6d65_psk".to_owned(),
            ]
        );
    }
}

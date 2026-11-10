//! mDNS/DNS-SD: the desktop advertises `_andromeda-link._udp`, the phone browses for it.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use mdns_sd::{IfKind, IfPredicate, ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::Error;
use crate::identity::DeviceId;

pub const SERVICE_TYPE: &str = "_andromeda-link._udp.local.";

/// The fallback for address changes that no [`recheck_interfaces`] call reports.
const IP_CHECK_SECS: u32 = 60;

/// Makes `daemon` take up interface addresses added since its last check and announce its services on them now.
pub fn recheck_interfaces(daemon: &ServiceDaemon) -> Result<(), Error> {
    // An empty list adds no selection; the crate still applies the existing ones to the current addresses.
    Ok(daemon.enable_interface(Vec::<IfKind>::new())?)
}

/// The one mDNS responder a long-running process shares: physical interfaces only, interfaces rechecked every minute.
pub fn responder() -> Result<ServiceDaemon, Error> {
    let daemon = ServiceDaemon::new()?;
    daemon.set_ip_check_interval(IP_CHECK_SECS)?;
    daemon
        .disable_interface(IfKind::Predicate(IfPredicate::new(|intf| intf.is_loopback() || is_virtual(&intf.name))))?;
    Ok(daemon)
}

/// Container, VM and VPN links, where no phone listens. A plain bridge (`br0`, `br-lan`) can be the LAN.
fn is_virtual(name: &str) -> bool {
    const PREFIXES: [&str; 14] = [
        "veth",
        "docker",
        "virbr",
        "ve-",
        "vb-",
        "podman",
        "cni-",
        "lxcbr",
        "lxdbr",
        "tun",
        "tap",
        "wg",
        "tailscale",
        "zt",
    ];
    // Docker names a user network's bridge br-<first 12 hex digits of the network id>.
    let docker_bridge =
        name.strip_prefix("br-").is_some_and(|id| id.len() == 12 && id.bytes().all(|b| b.is_ascii_hexdigit()));
    docker_bridge || PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// A registered advertisement on a shared responder; dropping it sends the goodbye.
pub struct Advertiser {
    daemon: ServiceDaemon,
    id: DeviceId,
    port: u16,
    fullname: String,
    pairing: bool,
}

#[derive(Debug, Clone)]
pub struct Found {
    pub id: DeviceId,
    pub pairing: bool,
    pub addresses: Vec<SocketAddr>,
}

impl Advertiser {
    pub fn start(daemon: &ServiceDaemon, id: &DeviceId, port: u16, pairing: bool) -> Result<Self, Error> {
        let fullname = register(daemon, id, port, pairing)?;
        Ok(Self { daemon: daemon.clone(), id: id.clone(), port, fullname, pairing })
    }

    /// Re-announces with `pair=1` while a pairing window is open; an unchanged state sends nothing.
    pub fn set_pairing(&mut self, open: bool) -> Result<(), Error> {
        if open != self.pairing {
            register(&self.daemon, &self.id, self.port, open)?;
            self.pairing = open;
        }
        Ok(())
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        if let Err(error) = self.daemon.unregister(&self.fullname) {
            log::debug!("mdns unregister: {error}");
        }
    }
}

fn register(daemon: &ServiceDaemon, id: &DeviceId, port: u16, pairing: bool) -> Result<String, Error> {
    let mut properties = vec![("v", "1")];
    if pairing {
        properties.push(("pair", "1"));
    }
    let host = format!("{id}.local.");
    let info = ServiceInfo::new(SERVICE_TYPE, id.as_str(), &host, "", port, &properties[..])?.enable_addr_auto();
    let fullname = info.get_fullname().to_owned();
    daemon.register(info)?;
    Ok(fullname)
}

/// Browses for up to `window`, returning early once `want` resolves.
pub async fn browse(window: Duration, want: Option<&DeviceId>) -> Result<Vec<Found>, Error> {
    let daemon = ServiceDaemon::new()?;
    let events = daemon.browse(SERVICE_TYPE)?;
    let deadline = tokio::time::Instant::now() + window;
    let mut found: Vec<Found> = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, events.recv_async()).await {
        let ServiceEvent::ServiceResolved(service) = event else {
            continue;
        };
        let Some(desktop) = to_found(&service) else { continue };
        let done = want.is_some_and(|id| *id == desktop.id);
        found.retain(|known| known.id != desktop.id);
        found.push(desktop);
        if done {
            break;
        }
    }
    if let Err(error) = daemon.shutdown() {
        log::debug!("mdns shutdown: {error}");
    }
    Ok(found)
}

fn to_found(service: &mdns_sd::ResolvedService) -> Option<Found> {
    let instance = service.get_fullname().strip_suffix(SERVICE_TYPE)?.strip_suffix('.')?;
    let id = DeviceId::parse(instance).ok()?;
    let port = service.get_port();
    let addresses = service
        .get_addresses()
        .iter()
        .map(mdns_sd::ScopedIp::to_ip_addr)
        .filter(|ip| !matches!(ip, IpAddr::V6(v6) if v6.is_unicast_link_local()))
        .map(|ip| SocketAddr::new(ip, port))
        .collect();
    Some(Found { id, pairing: service.get_property_val_str("pair") == Some("1"), addresses })
}

#[cfg(test)]
mod virtual_links {
    use super::is_virtual;

    #[test]
    fn skips_container_vm_and_vpn_links_but_keeps_lan_bridges() {
        for name in
            ["veth1a2b", "docker0", "br-0123456789ab", "virbr0", "ve-box", "podman0", "tailscale0", "wg0", "tun0"]
        {
            assert!(is_virtual(name), "{name}");
        }
        for name in ["wlan0", "wlp2s0", "enp3s0", "eth0", "br0", "br-lan", "bridge0"] {
            assert!(!is_virtual(name), "{name}");
        }
    }
}

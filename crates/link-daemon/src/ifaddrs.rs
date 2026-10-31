//! The IPv4 addresses on one network interface, for a join that waits for DHCP.

#![allow(unsafe_code, reason = "getifaddrs")]

use std::ffi::CStr;
use std::net::Ipv4Addr;

/// The interface's routable IPv4 addresses; a self-assigned 169.254 one means DHCP has not answered yet.
pub fn ipv4_addresses(interface: &str) -> Vec<Ipv4Addr> {
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

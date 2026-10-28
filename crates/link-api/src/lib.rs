//! The typed client of `org.andromeda.Link1`, generated from `api/org.andromeda.Link1.xml`: [`Link1Proxy`] and its
//! signal streams. Clients use only this, never names copied from the XML.

use zbus::proxy;

pub const SERVICE: &str = "org.andromeda.Link1";
pub const PATH: &str = "/org/andromeda/Link1";

include!(concat!(env!("OUT_DIR"), "/link1.rs"));

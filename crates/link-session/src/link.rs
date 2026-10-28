//! The calls `link-session` makes to `andromeda-linkd` over `org.andromeda.Link1`.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};

const SERVICE: &str = "org.andromeda.Link1";
const PATH: &str = "/org/andromeda/Link1";
const INTERFACE: &str = "org.andromeda.Link1";

#[derive(Clone)]
pub struct Link {
    pub proxy: zbus::Proxy<'static>,
}

impl Link {
    pub async fn connect(bus: &zbus::Connection) -> zbus::Result<Self> {
        Ok(Self { proxy: zbus::Proxy::new_owned(bus.clone(), SERVICE, PATH, INTERFACE).await? })
    }

    pub async fn notification_action(&self, device: &str, id: &str, action: &str, reply: &str) {
        let result: zbus::Result<()> = self.proxy.call("NotificationAction", &(device, id, action, reply)).await;
        log_failure("NotificationAction", result);
    }

    /// Whether a connected device holds the clipboard grant, so a new selection is worth sending.
    pub async fn clipboard_wanted(&self) -> bool {
        let devices: Vec<(String, String, bool)> = self.proxy.get_property("Devices").await.unwrap_or_default();
        let grants: HashMap<String, Vec<String>> = self.proxy.get_property("Grants").await.unwrap_or_default();
        devices.iter().any(|(id, _, connected)| {
            *connected && grants.get(id).is_some_and(|features| features.iter().any(|feature| feature == "clipboard"))
        })
    }

    /// `data` holds the bytes of `mimes[0]`.
    pub async fn offer_clipboard(&self, mimes: Vec<String>, data: &[u8]) {
        let offer = async {
            let fd = rustix::fs::memfd_create("link-clipboard", rustix::fs::MemfdFlags::CLOEXEC)?;
            let mut file = std::fs::File::from(fd);
            file.write_all(data)?;
            file.seek(SeekFrom::Start(0))?;
            let fd = zbus::OwnedFd::from(std::os::fd::OwnedFd::from(file));
            let result: () = self.proxy.call("OfferClipboard", &(mimes, fd)).await?;
            anyhow::Ok(result)
        };
        if let Err(error) = offer.await {
            log::warn!("OfferClipboard: {error}");
        }
    }

    /// Whether the phone's bytes reached `sink`.
    pub async fn pull_clipboard(&self, device: &str, id: u64, mime: &str, sink: std::os::fd::OwnedFd) -> bool {
        let sink = zbus::OwnedFd::from(sink);
        let result: zbus::Result<u64> = self.proxy.call("PullClipboard", &(device, id, mime, sink)).await;
        result.inspect_err(|error| log::warn!("PullClipboard: {error}")).is_ok()
    }

    pub async fn notification_dismiss(&self, device: &str, id: &str) {
        let result: zbus::Result<()> = self.proxy.call("NotificationDismiss", &(device, id)).await;
        log_failure("NotificationDismiss", result);
    }
}

fn log_failure(method: &str, result: zbus::Result<()>) {
    if let Err(error) = result {
        log::warn!("{method}: {error}");
    }
}

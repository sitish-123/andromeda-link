//! The calls `link-session` makes to `andromeda-linkd` over `org.andromeda.Link1`.

use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;

use link_api::Link1Proxy;

#[derive(Clone)]
pub struct Link {
    pub proxy: Link1Proxy<'static>,
}

impl Link {
    pub async fn connect(bus: &zbus::Connection) -> zbus::Result<Self> {
        Ok(Self { proxy: Link1Proxy::new(bus).await? })
    }

    pub async fn notification_action(&self, device: &str, id: &str, action: &str, reply: &str) {
        log_failure("NotificationAction", self.proxy.notification_action(device, id, action, reply).await);
    }

    /// Whether a connected device holds the clipboard grant, so a new selection is worth sending.
    pub async fn clipboard_wanted(&self) -> bool {
        let devices = self.proxy.devices().await.unwrap_or_default();
        let grants = self.proxy.grants().await.unwrap_or_default();
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
            let mimes: Vec<&str> = mimes.iter().map(String::as_str).collect();
            self.proxy.offer_clipboard(&mimes, file.as_fd().into()).await?;
            anyhow::Ok(())
        };
        if let Err(error) = offer.await {
            log::warn!("OfferClipboard: {error}");
        }
    }

    /// Whether the phone's bytes reached `sink`.
    pub async fn pull_clipboard(&self, device: &str, id: u64, mime: &str, sink: std::os::fd::OwnedFd) -> bool {
        let result = self.proxy.pull_clipboard(device, id, mime, sink.as_fd().into()).await;
        result.inspect_err(|error| log::warn!("PullClipboard: {error}")).is_ok()
    }

    pub async fn notification_dismiss(&self, device: &str, id: &str) {
        log_failure("NotificationDismiss", self.proxy.notification_dismiss(device, id).await);
    }

    pub async fn report_desktop_status(&self, volume: u8, muted: bool) {
        log_failure("SetDesktopStatus", self.proxy.set_desktop_status(u32::from(volume), muted).await);
    }

    /// Tells the daemon what this session can do; it forgets them when this process leaves the bus.
    pub async fn report_capabilities(&self, capabilities: &[&str]) {
        log_failure("SetSessionCapabilities", self.proxy.set_session_capabilities(capabilities).await);
    }
}

fn log_failure(method: &str, result: zbus::Result<()>) {
    if let Err(error) = result {
        log::warn!("{method}: {error}");
    }
}

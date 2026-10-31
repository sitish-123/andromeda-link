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

    /// Each type's bytes go in an anonymous file, under every name it is offered as.
    pub async fn offer_clipboard(&self, types: Vec<(Vec<String>, Vec<u8>)>) {
        let offer = async {
            let mut files = Vec::with_capacity(types.len());
            for (names, data) in &types {
                let mut file = anonymous_file()?;
                file.write_all(data)?;
                file.seek(SeekFrom::Start(0))?;
                files.push((names, file));
            }
            let entries: Vec<(&str, zbus::Fd<'_>)> = files
                .iter()
                .flat_map(|(names, file)| names.iter().map(move |name| (name.as_str(), file.as_fd().into())))
                .collect();
            let entries: Vec<&(&str, zbus::Fd<'_>)> = entries.iter().collect();
            self.proxy.offer_clipboard(&entries).await?;
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

    pub async fn report_desktop_status(&self, volume: Option<(u8, bool)>, state: &crate::system::State) {
        let mut values = Vec::new();
        if let Some((percent, muted)) = volume {
            values.push(("volume", zbus::Variant::from(u32::from(percent))));
            values.push(("muted", zbus::Variant::from(muted)));
        }
        if let Some((percent, charging)) = state.battery {
            values.push(("battery", zbus::Variant::from(u32::from(percent))));
            values.push(("charging", zbus::Variant::from(charging)));
        }
        let flags = [("locked", state.locked), ("idle", state.idle), ("dnd", state.dnd)];
        values.extend(flags.into_iter().filter_map(|(key, flag)| Some((key, zbus::Variant::from(flag?)))));
        if let Some(brightness) = state.brightness {
            values.push(("brightness", zbus::Variant::from(u32::from(brightness))));
        }
        if let Some(profile) = &state.profile {
            values.push(("profile", zbus::Variant::from(profile.as_str())));
        }
        let map = values.iter().map(|(key, value)| (*key, value)).collect();
        log_failure("SetDesktopStatus", self.proxy.set_desktop_status(map).await);
    }

    /// Answers a phone's `SystemCommand` `serial`; `error` is for the user, None when it was done.
    pub async fn system_result(&self, device: &str, serial: u64, error: Option<&str>) {
        log_failure("SystemResult", self.proxy.system_result(device, serial, error.unwrap_or_default()).await);
    }

    #[cfg_attr(target_os = "macos", allow(dead_code, reason = "macOS offers no remote control"))]
    pub async fn stop_remote_control(&self, device: &str, reason: &str) {
        log_failure("StopRemoteControl", self.proxy.stop_remote_control(device, reason).await);
    }

    /// Tells the daemon what this session can do; it forgets them when this process leaves the bus.
    pub async fn report_capabilities(&self, capabilities: &[&str]) {
        log_failure("SetSessionCapabilities", self.proxy.set_session_capabilities(capabilities).await);
    }
}

#[cfg(target_os = "linux")]
pub fn anonymous_file() -> std::io::Result<std::fs::File> {
    Ok(rustix::fs::memfd_create("link-clipboard", rustix::fs::MemfdFlags::CLOEXEC)?.into())
}

/// macOS has no memfd: a private file in the temporary directory, unlinked once open.
#[cfg(target_os = "macos")]
pub fn anonymous_file() -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!("link-clipboard-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let path = std::env::temp_dir().join(name);
    let file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&path)?;
    std::fs::remove_file(&path)?;
    Ok(file)
}

fn log_failure(method: &str, result: zbus::Result<()>) {
    if let Err(error) = result {
        log::warn!("{method}: {error}");
    }
}

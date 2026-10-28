//! A `RemoteDesktop` portal session with the Clipboard portal on it (GNOME, KDE). The portal asks the user once;
//! the restore token it returns (`persist_mode` 2) starts later sessions without asking.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::Context;
use futures_util::StreamExt;
use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant};
use zbus::{OwnedFd, Variant};

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const REMOTE_DESKTOP: &str = "org.freedesktop.portal.RemoteDesktop";
pub const CLIPBOARD: &str = "org.freedesktop.portal.Clipboard";
/// The portal's device types: keyboard and pointer.
const DEVICES: u32 = 1 | 2;
/// Kept until the user revokes it.
const PERSIST: u32 = 2;

static TOKENS: AtomicU32 = AtomicU32::new(0);

pub struct Session {
    pub bus: zbus::Connection,
    pub handle: OwnedObjectPath,
    pub clipboard: bool,
}

impl Session {
    /// `None` when the session bus has no `RemoteDesktop` portal with libei (version 2).
    pub async fn start(bus: &zbus::Connection) -> anyhow::Result<Option<Self>> {
        let remote = &zbus::Proxy::new(bus, SERVICE, PATH, REMOTE_DESKTOP).await?;
        let Ok(version) = remote.get_property::<u32>("version").await else { return Ok(None) };
        if version < 2 {
            return Ok(None);
        }
        let clipboard_portal = zbus::Proxy::new(bus, SERVICE, PATH, CLIPBOARD).await?;
        let has_clipboard = clipboard_portal.get_property::<u32>("version").await.is_ok();

        let session_token = token();
        let results = request(bus, "CreateSession", |token| async move {
            let options = HashMap::from([
                ("handle_token", Variant::from(token.as_str())),
                ("session_handle_token", Variant::from(session_token.as_str())),
            ]);
            remote.call("CreateSession", &(options,)).await
        })
        .await?;
        let handle = &OwnedObjectPath::try_from(string(&results, "session_handle")?)?;

        let restore = std::fs::read_to_string(token_path()).unwrap_or_default();
        let restore = restore.trim();
        request(bus, "SelectDevices", |token| async move {
            let mut options = HashMap::from([
                ("handle_token", Variant::from(token.as_str())),
                ("types", Variant::from(DEVICES)),
                ("persist_mode", Variant::from(PERSIST)),
            ]);
            if !restore.is_empty() {
                options.insert("restore_token", Variant::from(restore));
            }
            remote.call("SelectDevices", &(handle, options)).await
        })
        .await?;
        // Asked for before Start, as the Clipboard portal requires.
        if has_clipboard {
            let options: HashMap<&str, Variant<'_>> = HashMap::new();
            let () = clipboard_portal.call("RequestClipboard", &(handle, options)).await?;
        }
        let results = request(bus, "Start", |token| async move {
            let options = HashMap::from([("handle_token", Variant::from(token.as_str()))]);
            remote.call("Start", &(handle, "", options)).await
        })
        .await?;
        if let Ok(token) = string(&results, "restore_token") {
            save_token(&token);
        }
        let enabled = results.get("clipboard_enabled").and_then(|value| bool::try_from(value).ok());
        Ok(Some(Self { bus: bus.clone(), handle: handle.clone(), clipboard: has_clipboard && enabled == Some(true) }))
    }

    /// The libei socket for the session's devices.
    pub async fn eis(&self) -> anyhow::Result<std::os::fd::OwnedFd> {
        let remote = zbus::Proxy::new(&self.bus, SERVICE, PATH, REMOTE_DESKTOP).await?;
        let options: HashMap<&str, Variant<'_>> = HashMap::new();
        let fd: OwnedFd = remote.call("ConnectToEIS", &(&self.handle, options)).await?;
        Ok(fd.into())
    }
}

fn token() -> String {
    format!("andromeda_link_{}_{}", std::process::id(), TOKENS.fetch_add(1, Ordering::Relaxed))
}

/// Runs `call` with a fresh `handle_token`, and returns the results of the Response its Request object sends.
async fn request<C, F>(bus: &zbus::Connection, method: &str, call: C) -> anyhow::Result<HashMap<String, OwnedVariant>>
where
    C: FnOnce(String) -> F,
    F: Future<Output = zbus::Result<OwnedObjectPath>>,
{
    let token = token();
    let sender = bus.unique_name().context("no unique bus name")?.trim_start_matches(':').replace('.', "_");
    let path = format!("{PATH}/request/{sender}/{token}");
    let request = zbus::Proxy::new(bus, SERVICE, ObjectPath::try_from(path)?, "org.freedesktop.portal.Request").await?;
    // Subscribed before the call, so a fast Response is not missed.
    let mut responses = request.receive_signal("Response").await?;
    call(token).await.with_context(|| format!("calling {method}"))?;
    let message = responses.next().await.context("the portal sent no Response")?;
    let (code, results): (u32, HashMap<String, OwnedVariant>) = message.body().deserialize()?;
    match code {
        0 => Ok(results),
        1 => anyhow::bail!("{method}: the user denied the request"),
        _ => anyhow::bail!("{method}: the request was cancelled"),
    }
}

fn string(results: &HashMap<String, OwnedVariant>, key: &str) -> anyhow::Result<String> {
    let value = results.get(key).with_context(|| format!("no {key} in the portal's answer"))?;
    Ok(String::try_from(value.clone())?)
}

fn token_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME").map_or_else(
        || PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"),
        PathBuf::from,
    );
    state.join("andromeda-link-session/portal-restore-token")
}

fn save_token(token: &str) {
    let path = token_path();
    let saved = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, token));
    if let Err(error) = saved {
        log::warn!("saving the portal's restore token: {error}");
    }
}

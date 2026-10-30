use std::fs::File;
use std::io::{Seek, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};

use link_api::Link1Proxy;
use tokio::sync::mpsc::UnboundedSender;

use super::player::{PLAYER_INTERFACE, PLAYER_PATH, PLAYER_PREFIX, number};
use super::signals::{cache, hashed};
use super::{Command, Entry, Event, PlayerAction};

/// The `ReadFile` chunk, its largest allowed length.
const CHUNK: u32 = 1 << 20;

/// The phone's reason from an `.Refused` error, else the error's text.
fn reason(error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(name, Some(text), _) if name.as_str().ends_with(".Refused") => text.clone(),
        other => other.to_string(),
    }
}

pub(super) async fn execute(
    proxy: &Link1Proxy<'static>,
    events: &UnboundedSender<Event>,
    command: Command,
) -> zbus::Result<()> {
    match command {
        Command::Ring(device) => proxy.ring(&device, true).await,
        Command::Reply { device, id, action, text } => proxy.notification_action(&device, &id, &action, &text).await,
        Command::Dismiss { device, id } => proxy.notification_dismiss(&device, &id).await,
        Command::SendFiles { device, paths } => send_files(proxy, events, device, paths).await,
        Command::OfferClipboard(text) => {
            let failure = |error: std::io::Error| zbus::Error::Failure(format!("clipboard: {error}"));
            let fd = rustix::fs::memfd_create("link-clip", rustix::fs::MemfdFlags::CLOEXEC)
                .map_err(|error| failure(error.into()))?;
            let mut file = File::from(fd);
            file.write_all(text.as_bytes()).and_then(|()| file.rewind()).map_err(failure)?;
            let types = ["text/plain;charset=utf-8", "text/plain"].map(|mime| (mime, file.as_fd().into()));
            proxy.offer_clipboard(&types.iter().collect::<Vec<_>>()).await?;
            let _ = events.send(Event::ClipSent(text));
            Ok(())
        }
        Command::System { device, command, value } => proxy.send_system_command(&device, command, value).await,
        Command::Player { device, action } => {
            let name = format!("{PLAYER_PREFIX}{device}");
            let player = zbus::Proxy::new(proxy.inner().connection(), name, PLAYER_PATH, PLAYER_INTERFACE).await?;
            match action {
                PlayerAction::PlayPause => player.call("PlayPause", &()).await,
                PlayerAction::Next => player.call("Next", &()).await,
                PlayerAction::Previous => player.call("Previous", &()).await,
                PlayerAction::SetPosition(target) => {
                    let current = number(Some(&player.get_property::<zbus::OwnedVariant>("Position").await?));
                    player.call("Seek", &(target - current.unwrap_or(0))).await
                }
                PlayerAction::SetVolume(volume) => player.set_property("Volume", volume).await.map_err(Into::into),
            }
        }
        Command::StartService | Command::Resync | Command::SetQuickShare(_) => Ok(()),
        Command::Accept(transfer) => proxy.accept_transfer(&transfer).await,
        Command::Decline(transfer) => proxy.decline_transfer(&transfer).await,
        Command::Cancel(transfer) => proxy.cancel_transfer(&transfer).await,
        Command::SetGrant { device, feature, granted } => proxy.set_grant(&device, &feature, granted).await,
        Command::SetAutoAccept { device, enabled } => proxy.set_auto_accept(&device, enabled).await,
        Command::SetLocalSend(visible) => proxy.set_local_send_visible(visible).await,
        Command::SetNotifyLocked(on) => proxy.set_notifications_while_locked(on).await,
        Command::SetCommands(commands) => {
            let pairs: Vec<(&str, &str)> = commands.iter().map(|(name, line)| (name.as_str(), line.as_str())).collect();
            proxy.set_commands(&pairs.iter().collect::<Vec<_>>()).await
        }
        Command::Unpair(device) => proxy.unpair(&device).await,
        Command::SendText { device, to, body } => proxy.send_text(&device, &to, &body).await,
        Command::Pair => {
            let (code, uri) = proxy.start_pairing().await?;
            let _ = events.send(Event::Pairing { code, uri });
            Ok(())
        }
        Command::CancelPairing => proxy.cancel_pairing().await,
        Command::MirrorKey { device, action } => {
            let args: std::collections::HashMap<&str, &zbus::Variant<'_>> = std::collections::HashMap::new();
            proxy.mirror_input(&device, action, args).await
        }
        Command::MirrorKeyframe(device) => proxy.mirror_keyframe(&device).await,
        Command::ShowInFolder(path) => show_in_folder(proxy.inner().connection(), &path).await,
        // Phone reads wait on the phone, so they run beside the loop instead of holding the commands behind them.
        Command::List { device, path } => {
            let (proxy, events) = (proxy.clone(), events.clone());
            tokio::spawn(async move {
                let entries = proxy.list_files(&device, &path).await.map_err(|error| reason(&error)).map(|entries| {
                    entries.into_iter().map(|(name, folder, size, mtime)| Entry { name, folder, size, mtime }).collect()
                });
                let _ = events.send(Event::Listed { device, path, entries });
            });
            Ok(())
        }
        Command::Thumbnail { device, path, size } => {
            let (proxy, events) = (proxy.clone(), events.clone());
            tokio::spawn(async move {
                let file = thumbnail(&proxy, &device, &path, size).await;
                let _ = events.send(Event::Thumbnail { device, path, file });
            });
            Ok(())
        }
        Command::Fetch { device, paths, folder, page, purpose } => {
            let (proxy, events) = (proxy.clone(), events.clone());
            tokio::spawn(async move {
                let mut saved = Vec::new();
                for path in &paths {
                    match save(&proxy, &device, path, &folder).await {
                        Ok(file) => saved.push(file),
                        Err(error) => drop(events.send(Event::Failed(error.to_string()))),
                    }
                }
                if !saved.is_empty() {
                    let _ = events.send(Event::Fetched { device, page, purpose, paths: saved });
                }
            });
            Ok(())
        }
    }
}

async fn send_files(
    proxy: &Link1Proxy<'static>,
    events: &UnboundedSender<Event>,
    device: String,
    paths: Vec<PathBuf>,
) -> zbus::Result<()> {
    let mut files = Vec::new();
    for path in paths {
        let file = File::open(&path).map_err(|error| zbus::Error::Failure(format!("{}: {error}", path.display())))?;
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        files.push((file, name));
    }
    let pairs: Vec<(zbus::Fd<'_>, &str)> =
        files.iter().map(|(file, name)| (file.as_fd().into(), name.as_str())).collect();
    let transfer = proxy.send_files(&device, &pairs.iter().collect::<Vec<_>>()).await?;
    let files = files.iter().map(|(file, name)| (name.clone(), file.metadata().map_or(0, |meta| meta.len()))).collect();
    let _ = events.send(Event::Sending { transfer, device, files });
    Ok(())
}

/// The cached PNG of a phone file's thumbnail, asked for once per (device, path, size).
async fn thumbnail(proxy: &Link1Proxy<'_>, device: &str, path: &str, size: u16) -> Option<PathBuf> {
    let file = cache("thumbnails").ok()?.join(hashed(&(device, path, size), "png"));
    if file.exists() {
        return Some(file);
    }
    let png =
        proxy.thumbnail(device, path, size).await.map_err(|error| log::debug!("thumbnail {path}: {error}")).ok()?;
    std::fs::write(&file, png).map_err(|error| log::warn!("{}: {error}", file.display())).ok()?;
    Some(file)
}

/// Asks the file manager to show the file, over the freedesktop `FileManager1` interface.
async fn show_in_folder(bus: &zbus::Connection, path: &Path) -> zbus::Result<()> {
    let uri = format!("file://{}", path.display());
    bus.call_method(
        Some("org.freedesktop.FileManager1"),
        "/org/freedesktop/FileManager1",
        Some("org.freedesktop.FileManager1"),
        "ShowItems",
        &(vec![uri.as_str()], ""),
    )
    .await
    .map(drop)
}

/// Copies a phone file into the folder under a free name, a chunk at a time.
async fn save(proxy: &Link1Proxy<'_>, device: &str, path: &str, folder: &Path) -> zbus::Result<PathBuf> {
    static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let failure = |error: std::io::Error| zbus::Error::Failure(format!("saving {path}: {error}"));
    std::fs::create_dir_all(folder).map_err(failure)?;
    let name = path.rsplit('/').next().filter(|name| !name.is_empty()).unwrap_or("file");
    let save = SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = folder.join(format!(".{name}.{}-{save}.part", std::process::id()));
    let mut file = File::options().write(true).create_new(true).open(&partial).map_err(failure)?;
    let copied = async {
        let mut offset = 0;
        loop {
            let data = proxy.read_file(device, path, offset, CHUNK).await?;
            if data.is_empty() {
                return Ok(());
            }
            file.write_all(&data).map_err(failure)?;
            offset += data.len() as u64;
        }
    }
    .await
    .and_then(|()| claim(folder, name, &partial).map_err(failure));
    let _ = std::fs::remove_file(&partial);
    copied
}

/// Links `partial` to the first free name of "name", "name (2)", ...; a link fails rather than replace a file.
fn claim(folder: &Path, name: &str, partial: &Path) -> std::io::Result<PathBuf> {
    let (stem, extension) =
        name.rsplit_once('.').map_or((name, String::new()), |(stem, ext)| (stem, format!(".{ext}")));
    for copy in 1.. {
        let target = if copy == 1 { folder.join(name) } else { folder.join(format!("{stem} ({copy}){extension}")) };
        match std::fs::hard_link(partial, &target) {
            Ok(()) => return Ok(target),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    unreachable!("an unbounded range")
}

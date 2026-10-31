//! The app's view of `andromeda-linkd`: a D-Bus client on its own thread, events in and commands out. It talks on the
//! session bus, or peer to peer on the daemon's socket where there is none (macOS).

mod execute;
mod player;
mod signals;
mod types;

use std::sync::OnceLock;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::future::OptionFuture;
use link_api::{Link1Proxy, QuickShareProxy, SERVICE};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub use signals::cache;
pub use types::*;

use player::{send_players, tick};
use signals::{send_calls, send_controls, send_devices, send_info, send_status, send_texts, signal};

static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// The client thread's runtime, for other D-Bus services of the app; None before `start` or when it failed.
#[cfg_attr(target_os = "macos", allow(dead_code, reason = "only the Linux tray uses it"))]
pub fn runtime() -> Option<&'static tokio::runtime::Handle> {
    RUNTIME.get()
}

/// Starts the client thread; `sink` gets every event on that thread. It reconnects on its own when the service
/// restarts, but never starts the service without `Command::StartService`.
pub fn start(sink: impl Fn(Event) + Send + 'static) -> UnboundedSender<Command> {
    let (commands, command_rx) = unbounded_channel();
    let (event_tx, mut events) = unbounded_channel();
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("no tokio runtime: {error}");
            return commands;
        }
    };
    let _ = RUNTIME.set(runtime.handle().clone());
    std::thread::spawn(move || {
        runtime.spawn(async move {
            while let Some(event) = events.recv().await {
                sink(event);
            }
        });
        let ticks = event_tx.clone();
        runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            while ticks.send(Event::Tick).is_ok() {
                interval.tick().await;
            }
        });
        runtime.block_on(run(command_rx, event_tx));
    });
    commands
}

async fn run(mut commands: UnboundedReceiver<Command>, events: UnboundedSender<Event>) {
    #[cfg(target_os = "macos")]
    if let Err(error) = install_agents() {
        let _ = events.send(Event::Failed(format!("Could not set up the Link service: {error}")));
    }
    #[cfg(target_os = "linux")]
    start_session_units();
    loop {
        match session(&mut commands, &events).await {
            Ok(()) => return,
            Err(error) => log::info!("andromeda-linkd: {error}"),
        }
        let _ = events.send(Event::Service(false));
        let retry = tokio::time::sleep(Duration::from_secs(2));
        tokio::pin!(retry);
        loop {
            tokio::select! {
                () = &mut retry => break,
                command = commands.recv() => match command {
                    Some(Command::StartService) => {
                        if let Err(error) = start_service().await {
                            let _ = events.send(Event::Failed(format!("Could not start the service: {error}")));
                        }
                        break;
                    }
                    Some(Command::Resync) => {}
                    Some(_) => { let _ = events.send(Event::Failed("The Link service is not running".into())); }
                    None => return,
                },
            }
        }
    }
}

/// Loads the service and link-session as launchd agents from the bundle's installer, when they are missing or name
/// another copy of the app.
#[cfg(target_os = "macos")]
fn install_agents() -> std::io::Result<()> {
    // The binary is Andromeda Link.app/Contents/MacOS/andromeda-link.
    let exe = std::env::current_exe()?;
    let contents = exe.parent().and_then(std::path::Path::parent).ok_or(std::io::ErrorKind::NotFound)?;
    let app = contents.parent().ok_or(std::io::ErrorKind::NotFound)?;
    // Gatekeeper runs a downloaded app from a random read-only copy until the user moves it; agents there break.
    if app.to_string_lossy().contains("/AppTranslocation/") {
        return Err(std::io::Error::other("move Andromeda Link to Applications, then open it again"));
    }
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let installed = std::fs::read_to_string(home.join("Library/LaunchAgents/org.andromeda.link-session.plist"));
    let helper = format!("{}/Contents/Library/LoginItems/Andromeda Link Helper.app/", app.display());
    if installed.is_ok_and(|plist| plist.contains(&helper)) {
        return Ok(());
    }
    let status = std::process::Command::new(contents.join("Resources/install-agents.sh")).arg(app).status()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("install-agents.sh: {status}"))) }
}

/// Starts link-session and the mount, which their units start only with graphical-session.target: so not after a
/// package install until the next login, and never on a compositor without that target.
#[cfg(target_os = "linux")]
fn start_session_units() {
    let started = std::process::Command::new("systemctl")
        .args(["--user", "--no-block", "start", "andromeda-link-session", "andromeda-link-mount"])
        .status();
    match started {
        Ok(status) if status.success() => {}
        Ok(status) => log::warn!("systemctl start of the session units: {status}"),
        Err(error) => log::info!("systemctl: {error}"),
    }
}

/// launchd keeps the daemon alive; this starts it when the user stopped it.
#[cfg(target_os = "macos")]
#[expect(clippy::unused_async, reason = "the same signature as the D-Bus start elsewhere")]
async fn start_service() -> zbus::Result<()> {
    let status = std::process::Command::new("launchctl").args(["start", "org.andromeda.linkd"]).status()?;
    if status.success() { Ok(()) } else { Err(zbus::Error::Failure(format!("launchctl start: {status}"))) }
}

#[cfg(not(target_os = "macos"))]
async fn start_service() -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    zbus::fdo::DBusProxy::new(&bus).await?.start_service_by_name(SERVICE.try_into()?, 0).await?;
    Ok(())
}

/// One connection to the service; returns Ok when the app closes, Err when the service is gone.
async fn session(commands: &mut UnboundedReceiver<Command>, events: &UnboundedSender<Event>) -> zbus::Result<()> {
    let bus = link_api::connect().await?;
    // On the socket there are no names: the daemon is there when the connection is.
    let mut owner = None;
    if bus.is_bus() {
        let names = zbus::fdo::DBusProxy::new(&bus).await?;
        // A call to an absent service would start it through D-Bus activation; only StartService may do that.
        if !names.name_has_owner(SERVICE.try_into()?).await? {
            return Err(zbus::Error::Failure("the service is not running".into()));
        }
        owner = Some(names.receive_name_owner_changed().await?);
    }
    let proxy = Link1Proxy::new(&bus).await?;
    let quick_share = QuickShareProxy::new(&bus).await?;
    proxy.devices().await?;
    let _ = events.send(Event::Service(true));
    snapshot(&proxy, &quick_share, events).await?;

    let mut players = std::collections::HashMap::new();
    send_players(events, &mut players, proxy.phone_players().await?);
    let mut player_changes = proxy.receive_phone_players_changed().await;
    let mut ticks = tokio::time::interval(Duration::from_secs(1));
    let mut devices = proxy.receive_devices_changed().await;
    let mut info = proxy.receive_device_info_changed().await;
    let mut status = proxy.receive_device_status_changed().await;
    let mut controls = proxy.receive_device_controls_changed().await;
    let mut grants = proxy.receive_grants_changed().await;
    let mut auto_accept = proxy.receive_auto_accept_changed().await;
    let mut local_send = proxy.receive_local_send_visible_changed().await;
    let mut quick_share_visible = quick_share.receive_visible_changed().await;
    let mut notify_locked = proxy.receive_notifications_while_locked_changed().await;
    let mut notifications_off = proxy.receive_notifications_off_changed().await;
    let mut session_capabilities = proxy.receive_session_capabilities_changed().await;
    let mut stored = proxy.receive_commands_changed().await;
    let mut texts = proxy.receive_texts_changed().await;
    let mut contacts = proxy.receive_contacts_changed().await;
    let mut calls = proxy.receive_calls_changed().await;
    let mut calls_refused = proxy.receive_calls_refused_changed().await;
    let mut signals = proxy.inner().receive_all_signals().await?;
    loop {
        tokio::select! {
            Some(change) = devices.next() => send_devices(events, change.get().await?),
            Some(change) = player_changes.next() => send_players(events, &mut players, change.get().await?),
            _ = ticks.tick(), if players.values().any(|player| player.5) => tick(events, &players),
            Some(change) = info.next() => send_info(events, change.get().await?),
            Some(change) = status.next() => send_status(events, change.get().await?),
            Some(change) = controls.next() => send_controls(events, change.get().await?),
            Some(change) = grants.next() => drop(events.send(Event::Grants(change.get().await?))),
            Some(change) = auto_accept.next() => drop(events.send(Event::AutoAccept(change.get().await?))),
            Some(change) = local_send.next() => drop(events.send(Event::LocalSend(change.get().await?))),
            Some(change) = quick_share_visible.next() => drop(events.send(Event::QuickShare(change.get().await?))),
            Some(change) = notify_locked.next() => drop(events.send(Event::NotifyLocked(change.get().await?))),
            Some(change) = notifications_off.next() => {
                drop(events.send(Event::NotificationsOff(change.get().await?)));
            }
            Some(change) = session_capabilities.next() => {
                drop(events.send(Event::SessionCapabilities(change.get().await?)));
            }
            Some(change) = stored.next() => drop(events.send(Event::Commands(change.get().await?))),
            Some(change) = texts.next() => send_texts(events, change.get().await?),
            Some(change) = contacts.next() => drop(events.send(Event::Contacts(change.get().await?))),
            Some(change) = calls.next() => send_calls(events, change.get().await?),
            Some(change) = calls_refused.next() => drop(events.send(Event::CallsRefused(change.get().await?))),
            Some(message) = signals.next() => {
                if let Some(event) = signal(message) {
                    let _ = events.send(event);
                }
            }
            () = bus.closed() => return Err(zbus::Error::Failure("the service stopped".into())),
            Some(Some(change)) = OptionFuture::from(owner.as_mut().map(StreamExt::next)) => {
                let args = change.args()?;
                if args.name() == SERVICE && args.new_owner().is_none() {
                    return Err(zbus::Error::Failure("the service stopped".into()));
                }
            }
            command = commands.recv() => match command {
                Some(Command::Resync) => snapshot(&proxy, &quick_share, events).await?,
                Some(command) => {
                    let result = match command {
                        Command::SetQuickShare(on) => quick_share.set_visible(on).await,
                        command => execute::execute(&proxy, events, command).await,
                    };
                    if let Err(error) = result {
                        let _ = events.send(Event::Failed(execute::reason(&error)));
                    }
                }
                None => return Ok(()),
            },
        }
    }
}

/// Sends every property the pages show.
async fn snapshot(
    proxy: &Link1Proxy<'static>,
    quick_share: &QuickShareProxy<'static>,
    events: &UnboundedSender<Event>,
) -> zbus::Result<()> {
    send_devices(events, proxy.devices().await?);
    send_info(events, proxy.device_info().await.unwrap_or_default());
    send_status(events, proxy.device_status().await.unwrap_or_default());
    send_controls(events, proxy.device_controls().await.unwrap_or_default());
    let _ = events.send(Event::Grants(proxy.grants().await?));
    let _ = events.send(Event::AutoAccept(proxy.auto_accept().await?));
    let _ = events.send(Event::LocalSend(proxy.local_send_visible().await?));
    let _ = events.send(Event::QuickShare(quick_share.visible().await?));
    let _ = events.send(Event::NotifyLocked(proxy.notifications_while_locked().await?));
    let _ = events.send(Event::NotificationsOff(proxy.notifications_off().await?));
    let _ = events.send(Event::SessionCapabilities(proxy.session_capabilities().await?));
    let _ = events.send(Event::Commands(proxy.commands().await?));
    send_texts(events, proxy.texts().await?);
    let _ = events.send(Event::Contacts(proxy.contacts().await?));
    send_calls(events, proxy.calls().await.unwrap_or_default());
    let _ = events.send(Event::CallsRefused(proxy.calls_refused().await.unwrap_or_default()));
    Ok(())
}

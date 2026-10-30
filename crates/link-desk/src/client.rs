//! The app's view of `andromeda-linkd`: a D-Bus client on its own thread, events in and commands out.

mod execute;
mod player;
mod signals;
mod types;

use std::time::Duration;

use futures_util::StreamExt;
use link_api::{Link1Proxy, SERVICE};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub use signals::cache;
pub use types::*;

use player::{PLAYER_PREFIX, watch_player};
use signals::{send_controls, send_devices, send_info, send_status, send_texts, signal};

/// Starts the client thread; `sink` gets every event on that thread. It reconnects on its own when the service
/// restarts, but never starts the service without `Command::StartService`.
pub fn start(sink: impl Fn(Event) + Send + 'static) -> UnboundedSender<Command> {
    let (commands, command_rx) = unbounded_channel();
    let (event_tx, mut events) = unbounded_channel();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => {
                log::error!("no tokio runtime: {error}");
                return;
            }
        };
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

async fn start_service() -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    zbus::fdo::DBusProxy::new(&bus).await?.start_service_by_name(SERVICE.try_into()?, 0).await?;
    Ok(())
}

/// One connection to the service; returns Ok when the app closes, Err when the service is gone.
async fn session(commands: &mut UnboundedReceiver<Command>, events: &UnboundedSender<Event>) -> zbus::Result<()> {
    let bus = zbus::Connection::session().await?;
    let names = zbus::fdo::DBusProxy::new(&bus).await?;
    // A call to an absent service would start it through D-Bus activation; only StartService may do that.
    if !names.name_has_owner(SERVICE.try_into()?).await? {
        return Err(zbus::Error::Failure("the service is not running".into()));
    }
    let proxy = Link1Proxy::new(&bus).await?;
    let mut owner = names.receive_name_owner_changed().await?;
    for name in names.list_names().await? {
        if name.starts_with(PLAYER_PREFIX) {
            tokio::spawn(watch_player(bus.clone(), name.to_string(), events.clone()));
        }
    }
    proxy.devices().await?;
    let _ = events.send(Event::Service(true));
    snapshot(&proxy, events).await?;

    let mut devices = proxy.receive_devices_changed().await;
    let mut info = proxy.receive_device_info_changed().await;
    let mut status = proxy.receive_device_status_changed().await;
    let mut controls = proxy.receive_device_controls_changed().await;
    let mut grants = proxy.receive_grants_changed().await;
    let mut auto_accept = proxy.receive_auto_accept_changed().await;
    let mut local_send = proxy.receive_local_send_visible_changed().await;
    let mut notify_locked = proxy.receive_notifications_while_locked_changed().await;
    let mut notifications_off = proxy.receive_notifications_off_changed().await;
    let mut stored = proxy.receive_commands_changed().await;
    let mut texts = proxy.receive_texts_changed().await;
    let mut contacts = proxy.receive_contacts_changed().await;
    let mut signals = proxy.inner().receive_all_signals().await?;
    loop {
        tokio::select! {
            Some(change) = devices.next() => send_devices(events, change.get().await?),
            Some(change) = info.next() => send_info(events, change.get().await?),
            Some(change) = status.next() => send_status(events, change.get().await?),
            Some(change) = controls.next() => send_controls(events, change.get().await?),
            Some(change) = grants.next() => drop(events.send(Event::Grants(change.get().await?))),
            Some(change) = auto_accept.next() => drop(events.send(Event::AutoAccept(change.get().await?))),
            Some(change) = local_send.next() => drop(events.send(Event::LocalSend(change.get().await?))),
            Some(change) = notify_locked.next() => drop(events.send(Event::NotifyLocked(change.get().await?))),
            Some(change) = notifications_off.next() => {
                drop(events.send(Event::NotificationsOff(change.get().await?)));
            }
            Some(change) = stored.next() => drop(events.send(Event::Commands(change.get().await?))),
            Some(change) = texts.next() => send_texts(events, change.get().await?),
            Some(change) = contacts.next() => drop(events.send(Event::Contacts(change.get().await?))),
            Some(message) = signals.next() => {
                if let Some(event) = signal(message) {
                    let _ = events.send(event);
                }
            }
            Some(change) = owner.next() => {
                let args = change.args()?;
                if args.name() == SERVICE && args.new_owner().is_none() {
                    return Err(zbus::Error::Failure("the service stopped".into()));
                }
                if let Some(device) = args.name().strip_prefix(PLAYER_PREFIX) {
                    if args.new_owner().is_some() {
                        tokio::spawn(watch_player(bus.clone(), args.name().to_string(), events.clone()));
                    } else {
                        let _ = events.send(Event::Player { device: device.to_owned(), player: None });
                    }
                }
            }
            command = commands.recv() => match command {
                Some(Command::Resync) => snapshot(&proxy, events).await?,
                Some(command) => {
                    if let Err(error) = execute::execute(&proxy, events, command).await {
                        let _ = events.send(Event::Failed(error.to_string()));
                    }
                }
                None => return Ok(()),
            },
        }
    }
}

/// Sends every property the pages show.
async fn snapshot(proxy: &Link1Proxy<'static>, events: &UnboundedSender<Event>) -> zbus::Result<()> {
    send_devices(events, proxy.devices().await?);
    send_info(events, proxy.device_info().await.unwrap_or_default());
    send_status(events, proxy.device_status().await.unwrap_or_default());
    send_controls(events, proxy.device_controls().await.unwrap_or_default());
    let _ = events.send(Event::Grants(proxy.grants().await?));
    let _ = events.send(Event::AutoAccept(proxy.auto_accept().await?));
    let _ = events.send(Event::LocalSend(proxy.local_send_visible().await?));
    let _ = events.send(Event::NotifyLocked(proxy.notifications_while_locked().await?));
    let _ = events.send(Event::NotificationsOff(proxy.notifications_off().await?));
    let _ = events.send(Event::Commands(proxy.commands().await?));
    send_texts(events, proxy.texts().await?);
    let _ = events.send(Event::Contacts(proxy.contacts().await?));
    Ok(())
}

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;

use super::{Event, Player};

/// The prefix of the MPRIS names the service exports, one per phone, followed by the device id.
pub(super) const PLAYER_PREFIX: &str = "org.mpris.MediaPlayer2.andromeda_link_";
pub(super) const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
pub(super) const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const ROOT_INTERFACE: &str = "org.mpris.MediaPlayer2";

/// Follows one phone's MPRIS player until its name goes away; reads the position each second while it plays,
/// since MPRIS does not signal it.
pub(super) async fn watch_player(bus: zbus::Connection, name: String, events: UnboundedSender<Event>) {
    let Some(device) = name.strip_prefix(PLAYER_PREFIX).map(str::to_owned) else { return };
    let result: zbus::Result<()> = async {
        let properties =
            zbus::fdo::PropertiesProxy::builder(&bus).destination(name.as_str()).path(PLAYER_PATH).build().await?;
        let interface = zbus::names::InterfaceName::from_static_str_unchecked(PLAYER_INTERFACE);
        let mut changes = properties.receive_properties_changed().await?;
        let mut current = None;
        loop {
            match current.as_mut() {
                Some(player @ &mut Player { playing: true, .. }) => {
                    tokio::select! {
                        change = changes.next() => if change.is_none() { return Ok(()) } else { current = None },
                        () = tokio::time::sleep(Duration::from_secs(1)) => {
                            let position = properties.get(interface.clone(), "Position").await?;
                            player.position = number(Some(&position)).unwrap_or(player.position);
                        }
                    }
                }
                Some(_) => {
                    if changes.next().await.is_none() {
                        return Ok(());
                    }
                    current = None;
                }
                None => {
                    let all = properties.get_all(interface.clone()).await?;
                    let identity = properties
                        .get(zbus::names::InterfaceName::from_static_str_unchecked(ROOT_INTERFACE), "Identity")
                        .await
                        .ok()
                        .and_then(|value| String::try_from(value).ok())
                        .unwrap_or_default();
                    current = Some(player(&all, &identity));
                }
            }
            let _ = events.send(Event::Player { device: device.clone(), player: current.clone() });
        }
    }
    .await;
    if let Err(error) = result {
        log::debug!("{name}: {error}");
    }
}

pub(super) fn number(value: Option<&zbus::OwnedVariant>) -> Option<i64> {
    value.and_then(|value| {
        i64::try_from(value.clone())
            .ok()
            .or_else(|| u64::try_from(value.clone()).ok().and_then(|v| i64::try_from(v).ok()))
    })
}

fn player(all: &HashMap<String, zbus::OwnedVariant>, identity: &str) -> Player {
    let text = |value: Option<&zbus::OwnedVariant>| value.and_then(|value| String::try_from(value.clone()).ok());
    let metadata: HashMap<String, zbus::OwnedVariant> = all
        .get("Metadata")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| value.try_into().ok())
        .unwrap_or_default();
    let artist = metadata
        .get("xesam:artist")
        .and_then(|value| <Vec<String>>::try_from(value.clone()).ok())
        .map(|artists| artists.join(", "))
        .unwrap_or_default();
    Player {
        app: identity.split_once(" on ").map_or(identity, |(app, _)| app).to_owned(),
        title: text(metadata.get("xesam:title")).unwrap_or_default(),
        artist,
        art: text(metadata.get("mpris:artUrl")).unwrap_or_default(),
        playing: text(all.get("PlaybackStatus")).as_deref() == Some("Playing"),
        position: number(all.get("Position")).unwrap_or(0),
        length: number(metadata.get("mpris:length")).unwrap_or(0),
        volume: all.get("Volume").and_then(|value| f64::try_from(value.clone()).ok()).unwrap_or(1.0),
    }
}

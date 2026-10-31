//! Each phone's lead player, from the service's `PhonePlayers` property; the app advances the position while it plays.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc::UnboundedSender;

use super::{Event, Player};

/// One phone's `PhonePlayers` value; api/org.andromeda.Link1.xml names the fields.
pub(super) type Reported = (String, String, String, String, String, bool, u64, u64, i32, i64, Vec<String>);

/// Sends each phone's player, and None for a phone whose player went.
pub(super) fn send_players(
    events: &UnboundedSender<Event>,
    known: &mut HashMap<String, Reported>,
    players: HashMap<String, Reported>,
) {
    for device in known.keys().filter(|device| !players.contains_key(*device)) {
        let _ = events.send(Event::Player { device: device.clone(), player: None });
    }
    *known = players;
    tick(events, known);
}

/// Sends the players with their positions now.
pub(super) fn tick(events: &UnboundedSender<Event>, known: &HashMap<String, Reported>) {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis());
    let now = i64::try_from(now).unwrap_or(i64::MAX);
    for (device, (_, app, title, artist, art, playing, position, length, volume, at, _)) in known {
        let elapsed = if *playing { u64::try_from(now.saturating_sub(*at)).unwrap_or(0) } else { 0 };
        let position = position.saturating_add(elapsed);
        let position = if *length > 0 { position.min(*length) } else { position };
        let player = Player {
            app: app.clone(),
            title: title.clone(),
            artist: artist.clone(),
            art: art.clone(),
            playing: *playing,
            position: i64::try_from(position).unwrap_or(i64::MAX).saturating_mul(1000),
            length: i64::try_from(*length).unwrap_or(i64::MAX).saturating_mul(1000),
            volume: if *volume < 0 { 1.0 } else { f64::from(*volume) / 100.0 },
        };
        let _ = events.send(Event::Player { device: device.clone(), player: Some(player) });
    }
}

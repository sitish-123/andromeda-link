use std::collections::HashMap;
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QMap, QString, QUrl, QVariant};

use super::qobject;
use crate::client::{Event, Notification};
use crate::format::{age_text, file_url, now, text, variant};
use crate::history::{Direction, History, State};
use crate::hub;
use crate::tray;

/// The notification rows for one phone, grouped by app for the view: the app with the newest notification first,
/// newest first within each.
fn notification_rows(posted: &[(Notification, u64)], id: &str) -> Vec<Vec<QVariant>> {
    let mut posted: Vec<_> = posted.iter().filter(|(notification, _)| notification.device == id).collect();
    let mut newest: HashMap<&str, u64> = HashMap::new();
    for (notification, at) in &posted {
        let latest = newest.entry(notification.app.as_str()).or_default();
        *latest = (*latest).max(*at);
    }
    posted.sort_by(|(a, a_at), (b, b_at)| {
        let group = |notification: &Notification| (newest[notification.app.as_str()], notification.app.clone());
        group(b).cmp(&group(a)).then(b_at.cmp(a_at))
    });
    posted
        .into_iter()
        .map(|(notification, at)| {
            vec![
                text(&notification.id),
                text(&notification.app),
                text(&notification.title),
                text(&notification.text),
                text(&age_text(*at)),
                text(&notification.icon.as_ref().map(|path| file_url(&path.to_string_lossy())).unwrap_or_default()),
                variant(&notification.actions.iter().any(|(_, _, reply)| *reply)),
            ]
        })
        .collect()
}

/// The three newest finished transfers and clips of one phone, for Home.
fn recent_rows(history: &History, id: &str) -> Vec<Vec<QVariant>> {
    history
        .items
        .iter()
        .rev()
        .filter(|item| item.device == id && item.state == State::Done)
        .take(3)
        .map(|item| {
            let detail = match item.direction {
                Direction::Received => "Received",
                Direction::Sent => "Sent",
                Direction::CopiedFrom => "Copied from phone",
                Direction::CopiedTo => "Sent clipboard",
                Direction::Saved => "Saved from phone",
            };
            let thumb =
                if item.kind() == "photo" && !item.path.is_empty() { file_url(&item.path) } else { String::new() };
            vec![text(&item.name), text(detail), text(&age_text(item.at)), text(&thumb), text(item.kind())]
        })
        .collect()
}

impl qobject::Link {
    pub(super) fn apply(mut self: Pin<&mut Self>, event: Event) {
        let mut state = self.as_mut().rust_mut();
        match event {
            Event::Service(running) => {
                if !running {
                    for device in &mut state.devices {
                        device.connected = false;
                    }
                    state.players.clear();
                }
                self.as_mut().set_service_running(running);
            }
            Event::Devices(devices) => state.devices = devices,
            Event::Info(info) => state.info = info,
            Event::Status(status) => state.status.extend(status),
            Event::Controls(controls) => state.controls.extend(controls),
            Event::Notification(notification) => {
                state.posted.retain(|(old, _)| old.device != notification.device || old.id != notification.id);
                state.posted.push((notification, now()));
            }
            Event::NotificationRemoved { device, id } => {
                state.posted.retain(|(old, _)| old.device != device || old.id != id);
            }
            Event::Player { device, player } => match player {
                Some(player) => drop(state.players.insert(device, player)),
                None => drop(state.players.remove(&device)),
            },
            Event::Failed(message) => {
                self.as_mut().failed(QString::from(&message));
                return;
            }
            Event::Tick => {}
            Event::HotspotAsk { device, ssid } => {
                self.as_mut().set_hotspot_ask(QString::from(if ssid.is_empty() { "" } else { device.as_str() }));
                return;
            }
            other => {
                let phone = state.current_phone.to_string();
                if !state.history.apply(&other, &phone) {
                    return;
                }
            }
        }
        self.refresh();
    }

    /// Recomputes every property and both models from the state, for the current phone, and the tray menu.
    pub(super) fn refresh(mut self: Pin<&mut Self>) {
        self.as_mut().recompute();
        self.as_ref().show_in_tray();
    }

    fn recompute(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        let current = state.current_phone.to_string();
        if !state.devices.iter().any(|device| device.id == current) {
            let pick = state.devices.iter().find(|device| device.connected).or(state.devices.first());
            state.current_phone = QString::from(pick.map_or("", |device| device.id.as_str()));
            self.as_mut().current_phone_changed();
            state = self.as_mut().rust_mut();
        }
        let id = state.current_phone.to_string();
        if hub::phone() != id {
            hub::set_phone(&id);
        }

        let mut phones = QList::default();
        for device in &state.devices {
            let mut phone = QMap::<cxx_qt_lib::QMapPair_QString_QVariant>::default();
            phone.insert(QString::from("id"), text(&device.id));
            phone.insert(QString::from("name"), text(&device.name));
            phone.insert(QString::from("connected"), variant(&device.connected));
            phone.insert(QString::from("lastSeen"), variant(&state.info.get(&device.id).map_or(0, |info| info.0)));
            phones.append(variant(&phone));
        }
        let connected = state.devices.iter().any(|device| device.id == id && device.connected);
        let last_seen = state.info.get(&id).map_or(0, |info| info.0);
        let transport = match state.info.get(&id).filter(|_| connected).map_or("", |info| info.1.as_str()) {
            "wifi" => "Wi-Fi",
            "bluetooth" => "Bluetooth",
            _ => "",
        };
        let status = state.status.get(&id).cloned();
        let controls = state.controls.get(&id).cloned();
        let player = state.players.get(&id).cloned();

        let notifications = notification_rows(&state.posted, &id);
        let recent = recent_rows(&state.history, &id);
        if let Some(model) = state.notification_model.as_mut() {
            model.replace(notifications);
        }
        if let Some(model) = state.recent_model.as_mut() {
            model.replace(recent);
        }

        self.as_mut().set_phones(phones);
        self.as_mut().set_connected(connected);
        self.as_mut().set_last_seen(last_seen);
        self.as_mut().set_transport(QString::from(transport));
        self.as_mut().set_battery(status.as_ref().map_or(-1, |status| i32::try_from(status.battery).unwrap_or(-1)));
        self.as_mut().set_charging(status.as_ref().is_some_and(|status| status.charging));
        self.as_mut().set_network(QString::from(status.as_ref().map_or("", |status| status.network.as_str())));
        self.as_mut().set_signal(status.as_ref().map_or(-1, |status| status.signal));
        let ringer = match controls.as_ref().map(|controls| controls.ringer.as_str()) {
            Some("normal") => "ring",
            Some(other @ ("vibrate" | "silent")) => other,
            _ => "",
        };
        let ringer = QString::from(ringer);
        if self.ringer != ringer {
            self.as_mut().rust_mut().ringer = ringer;
            self.as_mut().ringer_changed();
        }
        let dnd = controls.as_ref().is_some_and(|controls| controls.dnd == 1);
        if self.dnd != dnd {
            self.as_mut().rust_mut().dnd = dnd;
            self.as_mut().dnd_changed();
        }
        let media_volume = controls.as_ref().map_or(-1, |controls| controls.media_volume);
        if self.media_volume != media_volume {
            self.as_mut().rust_mut().media_volume = media_volume;
            self.as_mut().media_volume_changed();
        }
        let player_ref = player.as_ref();
        self.as_mut().set_has_player(player.is_some());
        self.as_mut().set_player_title(QString::from(player_ref.map_or("", |player| player.title.as_str())));
        self.as_mut().set_player_artist(QString::from(player_ref.map_or("", |player| player.artist.as_str())));
        self.as_mut().set_player_app(QString::from(player_ref.map_or("", |player| player.app.as_str())));
        self.as_mut().set_player_art(QUrl::from(player_ref.map_or("", |player| player.art.as_str())));
        self.as_mut().set_playing(player_ref.is_some_and(|player| player.playing));
        self.as_mut().set_position_ms(player_ref.map_or(0, |player| player.position / 1000));
        self.as_mut().set_length_ms(player_ref.map_or(0, |player| player.length / 1000));
        #[expect(clippy::cast_possible_truncation, reason = "a volume in 0.0..=1.0 rounds into 0..=100")]
        let volume = player_ref.map_or(-1, |player| (player.volume.clamp(0.0, 1.0) * 100.0).round() as i32);
        if self.player_volume != volume {
            self.as_mut().rust_mut().player_volume = volume;
            self.as_mut().player_volume_changed();
        }
    }

    /// Gives the tray icon the current phone's state, on desktops that show one.
    fn show_in_tray(self: Pin<&Self>) {
        if !tray::wanted() {
            return;
        }
        let id = self.current_phone.to_string();
        let name = self.devices.iter().find(|device| device.id == id).map(|device| device.name.clone());
        tray::show(tray::State {
            name: name.unwrap_or_default(),
            connected: self.connected,
            battery: self.battery,
            last_seen: self.last_seen,
            dnd: self.dnd,
            ringer: self.ringer.to_string(),
        });
    }
}

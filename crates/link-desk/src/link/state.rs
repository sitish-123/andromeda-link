use std::collections::HashMap;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QMap, QString, QUrl, QVariant};

use super::qobject;
use crate::client::{Event, Notification};

pub(super) struct Recent {
    pub(super) device: String,
    pub(super) name: String,
    pub(super) detail: String,
    pub(super) at: u64,
    pub(super) thumb: String,
    /// photo, file, or clip.
    pub(super) kind: &'static str,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

fn age_text(at: u64) -> String {
    match now().saturating_sub(at) / 60 {
        0 => "now".to_owned(),
        minutes @ 1..60 => format!("{minutes} min"),
        minutes @ 60..1440 => format!("{} h", minutes / 60),
        minutes => format!("{} d", minutes / 1440),
    }
}

fn file_url(path: &str) -> String {
    format!("file://{path}")
}

fn is_image(name: &str) -> bool {
    let extension = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "avif" | "bmp")
}

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

fn variant<T: cxx_qt_lib::QVariantValue>(value: &T) -> QVariant {
    QVariant::from(value)
}

fn text(value: &str) -> QVariant {
    variant(&QString::from(value))
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
            Event::Offered { transfer, device } => {
                state.offered.insert(transfer, device);
            }
            Event::Sending { transfer, device, files } => {
                state.sending.insert(transfer, (device, files));
            }
            Event::Finished { transfer, status, paths } => {
                let incoming = state.offered.remove(&transfer);
                let outgoing = state.sending.remove(&transfer);
                if status == "done" {
                    let at = now();
                    if let Some(device) = incoming {
                        for path in paths {
                            let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
                            let photo = is_image(&name);
                            state.history.push(Recent {
                                device: device.clone(),
                                detail: "Received".to_owned(),
                                thumb: if photo { file_url(&path) } else { String::new() },
                                kind: if photo { "photo" } else { "file" },
                                name,
                                at,
                            });
                        }
                    }
                    if let Some((device, files)) = outgoing {
                        for name in files {
                            let kind = if is_image(&name) { "photo" } else { "file" };
                            let detail = "Sent".to_owned();
                            state.history.push(Recent {
                                device: device.clone(),
                                name,
                                detail,
                                at,
                                thumb: String::new(),
                                kind,
                            });
                        }
                    }
                }
            }
            Event::Received { device, text } => {
                let name = text.lines().next().unwrap_or_default().to_owned();
                let detail = "Copied from phone".to_owned();
                state.history.push(Recent { device, name, detail, at: now(), thumb: String::new(), kind: "clip" });
            }
            Event::ClipSent(text) => {
                let name = text.lines().next().unwrap_or_default().to_owned();
                let device = state.current_phone.to_string();
                let detail = "Sent clipboard".to_owned();
                state.history.push(Recent { device, name, detail, at: now(), thumb: String::new(), kind: "clip" });
            }
            Event::Player { device, player } => match player {
                Some(player) => drop(state.players.insert(device, player)),
                None => drop(state.players.remove(&device)),
            },
            Event::Tick => {}
            Event::Failed(message) => {
                self.as_mut().failed(QString::from(&message));
                return;
            }
        }
        self.refresh();
    }

    /// Recomputes every property and both models from the state, for the current phone.
    pub(super) fn refresh(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        let current = state.current_phone.to_string();
        if !state.devices.iter().any(|device| device.id == current) {
            let pick = state.devices.iter().find(|device| device.connected).or(state.devices.first());
            state.current_phone = QString::from(pick.map_or("", |device| device.id.as_str()));
            self.as_mut().current_phone_changed();
            state = self.as_mut().rust_mut();
        }
        let id = state.current_phone.to_string();

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
        let recent = state
            .history
            .iter()
            .rev()
            .filter(|item| item.device == id)
            .map(|item| {
                vec![text(&item.name), text(&item.detail), text(&age_text(item.at)), text(&item.thumb), text(item.kind)]
            })
            .collect();
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
}

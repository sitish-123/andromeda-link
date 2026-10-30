use std::path::PathBuf;
use std::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QString, QUrl};

use super::qobject;
use crate::client::{Command, PlayerAction};
use crate::hub;

impl qobject::Link {
    pub(super) fn send(self: Pin<&mut Self>, command: Command) {
        hub::send(command);
    }

    /// Sends the command built for the current phone, or says why there is none.
    pub(super) fn to_phone(mut self: Pin<&mut Self>, command: impl FnOnce(String) -> Command) {
        let id = self.current_phone.to_string();
        if id.is_empty() {
            self.as_mut().failed(QString::from("No phone is paired"));
            return;
        }
        self.send(command(id));
    }

    pub(super) fn choose_phone(mut self: Pin<&mut Self>, id: QString) {
        if self.current_phone == id {
            return;
        }
        self.as_mut().rust_mut().current_phone = id;
        self.as_mut().current_phone_changed();
        self.refresh();
    }

    pub(super) fn ring(self: Pin<&mut Self>) {
        self.to_phone(Command::Ring);
    }

    pub(super) fn send_clipboard(self: Pin<&mut Self>) {
        let text = qobject::clipboard_text().to_string();
        self.send(Command::OfferClipboard(text));
    }

    pub(super) fn send_files(self: Pin<&mut Self>, urls: &QList<QUrl>) {
        let paths: Vec<PathBuf> =
            urls.iter().filter_map(QUrl::to_local_file).map(|path| PathBuf::from(path.to_string())).collect();
        self.to_phone(|device| Command::SendFiles { device, paths });
    }

    pub(super) fn set_ringer(self: Pin<&mut Self>, mode: &QString) {
        let value = match mode.to_string().as_str() {
            "silent" => 0,
            "vibrate" => 1,
            _ => 2,
        };
        self.to_phone(|device| Command::System { device, command: "set-ringer", value });
    }

    pub(super) fn set_dnd(self: Pin<&mut Self>, on: bool) {
        self.to_phone(|device| Command::System { device, command: "set-dnd", value: u32::from(on) });
    }

    pub(super) fn set_media_volume(self: Pin<&mut Self>, percent: i32) {
        let value = u32::try_from(percent.clamp(0, 100)).unwrap_or(0);
        self.to_phone(|device| Command::System { device, command: "set-media-volume", value });
    }

    pub(super) fn play_pause(self: Pin<&mut Self>) {
        self.to_phone(|device| Command::Player { device, action: PlayerAction::PlayPause });
    }

    pub(super) fn next(self: Pin<&mut Self>) {
        self.to_phone(|device| Command::Player { device, action: PlayerAction::Next });
    }

    pub(super) fn previous(self: Pin<&mut Self>) {
        self.to_phone(|device| Command::Player { device, action: PlayerAction::Previous });
    }

    pub(super) fn seek(self: Pin<&mut Self>, ms: i64) {
        self.to_phone(|device| Command::Player { device, action: PlayerAction::SetPosition(ms.saturating_mul(1000)) });
    }

    pub(super) fn set_player_volume(self: Pin<&mut Self>, percent: i32) {
        let volume = f64::from(percent.clamp(0, 100)) / 100.0;
        self.to_phone(|device| Command::Player { device, action: PlayerAction::SetVolume(volume) });
    }

    pub(super) fn reply(self: Pin<&mut Self>, key: &QString, text: &QString) {
        let (id, text) = (key.to_string(), text.to_string());
        let device = self.current_phone.to_string();
        let action = self
            .posted
            .iter()
            .find(|(notification, _)| notification.device == device && notification.id == id)
            .and_then(|(notification, _)| notification.actions.iter().find(|(_, _, reply)| *reply))
            .map(|(action, _, _)| action.clone());
        match action {
            Some(action) => self.send(Command::Reply { device, id, action, text }),
            None => self.failed(QString::from("This notification takes no reply")),
        }
    }

    pub(super) fn dismiss(mut self: Pin<&mut Self>, key: &QString) {
        let id = key.to_string();
        let device = self.current_phone.to_string();
        self.as_mut().rust_mut().posted.retain(|(old, _)| old.device != device || old.id != id);
        self.as_mut().send(Command::Dismiss { device, id });
        self.refresh();
    }

    pub(super) fn clear_notifications(mut self: Pin<&mut Self>) {
        let device = self.current_phone.to_string();
        let (cleared, kept) =
            std::mem::take(&mut self.as_mut().rust_mut().posted).into_iter().partition(|(old, _)| old.device == device);
        self.as_mut().rust_mut().posted = kept;
        for (notification, _) in cleared {
            self.as_mut().send(Command::Dismiss { device: device.clone(), id: notification.id });
        }
        self.refresh();
    }

    /// The service reconnects to known phones on its own and has no call to force it; this starts the service
    /// when it is down.
    pub(super) fn connect_phone(mut self: Pin<&mut Self>) {
        if self.service_running {
            self.as_mut().failed(QString::from("The phone reconnects when it is on the same network"));
        } else {
            self.start_service();
        }
    }

    pub(super) fn start_service(self: Pin<&mut Self>) {
        self.send(Command::StartService);
    }
}

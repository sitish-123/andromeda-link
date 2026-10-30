//! `LinkSettings`: grants, auto accept, and the service's switches, for every paired phone; follows no phone.

use std::pin::Pin;
use std::process::{Command as Process, Stdio};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};

use crate::client::{Command, Event};
use crate::format::{text, variant};
use crate::hub;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qmap.h");
        type QMap_QString_QVariant = cxx_qt_lib::QMap<cxx_qt_lib::QMapPair_QString_QVariant>;
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QMap_QString_QVariant, grants)]
        #[qproperty(QStringList, auto_accept, READ, NOTIFY)]
        #[qproperty(bool, local_send_visible, READ, NOTIFY)]
        #[qproperty(bool, quick_share_visible, READ, NOTIFY)]
        #[qproperty(bool, notifications_while_locked, READ, NOTIFY)]
        #[qproperty(QStringList, notifications_off)]
        #[qproperty(QList_QVariant, commands, READ, NOTIFY)]
        #[qproperty(QString, helper_status, READ, NOTIFY)]
        type LinkSettings = super::SettingsRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qinvokable]
        fn set_grant(self: Pin<&mut LinkSettings>, id: &QString, feature: &QString, on: bool);
        #[qinvokable]
        fn set_auto_accept(self: Pin<&mut LinkSettings>, id: &QString, on: bool);
        #[qinvokable]
        fn set_local_send_visible(self: Pin<&mut LinkSettings>, on: bool);
        #[qinvokable]
        fn set_quick_share_visible(self: Pin<&mut LinkSettings>, on: bool);
        #[qinvokable]
        fn set_notifications_while_locked(self: Pin<&mut LinkSettings>, on: bool);
        #[qinvokable]
        fn set_commands(self: Pin<&mut LinkSettings>, list: &QList_QVariant);
        #[qinvokable]
        fn unpair(self: Pin<&mut LinkSettings>, id: &QString);
        #[qinvokable]
        fn start_helper(self: Pin<&mut LinkSettings>, name: &QString);
    }

    impl cxx_qt::Threading for LinkSettings {}
    impl cxx_qt::Initialize for LinkSettings {}
}

#[derive(Default)]
pub struct SettingsRust {
    grants: QMap<QMapPair_QString_QVariant>,
    auto_accept: QStringList,
    local_send_visible: bool,
    quick_share_visible: bool,
    notifications_while_locked: bool,
    notifications_off: QStringList,
    commands: QList<QVariant>,
    helper_status: QString,
}

fn strings(list: &[String]) -> QStringList {
    list.iter().map(QString::from).collect()
}

impl cxx_qt::Initialize for qobject::LinkSettings {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            if matches!(
                event,
                Event::Grants(_)
                    | Event::AutoAccept(_)
                    | Event::LocalSend(_)
                    | Event::QuickShare(_)
                    | Event::NotifyLocked(_)
                    | Event::NotificationsOff(_)
                    | Event::Commands(_)
            ) {
                let event = event.clone();
                let _ = thread.queue(move |settings| settings.apply(event));
            }
        });
    }
}

impl qobject::LinkSettings {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        match event {
            Event::Grants(grants) => {
                let mut map = QMap::<QMapPair_QString_QVariant>::default();
                for (id, features) in grants {
                    map.insert(QString::from(&id), variant(&strings(&features)));
                }
                self.as_mut().set_grants(map);
            }
            Event::AutoAccept(ids) => {
                self.as_mut().rust_mut().auto_accept = strings(&ids);
                self.as_mut().auto_accept_changed();
            }
            Event::LocalSend(on) => {
                self.as_mut().rust_mut().local_send_visible = on;
                self.as_mut().local_send_visible_changed();
            }
            Event::QuickShare(on) => {
                self.as_mut().rust_mut().quick_share_visible = on;
                self.as_mut().quick_share_visible_changed();
            }
            Event::NotifyLocked(on) => {
                self.as_mut().rust_mut().notifications_while_locked = on;
                self.as_mut().notifications_while_locked_changed();
            }
            Event::NotificationsOff(ids) => self.as_mut().set_notifications_off(strings(&ids)),
            Event::Commands(commands) => {
                let mut list = QList::default();
                for (name, command) in commands {
                    let mut row = QMap::<QMapPair_QString_QVariant>::default();
                    row.insert(QString::from("name"), text(&name));
                    row.insert(QString::from("command"), text(&command));
                    list.append(variant(&row));
                }
                self.as_mut().rust_mut().commands = list;
                self.as_mut().commands_changed();
            }
            _ => {}
        }
    }

    fn set_grant(self: Pin<&mut Self>, id: &QString, feature: &QString, on: bool) {
        let (device, feature) = (id.to_string(), feature.to_string());
        hub::send(Command::SetGrant { device, feature, granted: on });
    }

    fn set_auto_accept(self: Pin<&mut Self>, id: &QString, on: bool) {
        hub::send(Command::SetAutoAccept { device: id.to_string(), enabled: on });
    }

    fn set_local_send_visible(self: Pin<&mut Self>, on: bool) {
        hub::send(Command::SetLocalSend(on));
    }

    fn set_quick_share_visible(self: Pin<&mut Self>, on: bool) {
        hub::send(Command::SetQuickShare(on));
    }

    fn set_notifications_while_locked(self: Pin<&mut Self>, on: bool) {
        hub::send(Command::SetNotifyLocked(on));
    }

    fn set_commands(self: Pin<&mut Self>, list: &QList<QVariant>) {
        let field = |row: &QMap<QMapPair_QString_QVariant>, key: &str| {
            row.get(&QString::from(key)).and_then(|value| value.value::<QString>()).map(|value| value.to_string())
        };
        let commands = list
            .iter()
            .filter_map(QVariant::value::<QMap<QMapPair_QString_QVariant>>)
            .filter_map(|row| Some((field(&row, "name")?, field(&row, "command")?)))
            .collect();
        hub::send(Command::SetCommands(commands));
    }

    fn unpair(self: Pin<&mut Self>, id: &QString) {
        hub::send(Command::Unpair(id.to_string()));
    }

    /// Starts the phone's clipboard helper through `andromeda-link-apps`, which opens its pairing window when the
    /// phone is not paired for wireless debugging yet.
    fn start_helper(mut self: Pin<&mut Self>, name: &QString) {
        self.as_mut().show_helper("Starting…");
        let (thread, name) = (self.qt_thread(), name.to_string());
        std::thread::spawn(move || {
            let apps = std::env::current_exe()
                .ok()
                .and_then(|exe| Some(exe.parent()?.join("andromeda-link-apps")))
                .filter(|path| path.exists())
                .map_or("andromeda-link-apps".into(), std::path::PathBuf::into_os_string);
            let status = match Process::new(apps).arg(&name).arg("--helper").stdin(Stdio::null()).output() {
                Ok(output) if String::from_utf8_lossy(&output.stdout).contains("runs on the phone") => {
                    "Running until the phone restarts".to_owned()
                }
                Ok(output) => String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .rfind(|line| !line.trim().is_empty())
                    .map_or("Not started", |line| line.split_once("] ").map_or(line, |(_, text)| text))
                    .to_owned(),
                Err(error) => format!("andromeda-link-apps: {error}"),
            };
            let _ = thread.queue(move |settings| settings.show_helper(&status));
        });
    }

    fn show_helper(mut self: Pin<&mut Self>, status: &str) {
        self.as_mut().rust_mut().helper_status = QString::from(status);
        self.as_mut().helper_status_changed();
    }
}

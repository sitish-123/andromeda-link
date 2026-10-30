//! The `Link` `QObject` QML binds to, and the list models it owns; state comes from the client thread.

mod commands;
pub mod list_model;
use list_model::{ListModelRust, NOTIFICATION_ROLES, RECENT_ROLES};
mod state;

use std::collections::HashMap;
use std::pin::Pin;

use crate::client::{Controls, Device, Event, Notification, Player, Status};
use crate::history::History;
use crate::hub;
use crate::tray;
use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QString, QUrl, QVariant};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
        type QList_QUrl = cxx_qt_lib::QList<cxx_qt_lib::QUrl>;
        type QList_i32 = cxx_qt_lib::QList<i32>;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("link-desk/src/clipboard.h");
        #[cxx_name = "clipboardText"]
        fn clipboard_text() -> QString;
    }

    extern "C++Qt" {
        include!(<QtCore/QAbstractListModel>);
        #[qobject]
        type QAbstractListModel;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qml_uncreatable]
        type ListModel = super::ListModelRust;

        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, service_running)]
        #[qproperty(QList_QVariant, phones)]
        #[qproperty(QString, current_phone, READ, WRITE = choose_phone, NOTIFY)]
        #[qproperty(bool, connected)]
        #[qproperty(i64, last_seen)]
        #[qproperty(i32, battery)]
        #[qproperty(bool, charging)]
        #[qproperty(QString, transport)]
        #[qproperty(QString, network)]
        #[qproperty(i32, signal)]
        #[qproperty(QString, ringer, READ, NOTIFY)]
        #[qproperty(bool, dnd, READ, NOTIFY)]
        #[qproperty(i32, media_volume, READ, NOTIFY)]
        #[qproperty(QString, player_title)]
        #[qproperty(QString, player_artist)]
        #[qproperty(QString, player_app)]
        #[qproperty(QUrl, player_art)]
        #[qproperty(bool, playing)]
        #[qproperty(i64, position_ms)]
        #[qproperty(i64, length_ms)]
        #[qproperty(i32, player_volume, READ, NOTIFY)]
        #[qproperty(bool, has_player)]
        #[qproperty(*mut ListModel, notifications, READ, CONSTANT)]
        #[qproperty(*mut ListModel, recent, READ, CONSTANT)]
        type Link = super::LinkRust;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[cxx_override]
        fn data(self: &ListModel, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        fn role_names(self: &ListModel) -> QHash_i32_QByteArray;
        #[cxx_override]
        fn row_count(self: &ListModel, parent: &QModelIndex) -> i32;
        #[inherit]
        fn begin_reset_model(self: Pin<&mut ListModel>);
        #[inherit]
        fn end_reset_model(self: Pin<&mut ListModel>);
        #[inherit]
        fn index(self: &ListModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        #[inherit]
        #[qsignal]
        fn data_changed(
            self: Pin<&mut ListModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

        fn choose_phone(self: Pin<&mut Link>, id: QString);
        #[qinvokable]
        fn ring(self: Pin<&mut Link>);
        #[qinvokable]
        fn send_clipboard(self: Pin<&mut Link>);
        #[qinvokable]
        fn send_files(self: Pin<&mut Link>, urls: &QList_QUrl);
        #[qinvokable]
        fn set_ringer(self: Pin<&mut Link>, mode: &QString);
        #[qinvokable]
        fn set_dnd(self: Pin<&mut Link>, on: bool);
        #[qinvokable]
        fn set_media_volume(self: Pin<&mut Link>, percent: i32);
        #[qinvokable]
        fn play_pause(self: Pin<&mut Link>);
        #[qinvokable]
        fn next(self: Pin<&mut Link>);
        #[qinvokable]
        fn previous(self: Pin<&mut Link>);
        #[qinvokable]
        fn seek(self: Pin<&mut Link>, ms: i64);
        #[qinvokable]
        fn set_player_volume(self: Pin<&mut Link>, percent: i32);
        #[qinvokable]
        fn reply(self: Pin<&mut Link>, key: &QString, text: &QString);
        #[qinvokable]
        fn dismiss(self: Pin<&mut Link>, key: &QString);
        #[qinvokable]
        fn clear_notifications(self: Pin<&mut Link>);
        #[qinvokable]
        fn connect_phone(self: Pin<&mut Link>);
        #[qinvokable]
        fn start_service(self: Pin<&mut Link>);
        #[qsignal]
        fn failed(self: Pin<&mut Link>, message: QString);
        /// The tray asks the window to show and come to the front.
        #[qsignal]
        fn raise_requested(self: Pin<&mut Link>);
    }

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");
        #[rust_name = "new_list_model"]
        fn make_unique() -> UniquePtr<ListModel>;
    }

    impl cxx_qt::Threading for Link {}
    impl cxx_qt::Initialize for Link {}
}

#[expect(clippy::struct_excessive_bools, reason = "the properties of the QML contract")]
pub struct LinkRust {
    service_running: bool,
    phones: QList<QVariant>,
    current_phone: QString,
    connected: bool,
    last_seen: i64,
    battery: i32,
    charging: bool,
    transport: QString,
    network: QString,
    signal: i32,
    ringer: QString,
    dnd: bool,
    media_volume: i32,
    player_title: QString,
    player_artist: QString,
    player_app: QString,
    player_art: QUrl,
    playing: bool,
    position_ms: i64,
    length_ms: i64,
    player_volume: i32,
    has_player: bool,
    notifications: *mut qobject::ListModel,
    recent: *mut qobject::ListModel,

    notification_model: UniquePtr<qobject::ListModel>,
    recent_model: UniquePtr<qobject::ListModel>,
    devices: Vec<Device>,
    /// Device id to (last seen unix seconds, transport), from the service's `DeviceInfo`.
    info: HashMap<String, (i64, String)>,
    status: HashMap<String, Status>,
    controls: HashMap<String, Controls>,
    players: HashMap<String, Player>,
    /// Newest last, with the unix seconds each arrived.
    posted: Vec<(Notification, u64)>,
    history: History,
}

impl Default for LinkRust {
    fn default() -> Self {
        Self {
            service_running: false,
            phones: QList::default(),
            current_phone: QString::default(),
            connected: false,
            last_seen: 0,
            battery: -1,
            charging: false,
            transport: QString::default(),
            network: QString::default(),
            signal: -1,
            ringer: QString::default(),
            dnd: false,
            media_volume: -1,
            player_title: QString::default(),
            player_artist: QString::default(),
            player_app: QString::default(),
            player_art: QUrl::default(),
            playing: false,
            position_ms: 0,
            length_ms: 0,
            player_volume: -1,
            has_player: false,
            notifications: std::ptr::null_mut(),
            recent: std::ptr::null_mut(),
            notification_model: UniquePtr::null(),
            recent_model: UniquePtr::null(),
            devices: Vec::new(),
            info: HashMap::new(),
            status: HashMap::new(),
            controls: HashMap::new(),
            players: HashMap::new(),
            posted: Vec::new(),
            history: History::default(),
        }
    }
}

impl cxx_qt::Initialize for qobject::Link {
    fn initialize(mut self: Pin<&mut Self>) {
        let notifications = new_list_model(NOTIFICATION_ROLES);
        let recent = new_list_model(RECENT_ROLES);
        let mut state = self.as_mut().rust_mut();
        state.notifications = notifications.as_mut_ptr();
        state.recent = recent.as_mut_ptr();
        state.notification_model = notifications;
        state.recent_model = recent;
        let thread = self.qt_thread();
        let events = thread.clone();
        hub::subscribe(move |event| {
            if !matches!(event, Event::Phone(_)) {
                let event = event.clone();
                let _ = events.queue(move |link| link.apply(event));
            }
        });
        // After subscribe, which starts the client runtime the tray runs on.
        if tray::wanted() {
            tray::start(thread);
        }
    }
}

/// A list model with these role names, for the page singletons too.
pub fn new_list_model(roles: &'static [&'static str]) -> UniquePtr<qobject::ListModel> {
    let mut model = qobject::new_list_model();
    if let Some(model) = model.as_mut() {
        model.rust_mut().roles = roles;
    }
    model
}

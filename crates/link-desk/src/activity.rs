//! `LinkActivity`: the current phone's transfers and clips since the app started, newest first.

use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QVariant};

use crate::client::{Command, Event};
use crate::desktop::ffi as desktop;
use crate::format::{age_text, clock, day_detail, day_name, file_url, size, text, variant};
use crate::history::{Direction, History, Item, State};
use crate::hub;
use crate::link::new_list_model;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("link-desk/src/link.cxxqt.h");
        type ListModel = crate::link::qobject::ListModel;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, filter, READ, WRITE = choose_filter, NOTIFY)]
        #[qproperty(i32, file_count)]
        #[qproperty(i32, clip_count)]
        #[qproperty(*mut ListModel, items, READ, CONSTANT)]
        type LinkActivity = super::ActivityRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        fn choose_filter(self: Pin<&mut LinkActivity>, filter: QString);
        #[qinvokable]
        fn accept(self: Pin<&mut LinkActivity>, id: &QString);
        #[qinvokable]
        fn decline(self: Pin<&mut LinkActivity>, id: &QString);
        #[qinvokable]
        fn cancel(self: Pin<&mut LinkActivity>, id: &QString);
        #[qinvokable]
        fn open(self: Pin<&mut LinkActivity>, id: &QString);
        #[qinvokable]
        fn show_in_folder(self: Pin<&mut LinkActivity>, id: &QString);
        #[qinvokable]
        fn copy_again(self: Pin<&mut LinkActivity>, id: &QString);
    }

    impl cxx_qt::Threading for LinkActivity {}
    impl cxx_qt::Initialize for LinkActivity {}
}

const ROLES: &[&str] = &[
    "id",
    "kind",
    "name",
    "detail",
    "timeText",
    "day",
    "dayDetail",
    "state",
    "progress",
    "sizeText",
    "incoming",
    "path",
    "thumbUrl",
];

pub struct ActivityRust {
    filter: QString,
    file_count: i32,
    clip_count: i32,
    items: *mut qobject::ListModel,
    model: UniquePtr<qobject::ListModel>,
    phone: String,
    /// Device id to its name.
    names: HashMap<String, String>,
    history: History,
    /// The item id of each row on show.
    shown: Vec<String>,
}

impl Default for ActivityRust {
    fn default() -> Self {
        let model = new_list_model(ROLES);
        Self {
            filter: QString::from("all"),
            file_count: 0,
            clip_count: 0,
            items: model.as_mut_ptr(),
            model,
            phone: hub::phone(),
            names: HashMap::new(),
            history: History::default(),
            shown: Vec::new(),
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkActivity {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            let wanted = matches!(
                event,
                Event::Offered { .. }
                    | Event::Sending { .. }
                    | Event::Progress { .. }
                    | Event::Finished { .. }
                    | Event::Received { .. }
                    | Event::ClipSent(_)
                    | Event::Fetched { .. }
                    | Event::Devices(_)
                    | Event::Phone(_)
                    | Event::Tick
            );
            if wanted {
                let event = event.clone();
                let _ = thread.queue(move |activity| activity.apply(event));
            }
        });
    }
}

fn state_name(state: &State) -> &'static str {
    match state {
        State::Offered => "offered",
        State::Active => "active",
        State::Done => "done",
        State::Failed(_) => "failed",
    }
}

#[expect(clippy::cast_precision_loss, reason = "a fraction for a progress bar")]
fn progress(item: &Item) -> f64 {
    if item.total == 0 { 0.0 } else { item.bytes as f64 / item.total as f64 }
}

fn size_text(item: &Item) -> String {
    match (&item.state, item.total) {
        (_, 0) => String::new(),
        (State::Active, total) => format!("{} of {}", size(item.bytes), size(total)),
        (_, total) => size(total),
    }
}

impl ActivityRust {
    fn row(&self, item: &Item) -> Vec<QVariant> {
        let phone = self.names.get(&item.device).map_or("the phone", String::as_str);
        let detail = match (&item.state, item.direction) {
            (State::Failed(reason), _) => format!("Failed, {reason}"),
            (_, Direction::Received) => format!("Received from {phone}"),
            (_, Direction::Sent) => format!("Sent to {phone}"),
            (_, Direction::CopiedFrom) => format!("Copied from {phone}"),
            (_, Direction::CopiedTo) => format!("Copied to {phone}"),
            (_, Direction::Saved) => format!("Saved from {phone}"),
        };
        let name =
            if item.count > 1 { format!("{} and {} more", item.name, item.count - 1) } else { item.name.clone() };
        let today = day_name(item.at) == "Today";
        let thumb = if item.kind() == "photo" && !item.path.is_empty() { file_url(&item.path) } else { String::new() };
        vec![
            text(&item.id),
            text(item.kind()),
            text(&name),
            text(&detail),
            text(&if today { age_text(item.at) } else { clock(item.at) }),
            text(&day_name(item.at)),
            text(&day_detail(item.at)),
            text(state_name(&item.state)),
            variant(&progress(item)),
            text(&size_text(item)),
            variant(&matches!(item.direction, Direction::Received | Direction::CopiedFrom | Direction::Saved)),
            text(&item.path),
            text(&thumb),
        ]
    }

    fn item(&self, id: &QString) -> Option<&Item> {
        let id = id.to_string();
        self.history.items.iter().find(|item| item.id == id)
    }
}

impl qobject::LinkActivity {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        let mut state = self.as_mut().rust_mut();
        match event {
            Event::Devices(devices) => {
                state.names = devices.into_iter().map(|device| (device.id, device.name)).collect();
            }
            Event::Phone(id) => state.phone = id,
            Event::Tick => {}
            Event::Progress { ref transfer, .. } => {
                let phone = state.phone.clone();
                state.history.apply(&event, &phone);
                if let Some(row) = state.shown.iter().position(|id| id == transfer) {
                    let Some(item) = state.history.items.iter().find(|item| item.id == *transfer) else { return };
                    let cells = [
                        ("state", text("active")),
                        ("progress", variant(&progress(item))),
                        ("sizeText", text(&size_text(item))),
                    ];
                    if let Some(mut model) = state.model.as_mut() {
                        for (role, value) in cells {
                            model.as_mut().set(row, role, value);
                        }
                    }
                    return;
                }
            }
            other => {
                let phone = state.phone.clone();
                if !state.history.apply(&other, &phone) {
                    return;
                }
            }
        }
        self.refresh();
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        let filter = state.filter.to_string();
        let mine: Vec<&Item> = state.history.items.iter().rev().filter(|item| item.device == state.phone).collect();
        let clips = mine.iter().filter(|item| item.kind() == "clip").count();
        let files = mine.len() - clips;
        let shown: Vec<&Item> = mine
            .into_iter()
            .filter(|item| match filter.as_str() {
                "files" => item.kind() != "clip",
                "clips" => item.kind() == "clip",
                _ => true,
            })
            .collect();
        let rows = shown.iter().map(|item| state.row(item)).collect();
        let ids = shown.iter().map(|item| item.id.clone()).collect();
        state.shown = ids;
        if let Some(model) = state.model.as_mut() {
            model.replace(rows);
        }
        self.as_mut().set_file_count(i32::try_from(files).unwrap_or(i32::MAX));
        self.as_mut().set_clip_count(i32::try_from(clips).unwrap_or(i32::MAX));
    }

    fn choose_filter(mut self: Pin<&mut Self>, filter: QString) {
        if self.filter != filter {
            self.as_mut().rust_mut().filter = filter;
            self.as_mut().filter_changed();
            self.refresh();
        }
    }

    fn transfer(&self, id: &QString, command: fn(String) -> Command) {
        match self.item(id).map(|item| item.transfer.clone()).filter(|transfer| !transfer.is_empty()) {
            Some(transfer) => hub::send(command(transfer)),
            None => hub::publish(Event::Failed("This transfer has ended".to_owned())),
        }
    }

    fn accept(self: Pin<&mut Self>, id: &QString) {
        self.transfer(id, Command::Accept);
    }

    fn decline(self: Pin<&mut Self>, id: &QString) {
        self.transfer(id, Command::Decline);
    }

    fn cancel(self: Pin<&mut Self>, id: &QString) {
        self.transfer(id, Command::Cancel);
    }

    fn open(self: Pin<&mut Self>, id: &QString) {
        let path = self.item(id).map(|item| item.path.clone()).unwrap_or_default();
        if path.is_empty() || !desktop::open_path(&QString::from(&path)) {
            hub::publish(Event::Failed("No app opens this file".to_owned()));
        }
    }

    fn show_in_folder(self: Pin<&mut Self>, id: &QString) {
        if let Some(path) = self.item(id).map(|item| item.path.clone()).filter(|path| !path.is_empty()) {
            hub::send(Command::ShowInFolder(PathBuf::from(path)));
        }
    }

    fn copy_again(self: Pin<&mut Self>, id: &QString) {
        if let Some(clip) = self.item(id).map(|item| item.text.clone()).filter(|text| !text.is_empty()) {
            desktop::set_clipboard_text(&QString::from(&clip));
            hub::send(Command::OfferClipboard(clip));
        }
    }
}

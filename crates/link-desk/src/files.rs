//! `LinkFiles`: the current phone's storage, one folder at a time.

use std::collections::HashMap;
use std::pin::Pin;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};

use crate::client::{Command, Entry, Event, Fetch};
use crate::desktop;
use crate::format::{file_kind, modified, size, text, variant};
use crate::hub;
use crate::link::new_list_model;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qlist.h");
        type QList_QVariant = cxx_qt_lib::QList<cxx_qt_lib::QVariant>;
        include!("link-desk/src/link.cxxqt.h");
        type ListModel = crate::link::qobject::ListModel;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, path)]
        #[qproperty(QList_QVariant, crumbs)]
        #[qproperty(bool, loading)]
        #[qproperty(QString, error)]
        #[qproperty(*mut ListModel, entries, READ, CONSTANT)]
        type LinkFiles = super::FilesRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qinvokable]
        fn cd(self: Pin<&mut LinkFiles>, path: &QString);
        #[qinvokable]
        fn up(self: Pin<&mut LinkFiles>);
        #[qinvokable]
        fn reload(self: Pin<&mut LinkFiles>);
        #[qinvokable]
        fn save(self: Pin<&mut LinkFiles>, paths: &QStringList);
        #[qinvokable]
        fn open_file(self: Pin<&mut LinkFiles>, path: &QString);
        #[qsignal]
        fn saved(self: Pin<&mut LinkFiles>, count: i32, folder: QString);
    }

    impl cxx_qt::Threading for LinkFiles {}
    impl cxx_qt::Initialize for LinkFiles {}
}

const ROLES: &[&str] = &["name", "path", "folder", "size", "sizeText", "modifiedText", "icon"];
const PAGE: &str = "files";

pub struct FilesRust {
    path: QString,
    crumbs: QList<QVariant>,
    loading: bool,
    error: QString,
    entries: *mut qobject::ListModel,
    model: UniquePtr<qobject::ListModel>,
    phone: String,
    /// The page asked for a folder, so phone changes and reconnects list it again.
    asked: bool,
    /// Device id to connected.
    connected: HashMap<String, bool>,
}

impl Default for FilesRust {
    fn default() -> Self {
        let model = new_list_model(ROLES);
        Self {
            path: QString::from("/"),
            crumbs: QList::default(),
            loading: false,
            error: QString::default(),
            entries: model.as_mut_ptr(),
            model,
            phone: hub::phone(),
            asked: false,
            connected: HashMap::new(),
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkFiles {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().set_crumbs(crumbs("/"));
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            let wanted = match event {
                Event::Fetched { page, .. } => *page == PAGE,
                Event::Listed { .. } | Event::Devices(_) | Event::Phone(_) => true,
                _ => false,
            };
            if wanted {
                let event = event.clone();
                let _ = thread.queue(move |files| files.apply(event));
            }
        });
    }
}

/// {name, path} from "Phone" at the root to the folder.
fn crumbs(path: &str) -> QList<QVariant> {
    let mut list = QList::default();
    let mut at = String::new();
    let names = std::iter::once("Phone").chain(path.split('/').filter(|name| !name.is_empty()));
    for (n, name) in names.enumerate() {
        if n > 0 {
            at = format!("{at}/{name}");
        }
        let mut crumb = QMap::<QMapPair_QString_QVariant>::default();
        crumb.insert(QString::from("name"), text(name));
        crumb.insert(QString::from("path"), text(if at.is_empty() { "/" } else { &at }));
        list.append(variant(&crumb));
    }
    list
}

fn child(folder: &str, name: &str) -> String {
    if folder == "/" { format!("/{name}") } else { format!("{folder}/{name}") }
}

fn rows(folder: &str, mut entries: Vec<Entry>) -> Vec<Vec<QVariant>> {
    entries.sort_by_cached_key(|entry| (!entry.folder, entry.name.to_lowercase()));
    entries
        .into_iter()
        .map(|entry| {
            let icon = if entry.folder { "folder" } else { file_kind(&entry.name) };
            vec![
                text(&entry.name),
                text(&child(folder, &entry.name)),
                variant(&entry.folder),
                variant(&i64::try_from(entry.size).unwrap_or(i64::MAX)),
                text(&if entry.folder { String::new() } else { size(entry.size) }),
                text(&if entry.mtime == 0 { String::new() } else { modified(entry.mtime) }),
                text(icon),
            ]
        })
        .collect()
}

impl qobject::LinkFiles {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        let mut state = self.as_mut().rust_mut();
        match event {
            Event::Phone(id) => {
                state.phone = id;
                if state.asked {
                    self.cd(&QString::from("/"));
                }
            }
            Event::Devices(devices) => {
                let was = state.connected.get(&state.phone).copied().unwrap_or(false);
                state.connected = devices.into_iter().map(|device| (device.id, device.connected)).collect();
                let now = state.connected.get(&state.phone).copied().unwrap_or(false);
                if now && !was && state.asked {
                    self.reload();
                }
            }
            Event::Listed { device, path, entries } if device == state.phone && path == state.path.to_string() => {
                let rows = match entries {
                    Ok(entries) => {
                        self.as_mut().set_error(QString::default());
                        rows(&path, entries)
                    }
                    Err(reason) => {
                        self.as_mut().set_error(QString::from(&reason));
                        Vec::new()
                    }
                };
                if let Some(model) = self.as_mut().rust_mut().model.as_mut() {
                    model.replace(rows);
                }
                self.set_loading(false);
            }
            Event::Fetched { purpose, paths, .. } => {
                let folder = desktop::fetched(purpose, &paths);
                if purpose == Fetch::Save {
                    self.saved(i32::try_from(paths.len()).unwrap_or(i32::MAX), folder);
                }
            }
            _ => {}
        }
    }

    fn cd(mut self: Pin<&mut Self>, path: &QString) {
        let text = path.to_string();
        let clean = if text.is_empty() { "/".to_owned() } else { text.trim_end_matches('/').to_owned() };
        let clean = if clean.is_empty() { "/".to_owned() } else { clean };
        self.as_mut().set_crumbs(crumbs(&clean));
        self.as_mut().set_path(QString::from(&clean));
        self.reload();
    }

    fn up(self: Pin<&mut Self>) {
        let path = self.path.to_string();
        let parent = path.rsplit_once('/').map_or("/", |(parent, _)| if parent.is_empty() { "/" } else { parent });
        self.cd(&QString::from(parent));
    }

    fn reload(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        state.asked = true;
        if state.phone.is_empty() {
            return;
        }
        let (device, path) = (state.phone.clone(), state.path.to_string());
        hub::send(Command::List { device, path });
        self.set_loading(true);
    }

    fn save(self: Pin<&mut Self>, paths: &QStringList) {
        desktop::fetch(PAGE, paths, Fetch::Save);
    }

    fn open_file(self: Pin<&mut Self>, path: &QString) {
        desktop::fetch(PAGE, &QStringList::from(path), Fetch::Open);
    }
}

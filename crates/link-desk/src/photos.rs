//! `LinkPhotos`: every album under the current phone's /Photos as one grid, newest first, with thumbnails.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList, QVariant};

use crate::client::{Command, Entry, Event, Fetch};
use crate::desktop;
use crate::format::{day_text, file_kind, file_url, text, variant};
use crate::hub;
use crate::link::new_list_model;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("link-desk/src/link.cxxqt.h");
        type ListModel = crate::link::qobject::ListModel;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(bool, loading)]
        #[qproperty(QString, filter, READ, WRITE = choose_filter, NOTIFY)]
        #[qproperty(QString, error)]
        #[qproperty(*mut ListModel, photos, READ, CONSTANT)]
        type LinkPhotos = super::PhotosRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        fn choose_filter(self: Pin<&mut LinkPhotos>, filter: QString);
        #[qinvokable]
        fn reload(self: Pin<&mut LinkPhotos>);
        #[qinvokable]
        fn want(self: Pin<&mut LinkPhotos>, path: &QString);
        #[qinvokable]
        fn copy(self: Pin<&mut LinkPhotos>, paths: &QStringList);
        #[qinvokable]
        fn save(self: Pin<&mut LinkPhotos>, paths: &QStringList);
        #[qsignal]
        fn saved(self: Pin<&mut LinkPhotos>, count: i32, folder: QString);
    }

    impl cxx_qt::Threading for LinkPhotos {}
    impl cxx_qt::Initialize for LinkPhotos {}
}

/// The phone's root of albums, one folder each.
const ROOT: &str = "/Photos";
/// Thumbnail pixels on the longer side. A cell crops to the shorter side, up to 400 px at scale 2; a 640 x 480 RGB PNG
/// stays under `MAX_FS_READ` even with no compression.
const THUMB: u16 = 640;
/// Thumbnails asked for at once.
const IN_FLIGHT: usize = 4;
const ROLES: &[&str] = &["path", "name", "thumbUrl", "video", "durationText", "day"];
const PAGE: &str = "photos";

struct Photo {
    path: String,
    name: String,
    mtime: u64,
    thumb: String,
}

pub struct PhotosRust {
    loading: bool,
    filter: QString,
    error: QString,
    photos: *mut qobject::ListModel,
    model: UniquePtr<qobject::ListModel>,
    phone: String,
    /// The page asked for photos, so phone changes and reconnects load them again.
    asked: bool,
    /// Device id to connected.
    connected: HashMap<String, bool>,
    all: Vec<Photo>,
    /// Albums listed but not answered yet.
    pending: usize,
    /// Paths on screen without a thumbnail request yet, the next one last.
    queue: Vec<String>,
    /// Paths asked for, so a cell scrolled past twice asks once.
    asked_thumbs: HashSet<String>,
    in_flight: usize,
    /// The path of each row on show.
    shown: Vec<String>,
}

impl Default for PhotosRust {
    fn default() -> Self {
        let model = new_list_model(ROLES);
        Self {
            loading: false,
            filter: QString::from("all"),
            error: QString::default(),
            photos: model.as_mut_ptr(),
            model,
            phone: hub::phone(),
            asked: false,
            connected: HashMap::new(),
            all: Vec::new(),
            pending: 0,
            queue: Vec::new(),
            asked_thumbs: HashSet::new(),
            in_flight: 0,
            shown: Vec::new(),
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkPhotos {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            let wanted = match event {
                Event::Listed { path, .. } => path.starts_with(ROOT),
                Event::Fetched { page, .. } => *page == PAGE,
                Event::Thumbnail { .. } | Event::Devices(_) | Event::Phone(_) => true,
                _ => false,
            };
            if wanted {
                let event = event.clone();
                let _ = thread.queue(move |photos| photos.apply(event));
            }
        });
    }
}

fn is_video(name: &str) -> bool {
    file_kind(name) == "video"
}

impl qobject::LinkPhotos {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        let mut state = self.as_mut().rust_mut();
        match event {
            Event::Phone(id) => {
                state.phone = id;
                (state.all, state.queue, state.in_flight) = (Vec::new(), Vec::new(), 0);
                state.asked_thumbs.clear();
                if state.asked {
                    return self.reload();
                }
            }
            Event::Devices(devices) => {
                let was = state.connected.get(&state.phone).copied().unwrap_or(false);
                state.connected = devices.into_iter().map(|device| (device.id, device.connected)).collect();
                let now = state.connected.get(&state.phone).copied().unwrap_or(false);
                if now && !was && state.asked {
                    self.reload();
                }
                return;
            }
            Event::Listed { device, path, entries } if device == state.phone => {
                return self.listed(&path, entries);
            }
            Event::Thumbnail { device, path, file } if device == state.phone => {
                state.in_flight = state.in_flight.saturating_sub(1);
                let url = file.map(|file| file_url(&file.to_string_lossy())).unwrap_or_default();
                if let Some(photo) = state.all.iter_mut().find(|photo| photo.path == path) {
                    photo.thumb.clone_from(&url);
                }
                if let Some(row) = state.shown.iter().position(|shown| *shown == path)
                    && let Some(model) = state.model.as_mut()
                {
                    model.set(row, "thumbUrl", text(&url));
                }
                return self.ask_thumbnails();
            }
            Event::Fetched { purpose, paths, .. } => {
                let folder = desktop::fetched(purpose, &paths);
                return self.saved(i32::try_from(paths.len()).unwrap_or(i32::MAX), folder);
            }
            _ => return,
        }
        self.refresh();
    }

    fn listed(mut self: Pin<&mut Self>, path: &str, entries: Result<Vec<Entry>, String>) {
        let mut state = self.as_mut().rust_mut();
        let entries = match entries {
            Ok(entries) => entries,
            Err(reason) => {
                state.pending = state.pending.saturating_sub(1);
                if path == ROOT || state.pending == 0 {
                    state.pending = 0;
                    self.as_mut().set_error(QString::from(&reason));
                    self.as_mut().set_loading(false);
                }
                return;
            }
        };
        let (albums, files): (Vec<Entry>, Vec<Entry>) = entries.into_iter().partition(|entry| entry.folder);
        if path == ROOT {
            state.all.clear();
            state.pending = albums.len();
            for album in &albums {
                let (path, phone) = (format!("{ROOT}/{}", album.name), state.phone.clone());
                hub::send(Command::List { device: phone, path });
            }
        } else {
            state.pending = state.pending.saturating_sub(1);
        }
        for file in files {
            let path = format!("{path}/{}", file.name);
            if !state.all.iter().any(|photo| photo.path == path) {
                state.all.push(Photo { path, name: file.name, mtime: file.mtime, thumb: String::new() });
            }
        }
        if state.pending > 0 {
            return;
        }
        state.all.sort_by_key(|photo| std::cmp::Reverse(photo.mtime));
        self.as_mut().set_error(QString::default());
        self.as_mut().set_loading(false);
        self.refresh();
    }

    fn want(mut self: Pin<&mut Self>, path: &QString) {
        let mut state = self.as_mut().rust_mut();
        let path = path.to_string();
        if state.all.iter().any(|photo| photo.path == path && photo.thumb.is_empty())
            && state.asked_thumbs.insert(path.clone())
        {
            state.queue.push(path);
            self.ask_thumbnails();
        }
    }

    fn ask_thumbnails(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        while state.in_flight < IN_FLIGHT
            && let Some(path) = state.queue.pop()
        {
            state.in_flight += 1;
            hub::send(Command::Thumbnail { device: state.phone.clone(), path, size: THUMB });
        }
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        let filter = state.filter.to_string();
        let shown: Vec<&Photo> = state
            .all
            .iter()
            .filter(|photo| match filter.as_str() {
                "photos" => !is_video(&photo.name),
                "videos" => is_video(&photo.name),
                _ => true,
            })
            .collect();
        let rows: Vec<Vec<QVariant>> = shown
            .iter()
            .map(|photo| {
                vec![
                    text(&photo.path),
                    text(&photo.name),
                    text(&photo.thumb),
                    variant(&is_video(&photo.name)),
                    text(""),
                    text(&day_text(photo.mtime)),
                ]
            })
            .collect();
        state.shown = shown.iter().map(|photo| photo.path.clone()).collect();
        if let Some(model) = state.model.as_mut() {
            model.replace(rows);
        }
    }

    fn choose_filter(mut self: Pin<&mut Self>, filter: QString) {
        if self.filter != filter {
            self.as_mut().rust_mut().filter = filter;
            self.as_mut().filter_changed();
            self.refresh();
        }
    }

    fn reload(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        state.asked = true;
        state.queue.clear();
        state.asked_thumbs.clear();
        if state.phone.is_empty() {
            return;
        }
        let device = state.phone.clone();
        hub::send(Command::List { device, path: ROOT.to_owned() });
        self.as_mut().set_loading(true);
    }

    fn copy(self: Pin<&mut Self>, paths: &QStringList) {
        desktop::fetch(PAGE, paths, Fetch::Copy);
    }

    fn save(self: Pin<&mut Self>, paths: &QStringList) {
        desktop::fetch(PAGE, paths, Fetch::Save);
    }
}

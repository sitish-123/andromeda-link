//! `LinkPhotos`: every album under the current phone's /Photos as one grid, newest first, with thumbnails.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QMap, QMapPair_QString_QVariant, QString, QStringList, QVariant};

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
        /// Cells per grid row; `lines` regroups when it changes.
        #[qproperty(i32, columns, READ, WRITE = choose_columns, NOTIFY)]
        /// The grid: a day header (`day` set, no `cells`) or one row of up to `columns` photos (`cells`), with their
        /// thumbnail urls in `thumbs`, so a new thumbnail leaves `cells` and its delegates as they are.
        #[qproperty(*mut ListModel, lines, READ, CONSTANT)]
        type LinkPhotos = super::PhotosRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        fn choose_filter(self: Pin<&mut LinkPhotos>, filter: QString);
        fn choose_columns(self: Pin<&mut LinkPhotos>, columns: i32);
        #[qinvokable]
        fn reload(self: Pin<&mut LinkPhotos>);
        #[qinvokable]
        fn want(self: Pin<&mut LinkPhotos>, path: &QString);
        #[qinvokable]
        fn copy(self: Pin<&mut LinkPhotos>, paths: &QStringList);
        #[qinvokable]
        fn save(self: Pin<&mut LinkPhotos>, paths: &QStringList);
        /// Has the phone's camera take a photo; it lands in Pictures/Phone and opens.
        #[qinvokable]
        fn take_photo(self: Pin<&mut LinkPhotos>);
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
const LINE_ROLES: &[&str] = &["day", "cells", "thumbs"];
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
    columns: i32,
    lines: *mut qobject::ListModel,
    lines_model: UniquePtr<qobject::ListModel>,
    phone: String,
    /// The page asked for photos, so phone changes and reconnects load them again.
    asked: bool,
    /// Device id to connected.
    connected: HashMap<String, bool>,
    all: Vec<Photo>,
    /// Path to its index in `all`.
    index: HashMap<String, usize>,
    /// Albums listed but not answered yet.
    pending: usize,
    /// Paths on screen without a thumbnail request yet, the next one last.
    queue: Vec<String>,
    /// Paths asked for, so a cell scrolled past twice asks once.
    asked_thumbs: HashSet<String>,
    in_flight: usize,
    /// Path of each photo on show to its `photos` row and its `lines` row.
    shown: HashMap<String, (usize, usize)>,
    /// The `all` indices of each `lines` row.
    line_photos: Vec<Vec<usize>>,
}

impl Default for PhotosRust {
    fn default() -> Self {
        let model = new_list_model(ROLES);
        let lines_model = new_list_model(LINE_ROLES);
        Self {
            loading: false,
            filter: QString::from("all"),
            error: QString::default(),
            photos: model.as_mut_ptr(),
            model,
            columns: 6,
            lines: lines_model.as_mut_ptr(),
            lines_model,
            phone: hub::phone(),
            asked: false,
            connected: HashMap::new(),
            all: Vec::new(),
            index: HashMap::new(),
            pending: 0,
            queue: Vec::new(),
            asked_thumbs: HashSet::new(),
            in_flight: 0,
            shown: HashMap::new(),
            line_photos: Vec::new(),
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

/// One grid row: a map per photo with the roles a cell shows, less its thumbnail.
fn cells(all: &[Photo], photos: &[usize]) -> QVariant {
    let mut list = QList::<QVariant>::default();
    for &at in photos {
        let photo = &all[at];
        let mut cell = QMap::<QMapPair_QString_QVariant>::default();
        cell.insert(QString::from("path"), text(&photo.path));
        cell.insert(QString::from("name"), text(&photo.name));
        cell.insert(QString::from("video"), variant(&is_video(&photo.name)));
        cell.insert(QString::from("durationText"), text(""));
        list.append(variant(&cell));
    }
    variant(&list)
}

/// The thumbnail urls of one grid row.
fn thumbs(all: &[Photo], photos: &[usize]) -> QVariant {
    let mut list = QList::<QVariant>::default();
    for &at in photos {
        list.append(text(&all[at].thumb));
    }
    variant(&list)
}

impl PhotosRust {
    /// Drops the photo list and every row index into it.
    fn forget(&mut self) {
        self.all.clear();
        self.index.clear();
        self.shown.clear();
        self.line_photos.clear();
    }

    /// Stores a photo's thumbnail url. For a photo on show, gives its `photos` row, its `lines` row and that row's
    /// new `thumbs`.
    fn thumbnail(&mut self, path: &str, url: &str) -> Option<(usize, usize, QVariant)> {
        let &at = self.index.get(path)?;
        url.clone_into(&mut self.all[at].thumb);
        let &(row, line) = self.shown.get(path)?;
        Some((row, line, thumbs(&self.all, &self.line_photos[line])))
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
                (state.queue, state.in_flight) = (Vec::new(), 0);
                state.forget();
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
                if let Some((row, line, thumbs)) = state.thumbnail(&path, &url) {
                    if let Some(model) = state.model.as_mut() {
                        model.set(row, "thumbUrl", text(&url));
                    }
                    if let Some(lines) = state.lines_model.as_mut() {
                        lines.set(line, "thumbs", thumbs);
                    }
                }
                return self.ask_thumbnails();
            }
            Event::Fetched { purpose, paths, .. } => {
                let folder = desktop::fetched(purpose, &paths);
                if purpose == Fetch::Photo {
                    self.as_mut().reload();
                }
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
            state.forget();
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
            if !state.index.contains_key(&path) {
                let at = state.all.len();
                state.index.insert(path.clone(), at);
                state.all.push(Photo { path, name: file.name, mtime: file.mtime, thumb: String::new() });
            }
        }
        if state.pending > 0 {
            return;
        }
        state.all.sort_by_key(|photo| std::cmp::Reverse(photo.mtime));
        state.index = state.all.iter().enumerate().map(|(at, photo)| (photo.path.clone(), at)).collect();
        self.as_mut().set_error(QString::default());
        self.as_mut().set_loading(false);
        self.refresh();
    }

    fn want(mut self: Pin<&mut Self>, path: &QString) {
        let mut state = self.as_mut().rust_mut();
        let path = path.to_string();
        if state.index.get(&path).is_some_and(|&at| state.all[at].thumb.is_empty())
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
        let shown: Vec<usize> = (0..state.all.len())
            .filter(|&at| match filter.as_str() {
                "photos" => !is_video(&state.all[at].name),
                "videos" => is_video(&state.all[at].name),
                _ => true,
            })
            .collect();
        let days: Vec<String> = shown.iter().map(|&at| day_text(state.all[at].mtime)).collect();
        let rows: Vec<Vec<QVariant>> = shown
            .iter()
            .zip(&days)
            .map(|(&at, day)| {
                let photo = &state.all[at];
                vec![
                    text(&photo.path),
                    text(&photo.name),
                    text(&photo.thumb),
                    variant(&is_video(&photo.name)),
                    text(""),
                    text(day),
                ]
            })
            .collect();
        let columns = usize::try_from(state.columns.max(1)).unwrap_or(1);
        let mut line_photos: Vec<Vec<usize>> = Vec::new();
        let mut lines: Vec<Vec<QVariant>> = Vec::new();
        let mut map = HashMap::new();
        let mut start = 0;
        while start < shown.len() {
            let end = start + days[start..].iter().take_while(|day| **day == days[start]).count();
            let none = variant(&QList::<QVariant>::default());
            lines.push(vec![text(&days[start]), none.clone(), none]);
            line_photos.push(Vec::new());
            for row in (start..end).step_by(columns) {
                let photos = shown[row..end.min(row + columns)].to_vec();
                for (offset, &at) in photos.iter().enumerate() {
                    map.insert(state.all[at].path.clone(), (row + offset, lines.len()));
                }
                lines.push(vec![text(""), cells(&state.all, &photos), thumbs(&state.all, &photos)]);
                line_photos.push(photos);
            }
            start = end;
        }
        (state.shown, state.line_photos) = (map, line_photos);
        if let Some(model) = state.model.as_mut() {
            model.replace(rows);
        }
        if let Some(model) = state.lines_model.as_mut() {
            model.replace(lines);
        }
    }

    fn choose_columns(mut self: Pin<&mut Self>, columns: i32) {
        if self.columns != columns {
            self.as_mut().rust_mut().columns = columns;
            self.as_mut().columns_changed();
            self.refresh();
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

    fn take_photo(self: Pin<&mut Self>) {
        let folder = desktop::photo_folder();
        hub::to_phone(|device| Command::TakePhoto { device, folder });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use cxx::UniquePtr;
    use cxx_qt_lib::QString;

    use super::{Photo, PhotosRust};

    fn photos(all: Vec<Photo>) -> PhotosRust {
        PhotosRust {
            loading: false,
            filter: QString::default(),
            error: QString::default(),
            photos: std::ptr::null_mut(),
            model: UniquePtr::null(),
            columns: 6,
            lines: std::ptr::null_mut(),
            lines_model: UniquePtr::null(),
            phone: String::new(),
            asked: false,
            connected: HashMap::new(),
            index: all.iter().enumerate().map(|(at, photo)| (photo.path.clone(), at)).collect(),
            all,
            pending: 0,
            queue: Vec::new(),
            asked_thumbs: HashSet::new(),
            in_flight: 0,
            shown: HashMap::from([("/Photos/a".to_owned(), (0, 1))]),
            line_photos: vec![Vec::new(), vec![0]],
        }
    }

    fn photo(path: &str) -> Photo {
        Photo { path: path.to_owned(), name: "a.jpg".to_owned(), mtime: 0, thumb: String::new() }
    }

    #[test]
    fn a_thumbnail_for_a_photo_on_show_gives_its_rows() {
        let mut state = photos(vec![photo("/Photos/a")]);
        let rows = state.thumbnail("/Photos/a", "file:///a.png").map(|(row, line, _)| (row, line));
        assert_eq!((rows, state.all[0].thumb.as_str()), (Some((0, 1)), "file:///a.png"));
    }

    #[test]
    fn a_late_thumbnail_after_a_reload_is_dropped() {
        let mut state = photos(vec![photo("/Photos/a")]);
        state.forget();
        assert!(state.thumbnail("/Photos/a", "file:///a.png").is_none());
        let mut emptied = photos(Vec::new());
        assert!(emptied.thumbnail("/Photos/a", "file:///a.png").is_none());
    }
}

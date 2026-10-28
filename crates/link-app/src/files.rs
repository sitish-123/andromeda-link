//! Files: the phone's storage, a folder at a time; Photos: its albums as a grid. Both read through the browse switch,
//! and a click saves a file to Downloads.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::{self as gtk, Align, Orientation, gdk, glib, prelude::*};

use crate::link::{Command, Entry};
use crate::seam;
use crate::view::{Ctx, Gate};

/// Thumbnails in the Photos grid, pixels on the longer side.
const THUMB: u16 = 240;

#[derive(Clone)]
pub struct Files(Rc<FilesInner>);

pub struct FilesInner {
    pub gate: Gate,
    ctx: Ctx,
    list: gtk::ListBox,
    place: gtk::Label,
    up: gtk::Button,
    /// The folder on show: "/" for the roots.
    path: RefCell<String>,
    /// The folder asked for last; only its listing shows.
    asked: RefCell<String>,
    loaded: Cell<bool>,
}

impl Files {
    pub fn new(ctx: &Ctx) -> Self {
        let page = gtk::Box::new(Orientation::Vertical, 0);
        page.add_css_class("browse");
        let bar = gtk::Box::new(Orientation::Horizontal, 8);
        bar.add_css_class("place-bar");
        let up = seam::round("go-up-symbolic", "Up one folder");
        bar.append(&up);
        let place = gtk::Label::builder().xalign(0.0).ellipsize(pango::EllipsizeMode::Start).build();
        place.add_css_class("place");
        bar.append(&place);
        page.append(&bar);
        let list = gtk::ListBox::new();
        list.add_css_class("entries");
        list.set_selection_mode(gtk::SelectionMode::None);
        let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();
        page.append(&scroll);
        let gate = Gate::new(
            ctx,
            "folder-symbolic",
            "browse",
            "Phone storage is off",
            "Turn it on to open the phone's folders from this desk.",
            &page,
        );
        let files = Self(Rc::new(FilesInner {
            gate,
            ctx: ctx.clone(),
            list,
            place,
            up: up.clone(),
            path: RefCell::new("/".to_owned()),
            asked: RefCell::default(),
            loaded: Cell::new(false),
        }));
        up.connect_clicked({
            let files = files.clone();
            move |_| {
                let path = files.0.path.borrow().clone();
                let parent =
                    path.rsplit_once('/').map_or("/", |(parent, _)| if parent.is_empty() { "/" } else { parent });
                files.open(parent.to_owned());
            }
        });
        files
    }

    pub fn gate(&self) -> &Gate {
        &self.0.gate
    }

    /// Lists the folder on show the first time the page shows for a phone; `open` false goes back to the roots.
    pub fn shown(&self, open: bool) {
        if !open {
            self.0.loaded.set(false);
            self.0.path.replace("/".to_owned());
        } else if !self.0.loaded.get() {
            self.0.loaded.set(true);
            let path = self.0.path.borrow().clone();
            self.open(path);
        }
    }

    fn open(&self, path: String) {
        self.0.asked.replace(path.clone());
        self.0.ctx.send(|device| Command::List { device, path });
    }

    /// Fills the list when `path` is the folder asked for last.
    pub fn listed(&self, path: &str, entries: &[Entry]) {
        if *self.0.asked.borrow() != path {
            return;
        }
        let this = &self.0;
        this.path.replace(path.to_owned());
        this.up.set_sensitive(path != "/");
        this.place.set_text(&if path == "/" {
            "Phone".to_owned()
        } else {
            format!("Phone{}", path.replace('/', " / "))
        });
        while let Some(row) = this.list.first_child() {
            this.list.remove(&row);
        }
        if entries.is_empty() {
            let empty = gtk::Label::new(Some("This folder is empty."));
            empty.add_css_class("faint");
            empty.set_margin_top(24);
            this.list.append(&empty);
        }
        for entry in entries {
            let child = if path == "/" { format!("/{}", entry.name) } else { format!("{path}/{}", entry.name) };
            this.list.append(&self.row(entry, child));
        }
    }

    fn row(&self, entry: &Entry, path: String) -> gtk::Box {
        let row = gtk::Box::new(Orientation::Horizontal, 14);
        row.add_css_class("entry");
        let icon = if entry.folder { "folder-symbolic" } else { icon_of(&entry.name) };
        let image = gtk::Image::from_icon_name(icon);
        image.add_css_class("app");
        row.append(&image);
        let name = gtk::Label::builder().label(&entry.name).xalign(0.0).hexpand(true).build();
        name.set_ellipsize(pango::EllipsizeMode::Middle);
        row.append(&name);
        let when = gtk::Label::new(Some(&date(entry.mtime)));
        when.add_css_class("slate");
        row.append(&when);
        let size = gtk::Label::new(Some(&if entry.folder { String::new() } else { seam::size(entry.size) }));
        size.add_css_class("slate");
        size.set_width_chars(9);
        size.set_xalign(1.0);
        row.append(&size);
        let (label, solid) = if entry.folder { ("Open", false) } else { ("Save to Downloads", false) };
        let action = seam::button(label, solid);
        action.set_valign(Align::Center);
        let files = self.clone();
        let folder = entry.folder;
        action.connect_clicked(move |_| {
            if folder {
                files.open(path.clone());
            } else {
                let path = path.clone();
                files.0.ctx.send(|device| Command::Save { device, path });
            }
        });
        row.append(&action);
        row
    }
}

#[derive(Clone)]
pub struct Photos(Rc<PhotosInner>);

pub struct PhotosInner {
    pub gate: Gate,
    ctx: Ctx,
    albums: gtk::Box,
    grid: gtk::FlowBox,
    empty: gtk::Label,
    /// The album on show; empty while the album list loads.
    album: RefCell<String>,
    /// The folder asked for last.
    asked: RefCell<String>,
    /// Photo path to its picture, filled as thumbnails arrive.
    pictures: RefCell<std::collections::HashMap<String, gtk::Picture>>,
    loaded: Cell<bool>,
}

const PHOTOS: &str = "/Photos";

impl Photos {
    pub fn new(ctx: &Ctx) -> Self {
        let page = gtk::Box::new(Orientation::Vertical, 0);
        page.add_css_class("browse");
        let albums = gtk::Box::new(Orientation::Horizontal, 6);
        albums.add_css_class("place-bar");
        page.append(&albums);
        let grid = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(3)
            .max_children_per_line(12)
            .row_spacing(8)
            .column_spacing(8)
            .valign(Align::Start)
            .build();
        grid.add_css_class("photos");
        let empty = gtk::Label::new(Some("No photos in this album."));
        empty.add_css_class("faint");
        empty.set_visible(false);
        page.append(&empty);
        page.append(&gtk::ScrolledWindow::builder().child(&grid).vexpand(true).build());
        let gate = Gate::new(
            ctx,
            "image-x-generic-symbolic",
            "browse",
            "Phone storage is off",
            "Turn it on to see the phone's photos from this desk.",
            &page,
        );
        Self(Rc::new(PhotosInner {
            gate,
            ctx: ctx.clone(),
            albums,
            grid,
            empty,
            album: RefCell::default(),
            asked: RefCell::default(),
            pictures: RefCell::default(),
            loaded: Cell::new(false),
        }))
    }

    pub fn gate(&self) -> &Gate {
        &self.0.gate
    }

    pub fn shown(&self, open: bool) {
        if !open {
            self.0.loaded.set(false);
        } else if !self.0.loaded.get() {
            self.0.loaded.set(true);
            self.ask(PHOTOS.to_owned());
        }
    }

    fn ask(&self, path: String) {
        self.0.asked.replace(path.clone());
        self.0.ctx.send(|device| Command::List { device, path });
    }

    pub fn listed(&self, path: &str, entries: &[Entry]) {
        if *self.0.asked.borrow() != path {
            return;
        }
        if path == PHOTOS {
            self.show_albums(entries);
        } else {
            self.show_album(path, entries);
        }
    }

    fn show_albums(&self, entries: &[Entry]) {
        let this = &self.0;
        while let Some(child) = this.albums.first_child() {
            this.albums.remove(&child);
        }
        let mut first: Option<gtk::ToggleButton> = None;
        // Albums by their newest change, newest first, so Camera usually leads.
        let mut albums: Vec<&Entry> = entries.iter().filter(|entry| entry.folder).collect();
        albums.sort_by_key(|entry| std::cmp::Reverse(entry.mtime));
        for entry in &albums {
            let button = gtk::ToggleButton::with_label(&entry.name);
            button.add_css_class("album");
            if let Some(first) = &first {
                button.set_group(Some(first));
            } else {
                first = Some(button.clone());
            }
            let photos = self.clone();
            let name = entry.name.clone();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    photos.0.album.replace(name.clone());
                    photos.ask(format!("{PHOTOS}/{name}"));
                }
            });
            this.albums.append(&button);
        }
        this.empty.set_visible(albums.is_empty());
        this.empty.set_text("The phone has no albums.");
        if let Some(first) = first {
            first.set_active(true);
        }
    }

    fn show_album(&self, path: &str, entries: &[Entry]) {
        let this = &self.0;
        this.grid.remove_all();
        this.pictures.borrow_mut().clear();
        let mut photos: Vec<&Entry> = entries.iter().filter(|entry| !entry.folder).collect();
        photos.sort_by_key(|entry| std::cmp::Reverse(entry.mtime));
        this.empty.set_text("No photos in this album.");
        this.empty.set_visible(photos.is_empty());
        for entry in photos {
            let photo = format!("{path}/{}", entry.name);
            let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).build();
            picture.set_size_request(i32::from(THUMB) * 2 / 3, i32::from(THUMB) * 2 / 3);
            let button = gtk::Button::builder().child(&picture).tooltip_text(&entry.name).build();
            button.add_css_class("photo");
            button.update_property(&[gtk::accessible::Property::Label(&format!("Save {} to Downloads", entry.name))]);
            let ctx = this.ctx.clone();
            let saved = photo.clone();
            button.connect_clicked(move |_| {
                let path = saved.clone();
                ctx.send(|device| Command::Save { device, path });
            });
            this.grid.append(&button);
            this.pictures.borrow_mut().insert(photo.clone(), picture);
            this.ctx.send(|device| Command::Thumbnail { device, path: photo, size: THUMB });
        }
    }

    pub fn thumbnail(&self, path: &str, png: &[u8]) {
        let Some(picture) = self.0.pictures.borrow().get(path).cloned() else { return };
        match gdk::Texture::from_bytes(&glib::Bytes::from(png)) {
            Ok(texture) => picture.set_paintable(Some(&texture)),
            Err(error) => log::debug!("{path}: {error}"),
        }
    }
}

fn icon_of(name: &str) -> &'static str {
    let extension = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" => "image-x-generic-symbolic",
        "mp4" | "mkv" | "webm" | "mov" => "video-x-generic-symbolic",
        "mp3" | "m4a" | "ogg" | "opus" | "flac" | "wav" => "audio-x-generic-symbolic",
        "pdf" | "doc" | "docx" | "odt" | "txt" | "md" => "x-office-document-symbolic",
        "zip" | "tar" | "gz" | "7z" | "apk" => "package-x-generic-symbolic",
        _ => "text-x-generic-symbolic",
    }
}

/// A day as people read it: today's time, else the date; nothing for 0, which roots give.
fn date(mtime: u64) -> String {
    let Ok(seconds @ 1..) = i64::try_from(mtime) else { return String::new() };
    let Ok(when) = glib::DateTime::from_unix_local(seconds) else { return String::new() };
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let format = if when.ymd() == now.ymd() {
        "%H:%M"
    } else if when.year() == now.year() {
        "%-d %b"
    } else {
        "%-d %b %Y"
    };
    when.format(format).map(|text| text.to_string()).unwrap_or_default()
}

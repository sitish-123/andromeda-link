//! The seam: a vertical line down the middle of Home. What came from the phone sits left of it, what this desk sent
//! sits right of it, each row joined to the line by a short connector, with its time on the other side.

use gtk4::{self as gtk, Align, Orientation, gdk, glib, pango, prelude::*};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Phone,
    Desk,
}

#[derive(Clone)]
pub struct Seam {
    root: gtk::Overlay,
    rows: gtk::Box,
}

impl Seam {
    pub fn new() -> Self {
        let rows = gtk::Box::new(Orientation::Vertical, 0);
        rows.set_valign(Align::End);
        let scroller =
            gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&rows).build();
        // New rows arrive at the bottom; keep the newest in view unless the user scrolled up to read.
        let adjustment = scroller.vadjustment();
        let following = std::rc::Rc::new(std::cell::Cell::new(true));
        adjustment.connect_value_changed({
            let following = following.clone();
            move |adj| following.set(adj.value() + adj.page_size() >= adj.upper() - 8.0)
        });
        adjustment.connect_upper_notify(move |adj| {
            if following.get() {
                adj.set_value(adj.upper() - adj.page_size());
            }
        });
        // Older rows fade out under the seam's ends as they scroll up.
        let fade = gtk::Box::new(Orientation::Vertical, 0);
        fade.add_css_class("fade");
        fade.set_valign(Align::Start);
        fade.set_can_target(false);
        let root = gtk::Overlay::new();
        root.set_child(Some(&scroller));
        root.add_overlay(&fade);
        let seam = Self { root, rows };
        seam.day("Today");
        seam
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.root
    }

    fn day(&self, text: &str) {
        let row = gtk::CenterBox::new();
        let label = gtk::Label::new(Some(text));
        label.add_css_class("day");
        row.set_center_widget(Some(&label));
        row.set_margin_bottom(6);
        self.rows.append(&row);
    }

    /// Adds a row; `kind` is "" for something done, "moving" while it crosses, "need" when it waits for the user.
    pub fn add(&self, side: Side, item: &impl IsA<gtk::Widget>, time: &str, kind: &str) -> gtk::CenterBox {
        let row = gtk::CenterBox::new();
        row.add_css_class("x");
        if !kind.is_empty() {
            row.add_css_class(kind);
        }
        let node = gtk::Box::new(Orientation::Vertical, 0);
        node.add_css_class("node");
        let dot = gtk::Box::new(Orientation::Horizontal, 0);
        dot.add_css_class("dot");
        dot.set_halign(Align::Center);
        dot.set_valign(Align::Center);
        dot.set_vexpand(true);
        node.append(&dot);
        row.set_center_widget(Some(&node));

        let stamp = gtk::Label::new(Some(time));
        stamp.add_css_class("time");
        stamp.set_hexpand(true);
        if side == Side::Phone {
            stamp.set_halign(Align::Start);
            row.set_end_widget(Some(&stamp));
        } else {
            stamp.set_halign(Align::End);
            row.set_start_widget(Some(&stamp));
        }
        set_item(&row, side, item);
        self.rows.append(&row);
        row
    }
}

/// Puts `item` in the row on its side, replacing what was there.
pub fn set_item(row: &gtk::CenterBox, side: Side, item: &impl IsA<gtk::Widget>) {
    let near = gtk::Box::new(Orientation::Horizontal, 0);
    near.set_hexpand(true);
    let stub = gtk::Box::new(Orientation::Horizontal, 0);
    stub.add_css_class("stub");
    stub.set_valign(Align::Center);
    let item = item.as_ref();
    item.add_css_class("item");
    item.set_valign(Align::Center);
    if side == Side::Phone {
        near.set_halign(Align::End);
        near.append(item);
        near.append(&stub);
        row.set_start_widget(Some(&near));
    } else {
        near.set_halign(Align::Start);
        near.append(&stub);
        near.append(item);
        row.set_end_widget(Some(&near));
    }
}

/// Sets the row's state: "" done, "moving", or "need".
pub fn set_kind(row: &gtk::CenterBox, kind: &str) {
    for class in ["moving", "need"] {
        row.remove_css_class(class);
    }
    if !kind.is_empty() {
        row.add_css_class(kind);
    }
}

/// The time of day as the seam shows it, for example 10:32.
pub fn now() -> String {
    glib::DateTime::now_local().and_then(|time| time.format("%-H:%M")).map(String::from).unwrap_or_default()
}

/// A bare message row: the app's icon when there is one, who or what it is, and the text.
pub fn message(icon: Option<&gtk::Widget>, from: &str, text: &str) -> (gtk::Box, gtk::Box) {
    let item = gtk::Box::new(Orientation::Horizontal, 12);
    if let Some(icon) = icon {
        item.append(icon);
    }
    let body = gtk::Box::new(Orientation::Vertical, 0);
    body.set_hexpand(true);
    let header = gtk::Label::new(None);
    header.set_markup(from);
    header.add_css_class("from");
    header.set_xalign(0.0);
    header.set_wrap(true);
    body.append(&header);
    if !text.is_empty() {
        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.set_wrap_mode(pango::WrapMode::WordChar);
        label.set_max_width_chars(52);
        body.append(&label);
    }
    item.append(&body);
    (item, body)
}

pub fn app_icon(icon: Option<&gdk::Texture>, letter: &str) -> gtk::Widget {
    if let Some(texture) = icon {
        let picture = gtk::Picture::for_paintable(texture);
        picture.set_size_request(28, 28);
        picture.set_valign(Align::Start);
        picture.set_overflow(gtk::Overflow::Hidden);
        picture.add_css_class("app");
        return picture.upcast();
    }
    let label = gtk::Label::new(Some(letter));
    label.add_css_class("app");
    label.set_valign(Align::Start);
    label.upcast()
}

/// An object crossing: a file or a clip, with its size or state below the name.
pub struct Ticket {
    pub widget: gtk::Box,
    pub sub: gtk::Label,
    pub progress: gtk::ProgressBar,
    pub end: gtk::Box,
}

pub fn ticket(icon: &str, name: &str, sub: &str) -> Ticket {
    let widget = gtk::Box::new(Orientation::Horizontal, 12);
    widget.add_css_class("tk");
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class("slate");
    widget.append(&image);
    let body = gtk::Box::new(Orientation::Vertical, 0);
    body.set_hexpand(true);
    let title = gtk::Label::new(Some(name));
    title.add_css_class("name-line");
    title.set_xalign(0.0);
    title.set_ellipsize(pango::EllipsizeMode::Middle);
    title.set_max_width_chars(36);
    let sub_label = gtk::Label::new(Some(sub));
    sub_label.add_css_class("faint");
    sub_label.set_xalign(0.0);
    let progress = gtk::ProgressBar::new();
    progress.set_margin_top(6);
    progress.set_visible(false);
    body.append(&title);
    body.append(&sub_label);
    body.append(&progress);
    widget.append(&body);
    let end = gtk::Box::new(Orientation::Horizontal, 0);
    widget.append(&end);
    Ticket { widget, sub: sub_label, progress, end }
}

pub fn button(label: &str, solid: bool) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("sm");
    if solid {
        button.add_css_class("solid");
    }
    button
}

pub fn round(icon: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("round");
    button.set_tooltip_text(Some(name));
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}

/// Sizes as people read them: 512 bytes, 3.1 MB.
pub fn size(bytes: u64) -> String {
    #[expect(clippy::cast_precision_loss, reason = "a size shown with one decimal")]
    let value = bytes as f64;
    match bytes {
        0..1_000 => format!("{bytes} bytes"),
        1_000..1_000_000 => format!("{:.0} kB", value / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", value / 1e6),
        _ => format!("{:.1} GB", value / 1e9),
    }
}

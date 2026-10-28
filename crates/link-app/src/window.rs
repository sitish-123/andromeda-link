//! The window: the view rail, the phone's header, and Home's seam, kept current from the service's events.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::{self as gtk, Align, Orientation, gdk, gio, glib, prelude::*};
use libadwaita::{self as adw, prelude::*};
use tokio::sync::mpsc::UnboundedSender;

use crate::files::{Files, Photos};
use crate::link::{self, Command, Device, Event, Status};
use crate::messages::Messages;
use crate::screen::{Calls, Screen};
use crate::seam::{self, Seam, Side, Ticket};
use crate::settings::Settings;
use crate::view::Ctx;

struct Header {
    name: gtk::Label,
    dot: gtk::Box,
    live: gtk::Label,
    battery: gtk::Box,
    fill: gtk::Box,
    percent: gtk::Label,
    charging: gtk::Label,
    bars: Vec<gtk::Box>,
    bars_box: gtk::Box,
    now: Now,
    ring: gtk::Button,
}

struct Transfer {
    ticket: Ticket,
    row: gtk::CenterBox,
    incoming: bool,
    total: u64,
}

struct Ui {
    window: adw::ApplicationWindow,
    commands: UnboundedSender<Command>,
    header: Header,
    home: gtk::Stack,
    seam: Seam,
    compose: gtk::Box,
    entry: gtk::Entry,
    pair: gtk::Button,
    toasts: adw::ToastOverlay,
    devices: Vec<Device>,
    status: HashMap<String, Status>,
    service: bool,
    transfers: HashMap<String, Transfer>,
    notes: HashMap<String, gtk::CenterBox>,
    /// Per device: the call's row, the caller as markup, and whether it was answered.
    calls: HashMap<String, (gtk::CenterBox, String, bool)>,
    pairing: Option<adw::AlertDialog>,
    /// Per device: its player and when that state arrived, to move the seek line between updates.
    players: HashMap<String, (link::Player, std::time::Instant)>,
    ctx: Ctx,
    views: Views,
    texts: HashMap<String, Vec<link::Text>>,
    contacts: HashMap<String, Vec<(String, Vec<String>)>>,
    grants: HashMap<String, Vec<String>>,
    auto_accept: Vec<String>,
    localsend: bool,
    notify_locked: bool,
}

type Shared = Rc<RefCell<Ui>>;

/// The views past Home, in rail order.
struct Views {
    messages: Messages,
    calls: Calls,
    photos: Photos,
    files: Files,
    screen: Screen,
    settings: Settings,
}

impl Views {
    fn new(ctx: &Ctx, stack: &gtk::Stack) -> Self {
        let views = Self {
            messages: Messages::new(ctx),
            calls: Calls::new(ctx),
            photos: Photos::new(ctx),
            files: Files::new(ctx),
            screen: Screen::new(ctx),
            settings: Settings::new(ctx),
        };
        stack.add_named(&views.messages.gate().stack, Some("messages"));
        stack.add_named(&views.calls.gate.stack, Some("calls"));
        stack.add_named(&views.photos.gate().stack, Some("photos"));
        stack.add_named(&views.files.gate().stack, Some("files"));
        stack.add_named(&views.screen.gate.stack, Some("screen"));
        stack.add_named(&views.settings.page, Some("settings"));
        views
    }
}

pub fn present(app: &adw::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    let (commands, mut events) = link::start();
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Link")
        .default_width(1280)
        .default_height(800)
        .build();
    window.add_css_class("link");
    app.set_accels_for_action("win.send-clipboard", &["<Control><Shift>v"]);
    let style = adw::StyleManager::default();
    let follow = {
        let window = window.clone();
        move |style: &adw::StyleManager| {
            if style.is_dark() {
                window.add_css_class("dark");
            } else {
                window.remove_css_class("dark");
            }
        }
    };
    follow(&style);
    style.connect_dark_notify(follow);

    let root = gtk::Box::new(Orientation::Horizontal, 0);
    let sheet = gtk::Box::new(Orientation::Vertical, 0);
    sheet.add_css_class("sheet");
    sheet.set_hexpand(true);
    sheet.set_overflow(gtk::Overflow::Hidden);
    let header = header();
    let views = gtk::Stack::new();
    views.set_vexpand(true);
    views.set_transition_type(gtk::StackTransitionType::Crossfade);

    let seam = Seam::new();
    let home = home_page(&seam);
    views.add_named(&home.stack, Some("home"));
    let ctx = Ctx { commands: commands.clone(), device: Rc::default() };
    let pages = Views::new(&ctx, &views);
    root.append(&rail(&window, &views));
    sheet.append(&header.row());
    sheet.append(&views);
    root.append(&sheet);
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&root));
    window.set_content(Some(&toasts));

    let ui: Shared = Rc::new(RefCell::new(Ui {
        window: window.clone(),
        commands,
        header,
        home: home.stack.clone(),
        seam,
        compose: home.compose.clone(),
        entry: home.entry.clone(),
        pair: home.pair.clone(),
        toasts,
        devices: Vec::new(),
        status: HashMap::new(),
        service: false,
        transfers: HashMap::new(),
        notes: HashMap::new(),
        calls: HashMap::new(),
        pairing: None,
        players: HashMap::new(),
        ctx,
        views: pages,
        texts: HashMap::new(),
        contacts: HashMap::new(),
        grants: HashMap::new(),
        auto_accept: Vec::new(),
        localsend: false,
        notify_locked: false,
    }));
    glib::timeout_add_seconds_local(1, {
        let ui = Rc::downgrade(&ui);
        move || match ui.upgrade() {
            Some(ui) => {
                show_player(&ui.borrow());
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        }
    });
    wire(&ui, &home);
    refresh(&ui.borrow());
    glib::spawn_future_local({
        let ui = ui.clone();
        async move {
            while let Some(event) = events.recv().await {
                handle(&ui, event);
            }
        }
    });
    window.present();
    home.entry.grab_focus();
}

impl Header {
    fn row(&self) -> gtk::Box {
        let row = gtk::Box::new(Orientation::Horizontal, 24);
        row.add_css_class("head");
        self.name.set_valign(Align::End);
        row.append(&self.name);
        let state = gtk::Box::new(Orientation::Horizontal, 14);
        state.set_valign(Align::End);
        state.set_margin_bottom(6);
        let live = gtk::Box::new(Orientation::Horizontal, 7);
        self.dot.set_valign(Align::Center);
        live.append(&self.dot);
        live.append(&self.live);
        state.append(&live);
        state.append(&self.battery);
        state.append(&self.bars_box);
        row.append(&state);
        let space = gtk::Box::new(Orientation::Horizontal, 0);
        space.set_hexpand(true);
        row.append(&space);
        self.now.capsule.set_valign(Align::Center);
        row.append(&self.now.capsule);
        self.ring.set_valign(Align::Center);
        row.append(&self.ring);
        row
    }
}

/// The phone's player in the header: art, title and artist, a seek line, and play or pause.
struct Now {
    capsule: gtk::Box,
    art: gtk::Image,
    label: gtk::Label,
    seek: gtk::ProgressBar,
    toggle: gtk::Button,
}

fn now() -> Now {
    let capsule = gtk::Box::new(Orientation::Horizontal, 10);
    capsule.add_css_class("now");
    capsule.set_visible(false);
    let art = gtk::Image::new();
    art.add_css_class("art");
    art.set_pixel_size(28);
    art.set_overflow(gtk::Overflow::Hidden);
    art.set_valign(Align::Center);
    let label = gtk::Label::new(None);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.set_max_width_chars(28);
    let seek = gtk::ProgressBar::new();
    seek.set_valign(Align::Center);
    seek.set_size_request(64, -1);
    let toggle = seam::round("media-playback-pause-symbolic", "Pause");
    capsule.append(&art);
    capsule.append(&label);
    capsule.append(&seek);
    capsule.append(&toggle);
    Now { capsule, art, label, seek, toggle }
}

fn header() -> Header {
    let name = gtk::Label::new(Some("Link"));
    name.add_css_class("name");
    let dot = gtk::Box::new(Orientation::Horizontal, 0);
    dot.add_css_class("live-dot");
    let live = gtk::Label::new(None);
    live.add_css_class("live");
    let battery = gtk::Box::new(Orientation::Horizontal, 7);
    let cap = gtk::Box::new(Orientation::Horizontal, 0);
    cap.add_css_class("cap");
    cap.set_valign(Align::Center);
    let fill = gtk::Box::new(Orientation::Horizontal, 0);
    fill.add_css_class("fill");
    fill.set_halign(Align::Start);
    cap.append(&fill);
    let percent = gtk::Label::new(None);
    percent.set_markup("");
    let charging = gtk::Label::new(Some("charging"));
    charging.add_css_class("slate");
    battery.append(&cap);
    battery.append(&percent);
    battery.append(&charging);
    let bars_box = gtk::Box::new(Orientation::Horizontal, 2);
    bars_box.set_valign(Align::Center);
    let bars = [4, 7, 10, 12]
        .into_iter()
        .map(|height| {
            let bar = gtk::Box::new(Orientation::Horizontal, 0);
            bar.add_css_class("bar");
            bar.set_size_request(3, height);
            bar.set_valign(Align::End);
            bars_box.append(&bar);
            bar
        })
        .collect();
    let ring = gtk::Button::new();
    let content = gtk::Box::new(Orientation::Horizontal, 8);
    content.append(&gtk::Image::from_icon_name("preferences-system-notifications-symbolic"));
    content.append(&gtk::Label::new(Some("Ring")));
    ring.set_child(Some(&content));
    ring.add_css_class("pill");
    Header { name, dot, live, battery, fill, percent, charging, bars, bars_box, now: now(), ring }
}

/// The view rail; `win.view` names the view on show, so tests and other apps can open one.
fn rail(window: &adw::ApplicationWindow, views: &gtk::Stack) -> gtk::Box {
    let rail = gtk::Box::new(Orientation::Vertical, 2);
    rail.add_css_class("rail");
    let view = gio::ActionEntry::builder("view")
        .parameter_type(Some(glib::VariantTy::STRING))
        .state("home".to_variant())
        .change_state({
            let views = views.clone();
            move |_: &adw::ApplicationWindow, action, value| {
                if let Some(page) = value.and_then(glib::Variant::str)
                    && views.child_by_name(page).is_some()
                {
                    views.set_visible_child_name(page);
                    action.set_state(&page.to_variant());
                }
            }
        })
        .build();
    window.add_action_entries([view]);
    let entries = [
        ("home", "Home", "user-home-symbolic"),
        ("messages", "Messages", "chat-message-new-symbolic"),
        ("calls", "Calls", "call-start-symbolic"),
        ("photos", "Photos", "image-x-generic-symbolic"),
        ("files", "Files", "folder-symbolic"),
        ("screen", "Screen", "phone-symbolic"),
        ("", "", ""),
        ("settings", "Settings", "emblem-system-symbolic"),
    ];
    for (page, title, icon) in entries {
        if page.is_empty() {
            let space = gtk::Box::new(Orientation::Vertical, 0);
            space.set_vexpand(true);
            rail.append(&space);
            continue;
        }
        let content = gtk::Box::new(Orientation::Vertical, 4);
        content.append(&gtk::Image::from_icon_name(icon));
        content.append(&gtk::Label::new(Some(title)));
        let button = gtk::ToggleButton::builder().child(&content).build();
        button.set_action_name(Some("win.view"));
        button.set_action_target_value(Some(&page.to_variant()));
        rail.append(&button);
    }
    rail
}

struct Home {
    stack: gtk::Stack,
    compose: gtk::Box,
    entry: gtk::Entry,
    clip: gtk::Button,
    attach: gtk::Button,
    send: gtk::Button,
    pair: gtk::Button,
}

/// Home: the seam with its two ends named, and the composer; or why there is nothing to show.
fn home_page(seam: &Seam) -> Home {
    let stack = gtk::Stack::new();
    let page = gtk::Box::new(Orientation::Vertical, 0);
    let ends = gtk::CenterBox::new();
    ends.add_css_class("ends");
    let phone = gtk::Label::new(Some("From the phone"));
    phone.add_css_class("faint");
    phone.set_halign(Align::End);
    phone.set_hexpand(true);
    let desk = gtk::Label::new(Some("From this desk"));
    desk.add_css_class("faint");
    desk.set_halign(Align::Start);
    desk.set_hexpand(true);
    let ring = gtk::Box::new(Orientation::Horizontal, 0);
    ring.add_css_class("ring");
    ring.set_valign(Align::Center);
    ends.set_start_widget(Some(&phone));
    ends.set_center_widget(Some(&ring));
    ends.set_end_widget(Some(&desk));
    page.append(&ends);
    page.append(seam.widget());

    let compose = gtk::Box::new(Orientation::Horizontal, 6);
    compose.add_css_class("compose");
    let entry = gtk::Entry::builder().placeholder_text("Send text to the phone, or drop files").hexpand(true).build();
    compose.append(&entry);
    let clip = seam::round("edit-paste-symbolic", "Send clipboard");
    compose.append(&clip);
    let attach = seam::round("mail-attachment-symbolic", "Send files");
    compose.append(&attach);
    let send = gtk::Button::from_icon_name("go-next-symbolic");
    send.add_css_class("send");
    send.update_property(&[gtk::accessible::Property::Label("Send")]);
    compose.append(&send);
    page.append(&compose);
    stack.add_named(&page, Some("seam"));

    let down = adw::StatusPage::builder()
        .icon_name("phone-symbolic")
        .title("Link is not running")
        .description("Start andromeda-linkd; this window connects as soon as it runs.")
        .build();
    stack.add_named(&down, Some("down"));
    let pair = seam::button("Pair a phone", true);
    pair.set_halign(Align::Center);
    let empty = adw::StatusPage::builder()
        .icon_name("phone-symbolic")
        .title("No phone yet")
        .description("Install Link on the phone, then pair it with this desk.")
        .child(&pair)
        .build();
    stack.add_named(&empty, Some("empty"));
    Home { stack, compose, entry, clip, attach, send, pair }
}

/// Connects the controls that send commands.
fn wire(ui: &Shared, home: &Home) {
    let this = ui.borrow();
    this.header.now.toggle.connect_clicked({
        let ui = ui.clone();
        move |_| send(&ui, Command::PlayPause)
    });
    let clip = gio::ActionEntry::builder("send-clipboard")
        .activate({
            let ui = ui.clone();
            move |_: &adw::ApplicationWindow, _, _| send_clipboard(&ui)
        })
        .build();
    this.window.add_action_entries([clip]);
    home.clip.set_action_name(Some("win.send-clipboard"));
    home.clip.set_tooltip_text(Some("Send clipboard (Ctrl+Shift+V)"));
    this.header.ring.connect_clicked({
        let ui = ui.clone();
        move |_| send(&ui, Command::Ring)
    });
    let (entry, attach, go) = (home.entry.clone(), home.attach.clone(), home.send.clone());
    let share = {
        let ui = ui.clone();
        let entry = entry.clone();
        move || {
            let text = entry.text().trim().to_owned();
            if !text.is_empty() {
                send(&ui, |device| Command::Share { device, text });
                entry.set_text("");
            }
        }
    };
    entry.connect_activate({
        let share = share.clone();
        move |_| share()
    });
    go.connect_clicked(move |_| share());
    attach.connect_clicked({
        let ui = ui.clone();
        move |_| choose_files(&ui)
    });

    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop.connect_enter({
        let compose = this.compose.clone();
        move |_, _, _| {
            compose.add_css_class("drop");
            gdk::DragAction::COPY
        }
    });
    drop.connect_leave({
        let compose = this.compose.clone();
        move |_| compose.remove_css_class("drop")
    });
    drop.connect_drop({
        let ui = ui.clone();
        move |_, value, _, _| {
            ui.borrow().compose.remove_css_class("drop");
            let Ok(files) = value.get::<gdk::FileList>() else { return false };
            let paths: Vec<_> = files.files().iter().filter_map(gio::prelude::FileExt::path).collect();
            send(&ui, |device| Command::SendFiles { device, paths });
            true
        }
    });
    this.window.add_controller(drop);

    let commands = this.commands.clone();
    let pair = gio::ActionEntry::builder("pair")
        .activate(move |_: &adw::ApplicationWindow, _, _| {
            let _ = commands.send(Command::Pair);
        })
        .build();
    this.window.add_action_entries([pair]);
    home.pair.set_action_name(Some("win.pair"));
}

fn choose_files(ui: &Shared) {
    let dialog = gtk::FileDialog::builder().title("Send files to the phone").build();
    let window = ui.borrow().window.clone();
    let ui = ui.clone();
    dialog.open_multiple(Some(&window), gio::Cancellable::NONE, move |result| {
        let Ok(files) = result else { return };
        let paths: Vec<_> = (0..files.n_items())
            .filter_map(|index| files.item(index).and_downcast::<gio::File>())
            .filter_map(|file| file.path())
            .collect();
        if !paths.is_empty() {
            send(&ui, |device| Command::SendFiles { device, paths });
        }
    });
}

/// Sends a command for the phone on show, when there is one.
fn send(ui: &Shared, command: impl FnOnce(String) -> Command) {
    let this = ui.borrow();
    if let Some(device) = current(&this) {
        let _ = this.commands.send(command(device.id.clone()));
    }
}

/// The phone on show: the first connected one, else the first paired one.
fn current(ui: &Ui) -> Option<&Device> {
    ui.devices.iter().find(|device| device.connected).or_else(|| ui.devices.first())
}

fn name_of(ui: &Ui, id: &str) -> String {
    ui.devices
        .iter()
        .find(|device| device.id == id)
        .map_or_else(|| "the phone".to_owned(), |device| device.name.clone())
}

fn refresh(ui: &Ui) {
    let header = &ui.header;
    let device = current(ui);
    let page = match (ui.service, device) {
        (false, _) => "down",
        (true, None) => "empty",
        (true, Some(_)) => "seam",
    };
    if ui.home.visible_child_name().as_deref() != Some(page) {
        ui.home.set_visible_child_name(page);
        // The page's own action takes the focus: Pair a phone when there is none.
        if page == "empty" {
            ui.pair.grab_focus();
        }
    }
    header.name.set_text(device.map_or("Link", |device| device.name.as_str()));
    let connected = ui.service && device.is_some_and(|device| device.connected);
    let status = device.and_then(|device| ui.status.get(&device.id)).filter(|_| connected);
    for widget in [&header.dot.clone().upcast::<gtk::Widget>(), &header.live.clone().upcast()] {
        if connected {
            widget.remove_css_class("off");
        } else {
            widget.add_css_class("off");
        }
    }
    header.live.set_text(&match (connected, status.map(|status| status.network.as_str())) {
        (false, _) => "Not connected".to_owned(),
        (true, Some("wifi")) => "On Wi-Fi".to_owned(),
        (true, Some("cellular")) => "On mobile data".to_owned(),
        (true, Some("ethernet")) => "On Ethernet".to_owned(),
        (true, _) => "Connected".to_owned(),
    });
    // Without a service or a phone, Home's page says why; the header names no state.
    let known = ui.service && device.is_some();
    header.dot.set_visible(known);
    header.live.set_visible(known);
    header.battery.set_visible(status.is_some());
    header.bars_box.set_visible(status.is_some_and(|status| status.signal >= 0));
    if let Some(status) = status {
        header.fill.set_size_request(i32::try_from(status.battery.min(100) * 24 / 100).unwrap_or(0), 7);
        header.percent.set_markup(&format!("<b>{}%</b>", status.battery));
        header.charging.set_visible(status.charging);
        for (index, bar) in header.bars.iter().enumerate() {
            if i32::try_from(index).unwrap_or(4) < status.signal {
                bar.remove_css_class("off");
            } else {
                bar.add_css_class("off");
            }
        }
    }
    header.ring.set_sensitive(connected);
    show_player(ui);
    // The composer takes the focus when the phone connects, so typing goes to the phone.
    if connected && !ui.compose.is_sensitive() {
        ui.compose.set_sensitive(true);
        ui.entry.grab_focus();
    }
    ui.compose.set_sensitive(connected);

    let changed = ui.ctx.device.borrow().as_ref().map(|old| &old.id) != device.map(|device| &device.id);
    ui.ctx.device.replace(device.cloned());
    if changed {
        show_texts(ui);
    }
    ui.views.settings.show(ui.service, device, &ui.grants, &ui.auto_accept, ui.localsend, ui.notify_locked);
    ui.views.messages.gate().show(ui.service, device, &ui.grants);
    ui.views.calls.gate.show(ui.service, device, &ui.grants);
    ui.views.screen.gate.show(ui.service, device, &ui.grants);
    // Files and Photos list again from the top for a phone that went away or changed.
    let files = ui.views.files.gate().show(ui.service, device, &ui.grants);
    let photos = ui.views.photos.gate().show(ui.service, device, &ui.grants);
    if changed {
        ui.views.files.shown(false);
        ui.views.photos.shown(false);
    }
    ui.views.files.shown(files);
    ui.views.photos.shown(photos);
}

fn show_texts(ui: &Ui) {
    let id = ui.ctx.device.borrow().as_ref().map(|device| device.id.clone()).unwrap_or_default();
    ui.views
        .messages
        .set(ui.texts.get(&id).cloned().unwrap_or_default(), ui.contacts.get(&id).cloned().unwrap_or_default());
}

fn handle(ui: &Shared, event: Event) {
    match event {
        Event::Service(up) => ui.borrow_mut().service = up,
        Event::Devices(devices) => ui.borrow_mut().devices = devices,
        Event::Status(status) => ui.borrow_mut().status = status,
        Event::Notification(note) => notification(ui, &note),
        Event::NotificationRemoved { device, id } => {
            if let Some(row) = ui.borrow().notes.get(&format!("{device}/{id}")) {
                row.add_css_class("gone");
                row.set_sensitive(false);
            }
        }
        Event::Call { device, state, number, name } => call(ui, &device, &state, &number, &name),
        Event::Offered { transfer, device, files } => offered(ui, &transfer, &device, &files),
        Event::Sending { transfer, files } => sending(ui, &transfer, &files),
        Event::Progress { transfer, bytes, total } => {
            if let Some(entry) = ui.borrow_mut().transfers.get_mut(&transfer) {
                entry.total = total;
                entry.ticket.progress.set_visible(true);
                #[expect(clippy::cast_precision_loss, reason = "a progress fraction")]
                entry.ticket.progress.set_fraction(if total == 0 { 0.0 } else { bytes as f64 / total as f64 });
                entry.ticket.sub.set_text(&format!("{} of {}", seam::size(bytes), seam::size(total)));
            }
        }
        Event::Finished { transfer, status } => finished(ui, &transfer, &status),
        Event::Shared(text) => {
            let this = ui.borrow();
            let (item, _) = seam::message(None, "<b>Text</b>", &text);
            this.seam.add(Side::Desk, &item, &seam::now(), "");
        }
        Event::Received { device, text } => {
            let this = ui.borrow();
            let from = format!("<b>Text</b> from {}", glib::markup_escape_text(&name_of(&this, &device)));
            let (item, _) = seam::message(None, &from, &text);
            this.seam.add(Side::Phone, &item, &seam::now(), "");
        }
        Event::Pairing { code, uri } => pairing(ui, &code, &uri),
        Event::Paired(name) => {
            if let Some(dialog) = ui.borrow_mut().pairing.take() {
                dialog.force_close();
            }
            toast(ui, &format!("Paired with {name}"));
        }
        Event::Player { device, player } => {
            let mut this = ui.borrow_mut();
            match player {
                Some(player) => this.players.insert(device, (player, std::time::Instant::now())),
                None => this.players.remove(&device),
            };
        }
        Event::ClipSent(text) => {
            let this = ui.borrow();
            let ticket = seam::ticket("edit-paste-symbolic", text.lines().next().unwrap_or_default(), "Clipboard");
            this.seam.add(Side::Desk, &ticket.widget, &seam::now(), "");
        }
        Event::Grants(grants) => ui.borrow_mut().grants = grants,
        Event::AutoAccept(devices) => ui.borrow_mut().auto_accept = devices,
        Event::LocalSend(visible) => ui.borrow_mut().localsend = visible,
        Event::NotifyLocked(on) => ui.borrow_mut().notify_locked = on,
        // Replies for a phone no longer on show are dropped.
        Event::Listed { device, path, entries } => {
            let this = ui.borrow();
            if current(&this).is_some_and(|shown| shown.id == device) {
                this.views.files.listed(&path, &entries);
                this.views.photos.listed(&path, &entries);
            }
        }
        Event::Thumbnail { device, path, png } => {
            let this = ui.borrow();
            if current(&this).is_some_and(|shown| shown.id == device) {
                this.views.photos.thumbnail(&path, &png);
            }
        }
        Event::Saved(path) => {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            toast(ui, &format!("Saved {name} to Downloads"));
        }
        Event::Texts(texts) => {
            ui.borrow_mut().texts = texts;
            show_texts(&ui.borrow());
        }
        Event::Contacts(contacts) => {
            ui.borrow_mut().contacts = contacts;
            show_texts(&ui.borrow());
        }
        Event::Failed(message) => {
            log::warn!("{message}");
            toast(ui, &message);
        }
    }
    refresh(&ui.borrow());
}

/// Shows the player of the phone on show, moving its seek line by the time since the last update while it plays.
fn show_player(ui: &Ui) {
    let now = &ui.header.now;
    let entry = current(ui).filter(|device| device.connected).and_then(|device| ui.players.get(&device.id));
    let Some((player, since)) = entry.filter(|(player, _)| !player.title.is_empty()) else {
        now.capsule.set_visible(false);
        return;
    };
    now.capsule.set_visible(true);
    let title = glib::markup_escape_text(&player.title);
    now.label.set_markup(&if player.artist.is_empty() {
        format!("<b>{title}</b>")
    } else {
        format!("<b>{title}</b>, {}", glib::markup_escape_text(&player.artist))
    });
    let art = player.art.as_ref().and_then(|path| gdk::Texture::from_filename(path).ok());
    now.art.set_paintable(art.as_ref());
    now.art.set_visible(art.is_some());
    let elapsed = if player.playing { i64::try_from(since.elapsed().as_micros()).unwrap_or(0) } else { 0 };
    #[expect(clippy::cast_precision_loss, reason = "a seek fraction")]
    let fraction = if player.length > 0 { (player.position + elapsed) as f64 / player.length as f64 } else { 0.0 };
    now.seek.set_fraction(fraction.clamp(0.0, 1.0));
    now.seek.set_visible(player.length > 0);
    let (icon, name) = if player.playing {
        ("media-playback-pause-symbolic", "Pause")
    } else {
        ("media-playback-start-symbolic", "Play")
    };
    now.toggle.set_icon_name(icon);
    now.toggle.set_tooltip_text(Some(name));
    now.toggle.update_property(&[gtk::accessible::Property::Label(name)]);
}

/// Reads the desktop's clipboard text and offers it to the phone.
fn send_clipboard(ui: &Shared) {
    let Some(display) = gdk::Display::default() else { return };
    let ui = ui.clone();
    display.clipboard().read_text_async(gio::Cancellable::NONE, move |result| match result {
        Ok(Some(text)) if !text.is_empty() => {
            let _ = ui.borrow().commands.send(Command::OfferClipboard(text.to_string()));
        }
        Ok(_) => {
            log::info!("the clipboard holds no text");
            toast(&ui, "The clipboard holds no text");
        }
        Err(error) => {
            log::warn!("reading the clipboard: {error}");
            toast(&ui, "The clipboard holds no text");
        }
    });
}

/// The pairing URI as a QR code: dark modules on white with the 4-module quiet zone, 6 pixels a module.
fn qr(uri: &str) -> Option<gdk::MemoryTexture> {
    const SCALE: usize = 6;
    const QUIET: usize = 4;
    let code = qrcode::QrCode::new(uri.as_bytes()).ok()?;
    let modules = code.width();
    let colors = code.to_colors();
    let side = (modules + 2 * QUIET) * SCALE;
    let mut pixels = vec![255u8; side * side * 3];
    for (index, color) in colors.iter().enumerate() {
        if *color == qrcode::Color::Light {
            continue;
        }
        let (x, y) = ((index % modules + QUIET) * SCALE, (index / modules + QUIET) * SCALE);
        for row in y..y + SCALE {
            let start = (row * side + x) * 3;
            pixels[start..start + SCALE * 3].fill(0);
        }
    }
    let width = i32::try_from(side).ok()?;
    let bytes = glib::Bytes::from_owned(pixels);
    Some(gdk::MemoryTexture::new(width, width, gdk::MemoryFormat::R8g8b8, &bytes, side * 3))
}

fn toast(ui: &Shared, message: &str) {
    ui.borrow().toasts.add_toast(adw::Toast::new(message));
}

fn notification(ui: &Shared, note: &link::Notification) {
    let key = format!("{}/{}", note.device, note.id);
    let texture =
        (!note.icon.is_empty()).then(|| gdk::Texture::from_bytes(&glib::Bytes::from(&note.icon)).ok()).flatten();
    let letter = note.app.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let from = if note.title.is_empty() {
        format!("<b>{}</b>", glib::markup_escape_text(&note.app))
    } else {
        format!("<b>{}</b> in {}", glib::markup_escape_text(&note.title), glib::markup_escape_text(&note.app))
    };
    let (item, body) = seam::message(Some(&seam::app_icon(texture.as_ref(), &letter)), &from, &note.text);
    if let Some((action, label, _)) = note.actions.iter().find(|(_, _, reply)| *reply) {
        body.append(&reply_field(ui, &note.device, &note.id, action, label));
    }
    let buttons = gtk::Box::new(Orientation::Horizontal, 6);
    buttons.set_margin_top(6);
    for (action, label, _) in note.actions.iter().filter(|(_, _, reply)| !*reply) {
        let button = seam::button(label, false);
        let commands = ui.borrow().commands.clone();
        let (device, id, action) = (note.device.clone(), note.id.clone(), action.clone());
        button.connect_clicked(move |_| {
            let _ = commands.send(Command::Reply {
                device: device.clone(),
                id: id.clone(),
                action: action.clone(),
                text: String::new(),
            });
        });
        buttons.append(&button);
    }
    if buttons.first_child().is_some() {
        body.append(&buttons);
    }
    let mut this = ui.borrow_mut();
    if let Some(old) = this.notes.remove(&key)
        && let Some(parent) = old.parent().and_downcast::<gtk::Box>()
    {
        parent.remove(&old);
    }
    let row = this.seam.add(Side::Phone, &item, &seam::now(), "");
    this.notes.insert(key, row);
}

fn reply_field(ui: &Shared, device: &str, id: &str, action: &str, label: &str) -> gtk::Box {
    let field = gtk::Box::new(Orientation::Horizontal, 8);
    field.add_css_class("reply");
    let entry = gtk::Entry::builder().placeholder_text(label).hexpand(true).build();
    let go = gtk::Button::from_icon_name("go-next-symbolic");
    go.add_css_class("go");
    go.set_valign(Align::Center);
    go.update_property(&[gtk::accessible::Property::Label("Send reply")]);
    field.append(&entry);
    field.append(&go);
    let submit = {
        let ui = ui.clone();
        let entry = entry.clone();
        let (device, id, action) = (device.to_owned(), id.to_owned(), action.to_owned());
        move || {
            let text = entry.text().trim().to_owned();
            if text.is_empty() {
                return;
            }
            let this = ui.borrow();
            let _ = this.commands.send(Command::Reply {
                device: device.clone(),
                id: id.clone(),
                action: action.clone(),
                text: text.clone(),
            });
            entry.set_text("");
            let (item, _) = seam::message(None, "<b>Reply</b>", &text);
            this.seam.add(Side::Desk, &item, &seam::now(), "");
        }
    };
    entry.connect_activate({
        let submit = submit.clone();
        move |_| submit()
    });
    go.connect_clicked(move |_| submit());
    field
}

fn call(ui: &Shared, device: &str, state: &str, number: &str, name: &str) {
    let mut this = ui.borrow_mut();
    let previous = this.calls.remove(device);
    // Later states of a call may leave the caller out; keep the name the ringing state gave.
    let who = match (name, number, &previous) {
        ("", "", Some((_, who, _))) => who.clone(),
        ("", "", None) => "A caller".to_owned(),
        ("", number, _) => glib::markup_escape_text(number).to_string(),
        (name, ..) => glib::markup_escape_text(name).to_string(),
    };
    let answered = previous.as_ref().is_some_and(|(_, _, answered)| *answered) || state == "active";
    let (from, kind, missed) = match state {
        "ringing" => (format!("<b>{who}</b> is calling"), "need", false),
        "active" => (format!("On a call with <b>{who}</b>"), "", false),
        _ if answered => (format!("Call with <b>{who}</b> ended"), "", false),
        _ => (format!("<b>{who}</b> missed call"), "", true),
    };
    let icon = if missed { "call-missed-symbolic" } else { "call-start-symbolic" };
    let item = gtk::Box::new(Orientation::Horizontal, 12);
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class("app");
    if missed || state == "ringing" {
        image.add_css_class("miss");
        item.add_css_class("miss");
    }
    image.set_valign(Align::Start);
    item.append(&image);
    let body = gtk::Box::new(Orientation::Vertical, 0);
    let label = gtk::Label::new(None);
    label.set_markup(&from);
    label.add_css_class("from");
    label.set_xalign(0.0);
    body.append(&label);
    if state == "ringing" {
        let buttons = gtk::Box::new(Orientation::Horizontal, 6);
        buttons.set_margin_top(6);
        for (action, text, solid) in [("mute", "Mute", false), ("decline", "Decline", true)] {
            let button = seam::button(text, solid);
            let commands = this.commands.clone();
            let device = device.to_owned();
            button.connect_clicked(move |_| {
                let _ = commands.send(Command::CallAction { device: device.clone(), action: action.to_owned() });
            });
            buttons.append(&button);
        }
        body.append(&buttons);
    }
    item.append(&body);
    let row = match previous {
        Some((row, ..)) => {
            seam::set_item(&row, Side::Phone, &item);
            seam::set_kind(&row, kind);
            row
        }
        None => this.seam.add(Side::Phone, &item, &seam::now(), kind),
    };
    if state == "idle" {
        if current(&this).is_some_and(|shown| shown.id == device) {
            this.views.calls.ended(&who, missed, &seam::now());
        }
    } else {
        this.calls.insert(device.to_owned(), (row, who, answered));
    }
}

fn offered(ui: &Shared, transfer: &str, device: &str, files: &[(String, u64)]) {
    let mut this = ui.borrow_mut();
    let total = files.iter().map(|(_, size)| size).sum();
    let card = gtk::Box::new(Orientation::Vertical, 8);
    card.add_css_class("tk");
    card.add_css_class("need");
    let count = if files.len() == 1 { "1 file".to_owned() } else { format!("{} files", files.len()) };
    let title = gtk::Label::new(None);
    title.set_markup(&format!("<b>{count} from {}</b>", glib::markup_escape_text(&name_of(&this, device))));
    title.set_xalign(0.0);
    card.append(&title);
    for (name, size) in files.iter().take(4) {
        let line = gtk::CenterBox::new();
        let name_label = gtk::Label::new(Some(name));
        name_label.add_css_class("slate");
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        name_label.set_max_width_chars(30);
        let size_label = gtk::Label::new(Some(&seam::size(*size)));
        size_label.add_css_class("slate");
        line.set_start_widget(Some(&name_label));
        line.set_end_widget(Some(&size_label));
        card.append(&line);
    }
    let buttons = gtk::Box::new(Orientation::Horizontal, 8);
    buttons.set_halign(Align::End);
    let decline = seam::button("Decline", false);
    let accept = seam::button("Save to Downloads", true);
    buttons.append(&decline);
    buttons.append(&accept);
    card.append(&buttons);
    let row = this.seam.add(Side::Phone, &card, &seam::now(), "need");
    let label = if files.len() == 1 { files[0].0.clone() } else { count };
    let ticket = seam::ticket("folder-download-symbolic", &label, &seam::size(total));
    for (button, accept) in [(decline, false), (accept, true)] {
        let ui = ui.clone();
        let transfer = transfer.to_owned();
        button.connect_clicked(move |_| {
            let this = ui.borrow();
            let _ = this.commands.send(if accept {
                Command::Accept(transfer.clone())
            } else {
                Command::Decline(transfer.clone())
            });
            if let Some(entry) = this.transfers.get(&transfer) {
                seam::set_item(&entry.row, Side::Phone, &entry.ticket.widget);
                seam::set_kind(&entry.row, if accept { "moving" } else { "" });
                if !accept {
                    entry.ticket.sub.set_text("Declined");
                }
            }
        });
    }
    this.transfers.insert(transfer.to_owned(), Transfer { ticket, row, incoming: true, total });
}

fn sending(ui: &Shared, transfer: &str, files: &[(String, u64)]) {
    let mut this = ui.borrow_mut();
    let total = files.iter().map(|(_, size)| size).sum();
    let name = if files.len() == 1 { files[0].0.clone() } else { format!("{} files", files.len()) };
    let ticket = seam::ticket("text-x-generic-symbolic", &name, &seam::size(total));
    let cancel = seam::round("window-close-symbolic", "Cancel");
    let commands = this.commands.clone();
    let id = transfer.to_owned();
    cancel.connect_clicked(move |_| {
        let _ = commands.send(Command::Cancel(id.clone()));
    });
    ticket.end.append(&cancel);
    let row = this.seam.add(Side::Desk, &ticket.widget, &seam::now(), "moving");
    this.transfers.insert(transfer.to_owned(), Transfer { ticket, row, incoming: false, total });
}

fn finished(ui: &Shared, transfer: &str, status: &str) {
    let this = ui.borrow();
    let Some(entry) = this.transfers.get(transfer) else { return };
    seam::set_kind(&entry.row, "");
    entry.ticket.progress.set_visible(false);
    while let Some(child) = entry.ticket.end.first_child() {
        entry.ticket.end.remove(&child);
    }
    let size = seam::size(entry.total);
    entry.ticket.sub.set_text(&match status {
        "done" if entry.incoming => format!("{size}, saved to Downloads"),
        "done" => size,
        "declined" if entry.incoming => "Declined".to_owned(),
        "declined" => "The phone declined it".to_owned(),
        "cancelled" => "Cancelled".to_owned(),
        "no-space" => "Not enough space".to_owned(),
        "too-large" => "Too large to send".to_owned(),
        "busy" => "The phone is busy with another transfer".to_owned(),
        _ => "Failed".to_owned(),
    });
}

fn pairing(ui: &Shared, code: &str, uri: &str) {
    let dialog = adw::AlertDialog::new(
        Some("Pair a phone"),
        Some(&format!("In Link on the phone, choose Pair, then scan this code or enter {code}.")),
    );
    if let Some(texture) = qr(uri) {
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_can_shrink(false);
        picture.set_halign(Align::Center);
        picture.update_property(&[gtk::accessible::Property::Label("Pairing QR code")]);
        dialog.set_extra_child(Some(&picture));
    }
    dialog.add_response("cancel", "Cancel");
    let commands = ui.borrow().commands.clone();
    dialog.connect_response(Some("cancel"), move |_, _| {
        let _ = commands.send(Command::CancelPairing);
    });
    let window = ui.borrow().window.clone();
    dialog.present(Some(&window));
    ui.borrow_mut().pairing = Some(dialog);
}

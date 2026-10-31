//! The window: the phone's screen, letterboxed, with the pointer as one finger, the wheel, and keys sent back as
//! `MirrorInput` in the phone's coordinates (1/10000 of the video's width and height).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gstreamer as gst;
use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};

use crate::keys::{self, Key};
use crate::stream::Display;
use crate::{LINK, LINK_PATH};

const UNIT: f64 = 10_000.0;
const MOVE_EVERY: Duration = Duration::from_millis(8);
/// A wheel notch reports about this many surface units on a touchpad's smooth scroll.
const SURFACE_PER_NOTCH: f64 = 15.0;
const RETRIES: u32 = 3;

/// The video's size, which follows the phone's rotation.
type Size = Rc<Cell<(f64, f64)>>;

/// A widget point in phone units, clamped to the screen's edge.
fn to_phone(size: &Size, widget: &impl IsA<gtk4::Widget>, x: f64, y: f64) -> (i32, i32) {
    let (width, height) = size.get();
    let (area_w, area_h) = (f64::from(widget.width()), f64::from(widget.height()));
    let scale = (area_w / width).min(area_h / height);
    let (shown_w, shown_h) = (width * scale, height * scale);
    let fx = ((x - (area_w - shown_w) / 2.0) / shown_w).clamp(0.0, 1.0);
    let fy = ((y - (area_h - shown_h) / 2.0) / shown_h).clamp(0.0, 1.0);
    #[expect(clippy::cast_possible_truncation, reason = "clamped to 0..=10000")]
    ((fx * UNIT).round() as i32, (fy * UNIT).round() as i32)
}

struct Viewer {
    bus: gio::DBusConnection,
    device: String,
    name: String,
    display: Rc<Display>,
    window: glib::WeakRef<gtk4::ApplicationWindow>,
    tries: Cell<u32>,
}

#[expect(clippy::too_many_arguments, reason = "the window's whole setup")]
pub fn window(
    app: &gtk4::Application,
    bus: &gio::DBusConnection,
    device: &str,
    name: &str,
    display: &Rc<Display>,
    socket: UnixStream,
    width: u32,
    height: u32,
) {
    let size: Size = Rc::new(Cell::new((f64::from(width), f64::from(height))));
    let picture = display.picture.clone();
    picture.set_content_fit(gtk4::ContentFit::Contain);
    picture.set_can_focus(true);
    picture.set_focusable(true);
    let send = sender(bus, device);

    let header = gtk4::HeaderBar::new();
    for (icon, tooltip, action) in [
        ("go-previous-symbolic", "Back", "back"),
        ("go-home-symbolic", "Home", "home"),
        ("view-app-grid-symbolic", "Recent apps", "recents"),
    ] {
        let button = gtk4::Button::from_icon_name(icon);
        button.set_tooltip_text(Some(tooltip));
        let send = send.clone();
        button.connect_clicked(move |_| send(action, glib::VariantDict::new(None)));
        header.pack_start(&button);
    }
    let screen_off =
        gtk4::ToggleButton::builder().icon_name("video-display-symbolic").tooltip_text("Phone screen off").build();
    {
        let send = send.clone();
        screen_off.connect_toggled(move |button| {
            send(if button.is_active() { "screen-off" } else { "screen-on" }, glib::VariantDict::new(None));
        });
    }
    header.pack_end(&screen_off);
    for (icon, tooltip, code) in [
        ("system-shutdown-symbolic", "Power", keys::POWER),
        ("audio-volume-high-symbolic", "Volume up", keys::VOLUME_UP),
        ("audio-volume-low-symbolic", "Volume down", keys::VOLUME_DOWN),
    ] {
        let button = gtk4::Button::from_icon_name(icon);
        button.set_tooltip_text(Some(tooltip));
        let send = send.clone();
        button.connect_clicked(move |_| {
            send_key(&send, "key-down", code, 0);
            send_key(&send, "key-up", code, 0);
        });
        header.pack_end(&button);
    }

    add_pointer(&picture, &size, &send);

    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title(name)
        .child(&picture)
        .default_width(i32::try_from(width / 2).unwrap_or(540))
        .default_height(i32::try_from(height / 2).unwrap_or(1200))
        .build();
    window.set_titlebar(Some(&header));
    add_keys(&window, &send);
    follow_size(&window, &picture, &size);
    let viewer = Rc::new(Viewer {
        bus: bus.clone(),
        device: device.to_owned(),
        name: name.to_owned(),
        display: display.clone(),
        window: window.downgrade(),
        tries: Cell::new(0),
    });
    {
        let viewer = viewer.clone();
        window.connect_close_request(move |_| {
            if screen_off.is_active() {
                send_sync(&viewer.bus, &viewer.device, "screen-on");
            }
            viewer.display.stop();
            glib::Propagation::Proceed
        });
    }
    if let Err(error) = display.start(socket, on_end(&viewer)) {
        log::error!("showing the mirror: {error:#}");
        return window.close();
    }
    keyframe(bus, device);
    window.present();
    picture.grab_focus();
}

/// The pointer as one finger: down on press, moves while pressed, up on release; the wheel as wheel turns.
fn add_pointer(picture: &gtk4::Picture, size: &Size, send: &Send) {
    let drag = gtk4::GestureDrag::new();
    let moved = Rc::new(Cell::new(Instant::now()));
    // Where the finger is while it is down on the phone.
    let finger = Rc::new(Cell::new(None::<(f64, f64)>));
    let touch = {
        let (send, picture, size) = (send.clone(), picture.clone(), size.clone());
        move |action: &str, x: f64, y: f64| {
            let (x, y) = to_phone(&size, &picture, x, y);
            let args = glib::VariantDict::new(None);
            args.insert("x", x);
            args.insert("y", y);
            send(action, args);
        }
    };
    {
        let (touch, moved, picture, finger) = (touch.clone(), moved.clone(), picture.clone(), finger.clone());
        drag.connect_drag_begin(move |_, x, y| {
            picture.grab_focus();
            moved.set(Instant::now());
            finger.set(Some((x, y)));
            touch("touch-down", x, y);
        });
    }
    {
        let (touch, finger) = (touch.clone(), finger.clone());
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some((x, y)) = gesture.start_point() else { return };
            finger.set(Some((x + dx, y + dy)));
            if moved.get().elapsed() >= MOVE_EVERY {
                moved.set(Instant::now());
                touch("touch-move", x + dx, y + dy);
            }
        });
    }
    {
        let (touch, finger) = (touch.clone(), finger.clone());
        drag.connect_drag_end(move |gesture, dx, dy| {
            let Some((x, y)) = gesture.start_point() else { return };
            if finger.take().is_some() {
                touch("touch-up", x + dx, y + dy);
            }
        });
    }
    // A lost grab cancels the drag; the phone's finger lifts where it was.
    drag.connect_cancel(move |_, _| {
        if let Some((x, y)) = finger.take() {
            touch("touch-up", x, y);
        }
    });
    picture.add_controller(drag);

    let pointer = Rc::new(Cell::new((0.0, 0.0)));
    let motion = gtk4::EventControllerMotion::new();
    {
        let pointer = pointer.clone();
        motion.connect_motion(move |_, x, y| pointer.set((x, y)));
    }
    picture.add_controller(motion);
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::BOTH_AXES);
    {
        let (send, picture, size) = (send.clone(), picture.clone(), size.clone());
        scroll.connect_scroll(move |controller, dx, dy| {
            let per_notch = if controller.unit() == gdk::ScrollUnit::Wheel { 1.0 } else { SURFACE_PER_NOTCH };
            #[expect(clippy::cast_possible_truncation, reason = "clamped to -10000..=10000")]
            let turn = |d: f64| (d / per_notch * 100.0).round().clamp(-UNIT, UNIT) as i32;
            let (x2, y2) = (turn(dx), turn(dy));
            if (x2, y2) != (0, 0) {
                let (px, py) = pointer.get();
                let (x, y) = to_phone(&size, &picture, px, py);
                let args = glib::VariantDict::new(None);
                args.insert("x", x);
                args.insert("y", y);
                args.insert("x2", x2);
                args.insert("y2", y2);
                send("wheel", args);
            }
            glib::Propagation::Stop
        });
    }
    picture.add_controller(scroll);
}

/// Characters as text, other keys and chords as key events; F11 toggles fullscreen. Keys held when the window loses
/// focus go up on the phone.
fn add_keys(window: &gtk4::ApplicationWindow, send: &Send) {
    let keys = gtk4::EventControllerKey::new();
    // Before the focused widget, so Tab and the arrows reach the phone instead of moving focus.
    keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
    // Hardware key code to the Android key that went down, so the release matches even after modifiers changed.
    let held = Rc::new(RefCell::new(HashMap::<u32, (i32, i32)>::new()));
    {
        let (send, held) = (send.clone(), held.clone());
        keys.connect_key_pressed(move |controller, key, keycode, state| {
            if key == gdk::Key::F11 {
                if let Some(window) = controller.widget().and_downcast::<gtk4::Window>() {
                    window.set_fullscreened(!window.is_fullscreen());
                }
                return glib::Propagation::Stop;
            }
            match keys::map(key, state) {
                Some(Key::Text(text)) => {
                    let args = glib::VariantDict::new(None);
                    args.insert("text", text);
                    send("text", args);
                }
                Some(Key::Android { code, meta }) => {
                    held.borrow_mut().insert(keycode, (code, meta));
                    send_key(&send, "key-down", code, meta);
                }
                None => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
    }
    {
        let (send, held) = (send.clone(), held.clone());
        keys.connect_key_released(move |_, _, keycode, _| {
            if let Some((code, meta)) = held.borrow_mut().remove(&keycode) {
                send_key(&send, "key-up", code, meta);
            }
        });
    }
    window.add_controller(keys);
    let send = send.clone();
    window.connect_is_active_notify(move |window| {
        if !window.is_active() {
            for (_, (code, meta)) in held.borrow_mut().drain() {
                send_key(&send, "key-up", code, meta);
            }
        }
    });
}

/// Keeps the input mapping on the decoded size, and the window's scale when the phone turns.
fn follow_size(window: &gtk4::ApplicationWindow, picture: &gtk4::Picture, size: &Size) {
    let check = {
        let (window, size) = (window.downgrade(), size.clone());
        Rc::new(move |picture: &gtk4::Picture| {
            let Some(paintable) = picture.paintable() else { return };
            let (width, height) = (f64::from(paintable.intrinsic_width()), f64::from(paintable.intrinsic_height()));
            let (old_w, old_h) = size.get();
            if width <= 0.0 || height <= 0.0 || (width, height) == (old_w, old_h) {
                return;
            }
            size.set((width, height));
            let Some(window) = window.upgrade() else { return };
            if (width > height) == (old_w > old_h) || window.is_fullscreen() || window.is_maximized() {
                return;
            }
            let scale = (f64::from(picture.width()) / old_w).min(f64::from(picture.height()) / old_h);
            let chrome = window.height() - picture.height();
            #[expect(clippy::cast_possible_truncation, reason = "a window's size")]
            window.set_default_size((width * scale).round() as i32, (height * scale).round() as i32 + chrome);
        })
    };
    picture.connect_paintable_notify(move |picture| {
        check(picture);
        // The GTK sink's paintable stays and announces new sizes; the appsink's textures come one per frame.
        if let Some(paintable) = picture.paintable().filter(|paintable| !paintable.is::<gdk::Texture>()) {
            let (check, picture) = (check.clone(), picture.downgrade());
            paintable.connect_invalidate_size(move |_| {
                if let Some(picture) = picture.upgrade() {
                    check(&picture);
                }
            });
        }
    });
}

/// End of stream is the phone stopping: the window closes. An error is a drop: the mirror reopens, or the window
/// closes after [`RETRIES`] tries.
fn on_end(viewer: &Rc<Viewer>) -> impl FnMut(&gst::Message) + 'static {
    let viewer = viewer.clone();
    move |message| {
        let viewer = viewer.clone();
        let broke = match message.view() {
            gst::MessageView::Error(error) => {
                log::warn!("mirroring: {}", error.error());
                true
            }
            _ => false,
        };
        // Out of the bus watch, which `retry` removes.
        glib::idle_add_local_once(move || if broke { retry(&viewer) } else { close(&viewer) });
    }
}

fn close(viewer: &Viewer) {
    if let Some(window) = viewer.window.upgrade() {
        window.close();
    }
}

fn retry(viewer: &Rc<Viewer>) {
    viewer.display.stop();
    let tries = viewer.tries.get();
    let Some(window) = viewer.window.upgrade().filter(|_| tries < RETRIES) else { return close(viewer) };
    viewer.tries.set(tries + 1);
    window.set_title(Some("Reconnecting…"));
    let viewer = viewer.clone();
    glib::spawn_future_local(async move {
        glib::timeout_future_seconds(1 << tries).await;
        if viewer.window.upgrade().is_none() {
            return;
        }
        let reopened = reopen(&viewer.bus, &viewer.device).await;
        let started = reopened
            .and_then(|socket| viewer.display.start(socket, on_end(&viewer)).map_err(|error| format!("{error:#}")));
        match started {
            Ok(()) => {
                viewer.tries.set(0);
                if let Some(window) = viewer.window.upgrade() {
                    window.set_title(Some(&viewer.name));
                }
                keyframe(&viewer.bus, &viewer.device);
            }
            Err(error) if error.contains(".NotConnected") => {
                log::info!("reopening the mirror: {error}");
                close(&viewer);
            }
            Err(error) => {
                log::info!("reopening the mirror: {error}");
                retry(&viewer);
            }
        }
    });
}

async fn reopen(bus: &gio::DBusConnection, device: &str) -> Result<UnixStream, String> {
    let reply = bus
        .call_with_unix_fd_list_future(
            Some(LINK),
            LINK_PATH,
            LINK,
            "OpenMirror",
            Some(&(device,).to_variant()),
            glib::VariantTy::new("(huu)").ok(),
            gio::DBusCallFlags::NONE,
            crate::OPEN_TIMEOUT_MS,
            None::<&gio::UnixFDList>,
        )
        .await
        .map_err(|error| error.to_string())?;
    crate::opened(&reply).map(|(socket, _, _)| socket).map_err(|error| format!("{error:#}"))
}

type Send = Rc<dyn Fn(&str, glib::VariantDict)>;

fn send_key(send: &Send, action: &str, code: i32, meta: i32) {
    let args = glib::VariantDict::new(None);
    args.insert("code", code);
    args.insert("meta", meta);
    send(action, args);
}

/// One call at a time: the daemon runs each D-Bus call in a task of its own, so calls in flight together could reach
/// the phone out of order.
fn sender(bus: &gio::DBusConnection, device: &str) -> Send {
    let (bus, device) = (bus.clone(), device.to_owned());
    // The inputs not sent yet, and whether a call is in flight.
    let queue = Rc::new(RefCell::new((VecDeque::new(), false)));
    Rc::new(move |action: &str, args: glib::VariantDict| {
        let params = (device.as_str(), action, args.end()).to_variant();
        let idle = {
            let mut queue = queue.borrow_mut();
            queue.0.push_back(params);
            !std::mem::replace(&mut queue.1, true)
        };
        if idle {
            send_next(bus.clone(), queue.clone());
        }
    })
}

type Queue = Rc<RefCell<(VecDeque<glib::Variant>, bool)>>;

fn send_next(bus: gio::DBusConnection, queue: Queue) {
    let next = queue.borrow_mut().0.pop_front();
    let Some(params) = next else {
        queue.borrow_mut().1 = false;
        return;
    };
    bus.clone().call(
        Some(LINK),
        LINK_PATH,
        LINK,
        "MirrorInput",
        Some(&params),
        None,
        gio::DBusCallFlags::NONE,
        5_000,
        None::<&gio::Cancellable>,
        move |reply| {
            if let Err(error) = reply {
                log::info!("mirror input: {error}");
            }
            send_next(bus, queue);
        },
    );
}

/// Sends `action` and waits, for a window about to close.
fn send_sync(bus: &gio::DBusConnection, device: &str, action: &str) {
    let params = (device, action, glib::VariantDict::new(None).end()).to_variant();
    let reply = bus.call_sync(
        Some(LINK),
        LINK_PATH,
        LINK,
        "MirrorInput",
        Some(&params),
        None,
        gio::DBusCallFlags::NONE,
        2_000,
        None::<&gio::Cancellable>,
    );
    if let Err(error) = reply {
        log::info!("mirror input: {error}");
    }
}

/// Asks for a keyframe, so the picture starts at once even if the phone's first one was lost.
pub fn keyframe(bus: &gio::DBusConnection, device: &str) {
    bus.call(
        Some(LINK),
        LINK_PATH,
        LINK,
        "MirrorKeyframe",
        Some(&(device,).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5_000,
        None::<&gio::Cancellable>,
        |reply| {
            if let Err(error) = reply {
                log::debug!("asking for a keyframe: {error}");
            }
        },
    );
}

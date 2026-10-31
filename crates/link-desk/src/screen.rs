//! `LinkScreen`: the current phone's screen inside the Screen page, decoded here into the page's video sink; touches,
//! the wheel, keys, and the phone's buttons go back as `MirrorInput`. A broken stream reopens up to [`RETRIES`] times.

use std::os::unix::net::UnixStream;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QString};

use crate::client::{Command, Event};
use crate::hub;
use crate::keys::{self, Key};
use crate::mirror::{Frame, Stream};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
        include!("link-desk/src/video.h");
        type QVideoSink;
        #[cxx_name = "showNv12"]
        unsafe fn show_nv12(sink: *mut QVideoSink, width: i32, height: i32, stride: i32, data: &[u8]);
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, state)]
        #[qproperty(QString, error)]
        #[qproperty(i32, video_width)]
        #[qproperty(i32, video_height)]
        #[qproperty(bool, screen_off)]
        /// Rounded-corner radii (top-left, top-right, bottom-right, bottom-left) in 1/10000 of the picture's width;
        /// empty or all 0 when the phone did not say.
        #[qproperty(QList_i32, corners)]
        /// Camera cutouts as x, y, width, height after one another, in 1/10000 of the picture.
        #[qproperty(QList_i32, cutouts)]
        type LinkScreen = super::ScreenRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qinvokable]
        fn start(self: Pin<&mut LinkScreen>);
        #[qinvokable]
        fn stop(self: Pin<&mut LinkScreen>);
        #[qinvokable]
        fn back(self: Pin<&mut LinkScreen>);
        #[qinvokable]
        fn home(self: Pin<&mut LinkScreen>);
        #[qinvokable]
        fn recents(self: Pin<&mut LinkScreen>);
        #[qinvokable]
        fn keyframe(self: Pin<&mut LinkScreen>);
        /// The page's video sink, or null when it goes.
        #[qinvokable]
        unsafe fn set_sink(self: Pin<&mut LinkScreen>, sink: *mut QVideoSink);
        /// touch-down, touch-move, touch-up (x, y), or wheel (x, y and the turn x2, y2 in 1/100 of a notch,
        /// positive right and down); points in 1/10000 of the screen.
        #[qinvokable]
        fn input(self: Pin<&mut LinkScreen>, action: &QString, x: i32, y: i32, x2: i32, y2: i32);
        /// A Qt key press or release; whether it went to the phone.
        #[qinvokable]
        fn key(self: Pin<&mut LinkScreen>, key: i32, modifiers: i32, text: &QString, pressed: bool) -> bool;
        /// Lifts every key still down on the phone, when the page loses the keyboard.
        #[qinvokable]
        fn release_keys(self: Pin<&mut LinkScreen>);
        /// Presses and releases an Android key, such as volume or power.
        #[qinvokable]
        fn press(self: Pin<&mut LinkScreen>, code: i32);
        /// Turns the phone's own panel off or back on; mirroring goes on.
        #[qinvokable]
        fn toggle_screen(self: Pin<&mut LinkScreen>);
        /// Text into the phone's focused field: U+0008 deletes, a newline presses Enter.
        #[qinvokable]
        fn type_text(self: Pin<&mut LinkScreen>, text: &QString);
    }

    impl cxx_qt::Threading for LinkScreen {}
    impl cxx_qt::Initialize for LinkScreen {}
}

pub struct ScreenRust {
    state: QString,
    error: QString,
    video_width: i32,
    video_height: i32,
    phone: String,
    /// The phone asked or being mirrored, and its stream once open.
    mirror: Option<(String, Option<Stream>)>,
    sink: *mut qobject::QVideoSink,
    /// The newest picture the page has not shown yet; the decoder drops older ones.
    pending: Arc<Mutex<Option<Frame>>>,
    /// The user stopped the mirror, so its end is no failure.
    stopped: bool,
    screen_off: bool,
    corners: QList<i32>,
    cutouts: QList<i32>,
    /// Reopens since the stream last broke.
    tries: u32,
    /// Qt keys down on the phone, with the Android code and meta they went down with.
    held: Vec<(i32, i32, i32)>,
    /// When the page last showed a picture of the open stream.
    last_frame: Option<Instant>,
    /// When the open stream opened.
    opened_at: Option<Instant>,
}

const RETRIES: u32 = 3;
/// Pictures for this long after an open mean the stream works again, so its reopens count from 0.
const STEADY: Duration = Duration::from_secs(5);
/// The phone repeats a still screen every 100 ms, so a gap this long means the link is down.
const STALL: Duration = Duration::from_secs(2);

impl Default for ScreenRust {
    fn default() -> Self {
        Self {
            state: QString::from("idle"),
            error: QString::default(),
            video_width: 0,
            video_height: 0,
            phone: hub::phone(),
            mirror: None,
            sink: std::ptr::null_mut(),
            pending: Arc::default(),
            stopped: false,
            screen_off: false,
            corners: QList::default(),
            cutouts: QList::default(),
            tries: 0,
            held: Vec::new(),
            last_frame: None,
            opened_at: None,
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkScreen {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            if let Event::Devices(_) | Event::Phone(_) | Event::MirrorOpened { .. } | Event::ScreenShape { .. } = event
            {
                let event = event.clone();
                let _ = thread.queue(move |screen| screen.apply(event));
            }
        });
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            while thread.queue(qobject::LinkScreen::check_stall).is_ok() {
                std::thread::sleep(STALL / 4);
            }
        });
    }
}

impl qobject::LinkScreen {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        match event {
            Event::Devices(devices) => {
                let phone = self.mirror.as_ref().map(|(phone, _)| phone.clone());
                if phone.is_some_and(|phone| !devices.iter().any(|device| device.id == phone && device.connected)) {
                    self.stop();
                }
            }
            Event::Phone(id) => {
                self.as_mut().rust_mut().phone = id;
                self.as_mut().set_corners(QList::default());
                self.as_mut().set_cutouts(QList::default());
                self.stop();
            }
            Event::MirrorOpened { device, opened } => self.opened(&device, opened),
            Event::ScreenShape { device, corners, cutouts } if device == self.phone => {
                let list = |values: Vec<i32>| values.into_iter().collect::<QList<i32>>();
                self.as_mut().set_corners(list(corners.into_iter().map(i32::from).collect()));
                let flat = cutouts.into_iter().flat_map(|(x, y, width, height)| [x, y, width, height]).map(i32::from);
                self.set_cutouts(list(flat.collect()));
            }
            _ => {}
        }
    }

    fn opened(mut self: Pin<&mut Self>, device: &str, opened: Result<(Arc<std::os::fd::OwnedFd>, u32, u32), String>) {
        let asked = matches!(&self.mirror, Some((phone, None)) if phone == device);
        let stream = opened.and_then(|(fd, width, height)| {
            let socket = UnixStream::from(fd.try_clone().map_err(|error| error.to_string())?);
            if !asked {
                // The user stopped while the phone asked; dropping the socket tells the phone.
                return Ok(None);
            }
            let (pending, thread) = (Arc::clone(&self.pending), self.qt_thread());
            let ended = self.qt_thread();
            let stream = Stream::start(
                socket,
                move |frame| {
                    let mut slot = pending.lock().unwrap_or_else(PoisonError::into_inner);
                    if slot.replace(frame).is_none() {
                        let _ = thread.queue(qobject::LinkScreen::show);
                    }
                },
                move |broke| drop(ended.queue(move |screen| screen.ended("", broke))),
            )?;
            Ok(Some((stream, width, height)))
        });
        match stream {
            Ok(Some((stream, width, height))) => {
                self.as_mut().set_video_width(i32::try_from(width).unwrap_or(0));
                self.as_mut().set_video_height(i32::try_from(height).unwrap_or(0));
                if let Some((_, slot)) = &mut self.as_mut().rust_mut().mirror {
                    *slot = Some(stream);
                }
                self.as_mut().rust_mut().last_frame = Some(Instant::now());
                self.as_mut().rust_mut().opened_at = Some(Instant::now());
                self.as_mut().set_state(QString::from("live"));
                hub::send(Command::MirrorKeyframe(device.to_owned()));
            }
            Err(reason) if asked => {
                let reconnecting = self.state == QString::from("reconnecting");
                self.ended(&reason, reconnecting);
            }
            Ok(None) | Err(_) => {}
        }
    }

    /// The stream closed, or never opened: reopened after a break, else back to idle, with the reason unless the user
    /// stopped it.
    fn ended(mut self: Pin<&mut Self>, reason: &str, broke: bool) {
        let mut state = self.as_mut().rust_mut();
        let phone = state.mirror.take().map(|(phone, _)| phone);
        state.held.clear();
        state.last_frame = None;
        state.opened_at = None;
        let stopped = std::mem::take(&mut state.stopped);
        if let Some(phone) = &phone {
            self.as_mut().restore_screen(phone);
        }
        let tries = self.tries;
        if let Some(phone) = phone.filter(|_| broke && !stopped && tries < RETRIES) {
            log::info!("the mirror broke ({reason}), reopening in {} s", 1 << tries);
            let mut state = self.as_mut().rust_mut();
            state.tries += 1;
            state.mirror = Some((phone, None));
            self.as_mut().set_error(QString::default());
            self.as_mut().set_state(QString::from("reconnecting"));
            let thread = self.qt_thread();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(1 << tries));
                let _ = thread.queue(qobject::LinkScreen::reopen);
            });
            return;
        }
        self.as_mut().rust_mut().tries = 0;
        let reason = if stopped {
            ""
        } else if reason.is_empty() {
            "The mirror stopped"
        } else {
            reason
        };
        self.as_mut().set_error(QString::from(reason));
        self.set_state(QString::from("idle"));
    }

    fn reopen(self: Pin<&mut Self>) {
        if self.state != QString::from("reconnecting") {
            return;
        }
        if let Some((phone, None)) = &self.mirror {
            hub::send(Command::OpenMirror(phone.clone()));
        }
    }

    fn restore_screen(mut self: Pin<&mut Self>, phone: &str) {
        if self.screen_off {
            self.as_mut().set_screen_off(false);
            let device = phone.to_owned();
            hub::send(Command::MirrorInput { device, action: "screen-on", points: Vec::new(), ms: None, text: None });
        }
    }

    /// A live stream that stopped bringing pictures shows as reconnecting until they come again.
    fn check_stall(mut self: Pin<&mut Self>) {
        let stalled = self.last_frame.is_some_and(|last| last.elapsed() > STALL);
        if stalled && self.state == QString::from("live") {
            log::info!("no picture for {} s", STALL.as_secs());
            self.as_mut().set_state(QString::from("reconnecting"));
        }
    }

    fn show(mut self: Pin<&mut Self>) {
        let frame = self.pending.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(frame) = frame {
            let open = matches!(self.mirror, Some((_, Some(_))));
            if open {
                self.as_mut().rust_mut().last_frame = Some(Instant::now());
                if self.tries > 0 && self.opened_at.is_some_and(|at| at.elapsed() >= STEADY) {
                    self.as_mut().rust_mut().tries = 0;
                }
                if self.state == QString::from("reconnecting") {
                    self.as_mut().set_state(QString::from("live"));
                }
            }
            // A rotated phone sends a new size mid-stream.
            if (frame.width, frame.height) != (self.video_width, self.video_height) {
                self.as_mut().set_video_width(frame.width);
                self.as_mut().set_video_height(frame.height);
            }
            // SAFETY: the page clears the sink before the sink goes, and both live on this thread.
            unsafe { qobject::show_nv12(self.sink, frame.width, frame.height, frame.stride, frame.data.as_slice()) };
        }
    }

    unsafe fn set_sink(self: Pin<&mut Self>, sink: *mut qobject::QVideoSink) {
        self.rust_mut().sink = sink;
    }

    fn start(mut self: Pin<&mut Self>) {
        if self.mirror.is_some() {
            return;
        }
        let phone = self.phone.clone();
        if phone.is_empty() {
            return hub::publish(Event::Failed("No phone is paired".to_owned()));
        }
        self.as_mut().rust_mut().mirror = Some((phone.clone(), None));
        self.as_mut().set_error(QString::default());
        self.as_mut().set_state(QString::from("starting"));
        hub::send(Command::OpenMirror(phone));
    }

    fn stop(mut self: Pin<&mut Self>) {
        if self.mirror.is_none() {
            return;
        }
        let open = matches!(&self.mirror, Some((_, Some(_))));
        self.as_mut().rust_mut().stopped = open;
        if !open {
            // Still asking: the answer finds no mirror and closes its socket.
            self.ended("", false);
            return;
        }
        if let Some((phone, _)) = &self.mirror {
            let phone = phone.clone();
            self.as_mut().restore_screen(&phone);
        }
        if let Some((_, stream)) = &mut self.as_mut().rust_mut().mirror {
            // The decoder's end runs `ended`.
            drop(stream.take());
        }
    }

    fn send(&self, action: &'static str, points: Vec<(&'static str, i32)>, ms: Option<u32>, text: Option<String>) {
        match &self.mirror {
            Some((device, Some(_))) => {
                hub::send(Command::MirrorInput { device: device.clone(), action, points, ms, text });
            }
            _ => hub::publish(Event::Failed("The screen is not mirrored".to_owned())),
        }
    }

    fn back(self: Pin<&mut Self>) {
        self.send("back", Vec::new(), None, None);
    }

    fn home(self: Pin<&mut Self>) {
        self.send("home", Vec::new(), None, None);
    }

    fn recents(self: Pin<&mut Self>) {
        self.send("recents", Vec::new(), None, None);
    }

    fn input(self: Pin<&mut Self>, action: &QString, x: i32, y: i32, x2: i32, y2: i32) {
        let action = match action.to_string().as_str() {
            "touch-down" => "touch-down",
            "touch-move" => "touch-move",
            "touch-up" => "touch-up",
            "wheel" => return self.send("wheel", vec![("x", x), ("y", y), ("x2", x2), ("y2", y2)], None, None),
            other => return log::warn!("not a mirror action: {other}"),
        };
        self.send(action, vec![("x", x), ("y", y)], None, None);
    }

    fn key(mut self: Pin<&mut Self>, key: i32, modifiers: i32, text: &QString, pressed: bool) -> bool {
        if !pressed {
            let Some(at) = self.held.iter().position(|(held, _, _)| *held == key) else { return false };
            let (_, code, meta) = self.as_mut().rust_mut().held.remove(at);
            self.send("key-up", vec![("code", code), ("meta", meta)], None, None);
            return true;
        }
        match keys::map(key, modifiers, &text.to_string()) {
            Some(Key::Text(text)) => self.send("text", Vec::new(), None, Some(text)),
            Some(Key::Android { code, meta }) => {
                self.as_mut().rust_mut().held.push((key, code, meta));
                self.send("key-down", vec![("code", code), ("meta", meta)], None, None);
            }
            None => return false,
        }
        true
    }

    fn release_keys(mut self: Pin<&mut Self>) {
        for (_, code, meta) in std::mem::take(&mut self.as_mut().rust_mut().held) {
            self.send("key-up", vec![("code", code), ("meta", meta)], None, None);
        }
    }

    fn press(self: Pin<&mut Self>, code: i32) {
        self.send("key-down", vec![("code", code), ("meta", 0)], None, None);
        self.send("key-up", vec![("code", code), ("meta", 0)], None, None);
    }

    fn toggle_screen(mut self: Pin<&mut Self>) {
        if !matches!(&self.mirror, Some((_, Some(_)))) {
            return;
        }
        let off = !self.screen_off;
        self.as_mut().set_screen_off(off);
        self.send(if off { "screen-off" } else { "screen-on" }, Vec::new(), None, None);
    }

    fn type_text(self: Pin<&mut Self>, text: &QString) {
        self.send("text", Vec::new(), None, Some(text.to_string()));
    }

    fn keyframe(self: Pin<&mut Self>) {
        if let Some((device, Some(_))) = &self.mirror {
            hub::send(Command::MirrorKeyframe(device.clone()));
        }
    }
}

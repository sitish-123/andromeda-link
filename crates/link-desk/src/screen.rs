//! `LinkScreen`: the current phone's screen inside the Screen page, decoded here into the page's video sink; taps,
//! swipes, scrolls, keys, and Back/Home/Recents go back as `MirrorInput`.

use std::os::unix::net::UnixStream;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use crate::client::{Command, Event};
use crate::hub;
use crate::mirror::{Frame, Stream};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
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
        /// tap (x, y), long-press (x, y, ms), swipe (x, y, x2, y2, ms), or scroll (x, y and a delta x2, y2); points
        /// in 1/10000 of the screen.
        #[qinvokable]
        fn input(self: Pin<&mut LinkScreen>, action: &QString, x: i32, y: i32, x2: i32, y2: i32, ms: i32);
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
}

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
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkScreen {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            if let Event::Devices(_) | Event::Phone(_) | Event::MirrorOpened { .. } = event {
                let event = event.clone();
                let _ = thread.queue(move |screen| screen.apply(event));
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
                self.stop();
            }
            Event::MirrorOpened { device, opened } => self.opened(&device, opened),
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
                move || drop(ended.queue(|screen| screen.ended(""))),
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
                self.as_mut().set_state(QString::from("live"));
                hub::send(Command::MirrorKeyframe(device.to_owned()));
            }
            Err(reason) if asked => self.ended(&reason),
            Ok(None) | Err(_) => {}
        }
    }

    /// The stream closed, or never opened: back to idle, with the reason unless the user stopped it.
    fn ended(mut self: Pin<&mut Self>, reason: &str) {
        let mut state = self.as_mut().rust_mut();
        state.mirror = None;
        let stopped = std::mem::take(&mut state.stopped);
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

    fn show(self: Pin<&mut Self>) {
        let frame = self.pending.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(frame) = frame {
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
            self.ended("");
        } else if let Some((_, stream)) = &mut self.as_mut().rust_mut().mirror {
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

    fn input(self: Pin<&mut Self>, action: &QString, x: i32, y: i32, x2: i32, y2: i32, ms: i32) {
        let ms = u32::try_from(ms).ok();
        match action.to_string().as_str() {
            "tap" => self.send("tap", vec![("x", x), ("y", y)], None, None),
            "long-press" => self.send("long-press", vec![("x", x), ("y", y)], ms, None),
            "swipe" => self.send("swipe", vec![("x", x), ("y", y), ("x2", x2), ("y2", y2)], ms, None),
            "scroll" => self.send("scroll", vec![("x", x), ("y", y), ("x2", x2), ("y2", y2)], None, None),
            other => log::warn!("not a mirror action: {other}"),
        }
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

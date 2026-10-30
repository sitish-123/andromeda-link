//! `LinkScreen`: mirrors the current phone in `andromeda-link-mirror`'s window, and its navigation keys.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::pin::Pin;
use std::process::{Child, Command as Process, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;

use crate::client::{Command, Event};
use crate::hub;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, state)]
        #[qproperty(QString, error)]
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
    }

    impl cxx_qt::Threading for LinkScreen {}
    impl cxx_qt::Initialize for LinkScreen {}
}

pub struct ScreenRust {
    state: QString,
    error: QString,
    phone: String,
    /// Device id to its name, for the mirror's window title.
    names: HashMap<String, String>,
    /// The phone being mirrored, and its mirror process.
    mirror: Option<(String, Arc<Mutex<Child>>)>,
    /// The user stopped the mirror, so its exit is no failure.
    stopped: bool,
}

impl Default for ScreenRust {
    fn default() -> Self {
        Self {
            state: QString::from("idle"),
            error: QString::default(),
            phone: hub::phone(),
            names: HashMap::new(),
            mirror: None,
            stopped: false,
        }
    }
}

impl Drop for ScreenRust {
    fn drop(&mut self) {
        if let Some((_, child)) = &self.mirror {
            let _ = child.lock().unwrap_or_else(PoisonError::into_inner).kill();
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkScreen {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            if let Event::Devices(_) | Event::Phone(_) = event {
                let event = event.clone();
                let _ = thread.queue(move |screen| screen.apply(event));
            }
        });
    }
}

/// The mirror ships beside this binary; a build tree has it there too.
fn mirror_binary() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("andromeda-link-mirror")))
        .filter(|path| path.exists())
        .unwrap_or_else(|| "andromeda-link-mirror".into())
}

impl qobject::LinkScreen {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        match event {
            Event::Devices(devices) => {
                let names = devices.iter().map(|device| (device.id.clone(), device.name.clone())).collect();
                self.as_mut().rust_mut().names = names;
                let phone = self.mirror.as_ref().map(|(phone, _)| phone.clone());
                let gone =
                    phone.is_some_and(|phone| !devices.iter().any(|device| device.id == phone && device.connected));
                if gone {
                    self.stop();
                }
            }
            Event::Phone(id) => {
                self.as_mut().rust_mut().phone = id;
                self.stop();
            }
            _ => {}
        }
    }

    fn start(mut self: Pin<&mut Self>) {
        if self.mirror.is_some() {
            return;
        }
        let phone = self.phone.clone();
        if phone.is_empty() {
            return hub::publish(Event::Failed("No phone is paired".to_owned()));
        }
        let name = self.names.get(&phone).cloned().unwrap_or_else(|| "Phone".to_owned());
        let binary = mirror_binary();
        let spawned = Process::new(&binary).args([&phone, "--name", &name]).stderr(Stdio::piped()).spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                self.as_mut().set_error(QString::from(&format!("{}: {error}", binary.display())));
                return;
            }
        };
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));
        self.as_mut().rust_mut().mirror = Some((phone, Arc::clone(&child)));
        self.as_mut().set_error(QString::default());
        self.as_mut().set_state(QString::from("starting"));
        let thread = self.qt_thread();
        std::thread::spawn(move || {
            let mut failure = String::new();
            for line in stderr.into_iter().flat_map(|stderr| BufReader::new(stderr).lines().map_while(Result::ok)) {
                if line.contains("mirroring ") {
                    let _ = thread.queue(|screen| screen.set_state(QString::from("live")));
                } else if line.contains("ERROR") {
                    line.split_once("] ").map_or(line.as_str(), |(_, text)| text).clone_into(&mut failure);
                }
            }
            let status = child.lock().unwrap_or_else(PoisonError::into_inner).wait();
            if !status.is_ok_and(|status| status.success()) && failure.is_empty() {
                "The mirror stopped".clone_into(&mut failure);
            }
            let _ = thread.queue(move |mut screen| {
                let mut state = screen.as_mut().rust_mut();
                state.mirror = None;
                let stopped = std::mem::take(&mut state.stopped);
                screen.as_mut().set_error(QString::from(if stopped { "" } else { failure.as_str() }));
                screen.set_state(QString::from("idle"));
            });
        });
    }

    fn stop(mut self: Pin<&mut Self>) {
        if let Some((_, child)) = &self.mirror {
            let _ = child.lock().unwrap_or_else(PoisonError::into_inner).kill();
            self.as_mut().rust_mut().stopped = true;
        }
    }

    fn key(&self, action: &'static str) {
        match &self.mirror {
            Some((device, _)) => hub::send(Command::MirrorKey { device: device.clone(), action }),
            None => hub::publish(Event::Failed("The screen is not mirrored".to_owned())),
        }
    }

    fn back(self: Pin<&mut Self>) {
        self.key("back");
    }

    fn home(self: Pin<&mut Self>) {
        self.key("home");
    }

    fn recents(self: Pin<&mut Self>) {
        self.key("recents");
    }

    fn keyframe(self: Pin<&mut Self>) {
        if let Some((device, _)) = &self.mirror {
            hub::send(Command::MirrorKeyframe(device.clone()));
        }
    }
}

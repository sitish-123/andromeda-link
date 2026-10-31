//! No `StatusNotifierItem` tray off Linux; on macOS the menu bar item is QML (`MenuBarIcon.qml`).

use cxx_qt::CxxQtThread;

use crate::link::qobject::Link;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct State {
    pub name: String,
    pub connected: bool,
    pub battery: i32,
    pub last_seen: i64,
    pub dnd: bool,
    pub ringer: String,
}

pub fn wanted() -> bool {
    cfg!(target_os = "macos")
}

/// On macOS, closing the window keeps the app in the menu bar.
#[expect(clippy::needless_pass_by_value, reason = "the same signature as tray.rs, which keeps the thread")]
pub fn start(link: CxxQtThread<Link>) {
    if cfg!(target_os = "macos") {
        let _ = link.queue(|_| crate::desktop::ffi::keep_running_without_windows());
    }
}

pub fn show(_state: State) {}

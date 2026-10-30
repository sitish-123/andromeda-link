//! No tray icon on platforms without `StatusNotifierItem`.

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
    false
}

pub fn start(_link: CxxQtThread<Link>) {}

pub fn show(_state: State) {}

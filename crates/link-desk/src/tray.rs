//! The tray icon (`StatusNotifierItem`) on desktops other than Andromeda, whose shell has its own bar item.

use std::sync::{Mutex, OnceLock, PoisonError};

use cxx_qt::CxxQtThread;
use cxx_qt_lib::QString;
use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{Handle, TrayMethods};

use crate::client;
use crate::desktop::ffi;
use crate::format::age_text;
use crate::link::qobject::Link;

/// What the menu shows for the current phone.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct State {
    /// The phone's name; empty without a paired phone.
    pub name: String,
    pub connected: bool,
    /// Percent, or -1.
    pub battery: i32,
    /// Unix seconds, or 0.
    pub last_seen: i64,
    pub dnd: bool,
    /// ring, vibrate, silent, or empty.
    pub ringer: String,
}

const RINGER: [(&str, &str); 3] = [("ring", "Ring"), ("vibrate", "Vibrate"), ("silent", "Silent")];

static STATE: Mutex<Option<State>> = Mutex::new(None);
static TRAY: OnceLock<Handle<Tray>> = OnceLock::new();

struct Tray {
    link: CxxQtThread<Link>,
}

impl Tray {
    fn run(&self, action: impl FnOnce(std::pin::Pin<&mut Link>) + Send + 'static) {
        let _ = self.link.queue(action);
    }
}

/// True unless the desktop is Andromeda.
pub fn wanted() -> bool {
    !std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("andromeda"))
}

/// Shows the tray icon on the client's runtime; once it shows, closing the window keeps the app running.
pub fn start(link: CxxQtThread<Link>) {
    let Some(runtime) = client::runtime() else { return };
    runtime.spawn(async move {
        match (Tray { link: link.clone() }).spawn().await {
            Ok(handle) => {
                let _ = TRAY.set(handle);
                let _ = link.queue(|_| ffi::keep_running_without_windows());
            }
            Err(error) => log::info!("no tray icon: {error}"),
        }
    });
}

/// Updates the menu when the state changed.
pub fn show(state: State) {
    let mut current = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    if current.as_ref() == Some(&state) {
        return;
    }
    *current = Some(state);
    if let (Some(runtime), Some(handle)) = (client::runtime(), TRAY.get()) {
        runtime.spawn(handle.update(|_| ()));
    }
}

/// "Pixel 8, 80%", or "Pixel 8, last seen 5 min ago" while it is away.
fn headline(state: &State) -> String {
    if state.name.is_empty() {
        return "No phone paired".to_owned();
    }
    if state.connected {
        return if state.battery >= 0 { format!("{}, {}%", state.name, state.battery) } else { state.name.clone() };
    }
    match u64::try_from(state.last_seen).ok().filter(|&seen| seen > 0).map(age_text).as_deref() {
        Some("now") => format!("{}, last seen just now", state.name),
        Some(age) => format!("{}, last seen {age} ago", state.name),
        None => format!("{}, not connected", state.name),
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "org.andromeda.Link".to_owned()
    }

    fn title(&self) -> String {
        "Link".to_owned()
    }

    fn icon_name(&self) -> String {
        "phone-symbolic".to_owned()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let state = STATE.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_default();
        ksni::ToolTip { title: headline(&state), ..ksni::ToolTip::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.run(Link::raise_requested);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = STATE.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_default();
        let live = state.connected;
        let dnd = state.dnd;
        let item = |label: &str, enabled: bool, activate: fn(&mut Self)| {
            StandardItem { label: label.to_owned(), enabled, activate: Box::new(activate), ..StandardItem::default() }
                .into()
        };
        vec![
            item(&headline(&state), false, |_| {}),
            MenuItem::Separator,
            item("Ring", live, |tray| tray.run(Link::ring)),
            item("Send clipboard", live, |tray| tray.run(Link::send_clipboard)),
            CheckmarkItem {
                label: "Do not disturb".to_owned(),
                enabled: live,
                checked: dnd,
                activate: Box::new(move |tray: &mut Self| tray.run(move |link| link.set_dnd(!dnd))),
                ..CheckmarkItem::default()
            }
            .into(),
            SubMenu {
                label: "Ringer".to_owned(),
                enabled: live,
                submenu: vec![
                    RadioGroup {
                        selected: RINGER.iter().position(|(mode, _)| *mode == state.ringer).unwrap_or(usize::MAX),
                        select: Box::new(|tray: &mut Self, index| {
                            let mode = RINGER[index].0;
                            tray.run(move |link| link.set_ringer(&QString::from(mode)));
                        }),
                        options: RINGER
                            .iter()
                            .map(|(_, label)| RadioItem { label: (*label).to_owned(), ..RadioItem::default() })
                            .collect(),
                    }
                    .into(),
                ],
                ..SubMenu::default()
            }
            .into(),
            MenuItem::Separator,
            item("Open Link", true, |tray| tray.run(Link::raise_requested)),
            item("Quit", true, |tray| tray.run(|_| ffi::quit_app())),
        ]
    }
}

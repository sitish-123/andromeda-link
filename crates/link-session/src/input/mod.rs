//! The phone as the desktop's touchpad and keyboard, from the daemon's `RemoteInput` signal.

pub mod ei;
pub mod wayland;
pub mod x11;

pub enum Backend {
    Wayland(Box<wayland::Wayland>),
    Ei(Box<ei::Ei>),
    X11(Box<x11::X11>),
}

impl Backend {
    /// The wlr virtual pointer and keyboard, or `XTest`, whichever the session offers without asking the user.
    pub fn native() -> Option<Self> {
        crate::attempt("virtual pointer input", wayland::Wayland::connect())
            .map(|backend| Self::Wayland(Box::new(backend)))
            .or_else(|| crate::attempt("XTest input", x11::X11::connect()).map(|backend| Self::X11(Box::new(backend))))
    }

    pub async fn portal(session: &crate::portal::Session) -> Option<Self> {
        let connect = async { ei::Ei::connect(session.eis().await?).await.map(Some) };
        crate::attempt("libei input", connect.await).map(|backend| Self::Ei(Box::new(backend)))
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Wayland(_) => "wlr-virtual-pointer and virtual-keyboard",
            Self::Ei(_) => "libei on the RemoteDesktop portal",
            Self::X11(_) => "XTest",
        }
    }

    pub fn run(&mut self, action: Action) -> anyhow::Result<()> {
        match self {
            Self::Wayland(backend) => backend.run(action),
            Self::Ei(backend) => backend.run(action),
            Self::X11(backend) => backend.run(action),
        }
    }

    /// Handles what the backend's server sends; only libei's needs it, the others never return.
    pub async fn pump(&mut self) -> anyhow::Result<()> {
        match self {
            Self::Ei(backend) => backend.pump().await,
            Self::Wayland(_) | Self::X11(_) => std::future::pending().await,
        }
    }
}

/// evdev button codes.
const BUTTONS: [(&str, u32); 3] = [("left", 0x110), ("right", 0x111), ("middle", 0x112)];
const RETURN: u32 = 0xff0d;
const TAB: u32 = 0xff09;
/// The keys the phone names, as keysyms.
const KEYS: [(&str, u32); 15] = [
    ("enter", RETURN),
    ("backspace", 0xff08),
    ("delete", 0xffff),
    ("escape", 0xff1b),
    ("tab", TAB),
    ("space", 0x20),
    ("left", 0xff51),
    ("right", 0xff53),
    ("up", 0xff52),
    ("down", 0xff54),
    ("home", 0xff50),
    ("end", 0xff57),
    ("page-up", 0xff55),
    ("page-down", 0xff56),
    ("f5", 0xffc2),
];

/// A place on a shared monitor, for remote control's direct touch.
pub struct Point {
    /// The monitor's output name, as the `ScreenCast` portal's `mapping_id`.
    pub output: Option<String>,
    /// The monitor's position and size in the layout, in logical pixels.
    pub rect: (i32, i32, i32, i32),
    /// Fractions of the monitor's width and height.
    pub x: f64,
    pub y: f64,
}

pub enum Action {
    /// Relative motion in logical pixels.
    Move(f64, f64),
    Point(Point),
    Scroll(f64, f64),
    Button {
        code: u32,
        pressed: bool,
    },
    Click(u32),
    Key(u32),
    Text(String),
}

impl Action {
    /// The signal's `(action, dx, dy, detail)`; `None` for one this desktop does not know.
    pub fn parse(action: &str, dx: i32, dy: i32, detail: String) -> Option<Self> {
        let button = || BUTTONS.iter().find(|(name, _)| *name == detail).map(|(_, code)| *code);
        match action {
            "move" => Some(Self::Move(dx.into(), dy.into())),
            "scroll" => Some(Self::Scroll(dx.into(), dy.into())),
            "press" | "release" => Some(Self::Button { code: button()?, pressed: action == "press" }),
            "click" => Some(Self::Click(button()?)),
            "key" => KEYS.iter().find(|(name, _)| *name == detail).map(|(_, keysym)| Self::Key(*keysym)),
            "text" => Some(Self::Text(detail)),
            _ => None,
        }
    }
}

/// The keysym that types `c`, as `xkb_utf32_to_keysym` maps it; `None` for a control character.
pub fn keysym(c: char) -> Option<u32> {
    let point = u32::from(c);
    match c {
        '\n' | '\r' => Some(RETURN),
        '\t' => Some(TAB),
        _ if point < 0x20 || (0x7f..0xa0).contains(&point) => None,
        _ if point <= 0xff => Some(point),
        _ => Some(0x0100_0000 + point),
    }
}

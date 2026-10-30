//! Input through `XTest` in an X11 session.

use std::collections::HashMap;

use anyhow::Context;
use x11rb::NONE;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt as _, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, Keycode, Keysym,
    MOTION_NOTIFY_EVENT,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::{Action, keysym};

/// Logical pixels of scrolling per wheel click, as libinput reports a click.
const SCROLL_STEP: f64 = 15.0;

pub struct X11 {
    conn: RustConnection,
    /// Keycodes with no keysym, lent in turn to the keysyms the phone types.
    spare: Vec<Keycode>,
    lent: HashMap<Keysym, Keycode>,
    next: usize,
    /// Scrolling below one wheel click, kept for the next scroll.
    scroll: (f64, f64),
}

impl X11 {
    /// `None` outside an X11 session; under Xwayland it would reach only X11 clients.
    pub fn connect() -> anyhow::Result<Option<Self>> {
        if crate::on_wayland() || std::env::var_os("DISPLAY").is_none() {
            return Ok(None);
        }
        let (conn, _) = RustConnection::connect(None).context("connecting to the X server")?;
        conn.xtest_get_version(2, 2)?.reply().context("the X server has no XTest")?;
        let setup = conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let mapping = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        let per = usize::from(mapping.keysyms_per_keycode);
        let spare = (min..=max)
            .zip(mapping.keysyms.chunks(per))
            .filter(|(_, keysyms)| keysyms.iter().all(|keysym| *keysym == 0))
            .map(|(code, _)| code)
            .collect::<Vec<_>>();
        anyhow::ensure!(!spare.is_empty(), "the keymap has no free keycode for typing");
        Ok(Some(Self { conn, spare, lent: HashMap::new(), next: 0, scroll: (0.0, 0.0) }))
    }

    pub fn run(&mut self, action: Action) -> anyhow::Result<()> {
        match action {
            Action::Move(dx, dy) => {
                // XTest's motion is relative when detail is 1.
                #[expect(clippy::cast_possible_truncation, reason = "a move from the phone is a few pixels")]
                self.conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 1, 0, NONE, dx as i16, dy as i16, 0)?;
            }
            Action::Point(point) => {
                let (x, y, width, height) = point.rect;
                #[expect(clippy::cast_possible_truncation, reason = "a point on the screen")]
                let (x, y) = (
                    (f64::from(x) + point.x * f64::from(width)) as i16,
                    (f64::from(y) + point.y * f64::from(height)) as i16,
                );
                let root = self.conn.setup().roots.first().map_or(NONE, |screen| screen.root);
                self.conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, root, x, y, 0)?;
            }
            Action::Scroll(dx, dy) => self.scroll(dx, dy)?,
            Action::Button { code, pressed } => self.button(code, pressed)?,
            Action::Click(code) => {
                self.button(code, true)?;
                self.button(code, false)?;
            }
            Action::Key(keysym) => self.key(keysym)?,
            Action::Text(text) => {
                for keysym in text.chars().filter_map(keysym) {
                    self.key(keysym)?;
                }
            }
        }
        self.conn.flush()?;
        Ok(())
    }

    fn button(&self, code: u32, pressed: bool) -> anyhow::Result<()> {
        // X11 numbers the middle button 2 and the right one 3.
        let button = match code {
            0x110 => 1,
            0x112 => 2,
            0x111 => 3,
            _ => return Ok(()),
        };
        let kind = if pressed { BUTTON_PRESS_EVENT } else { BUTTON_RELEASE_EVENT };
        self.conn.xtest_fake_input(kind, button, 0, NONE, 0, 0, 0)?;
        Ok(())
    }

    /// X11 scrolls in wheel clicks: buttons 4 and 5 up and down, 6 and 7 left and right.
    fn scroll(&mut self, dx: f64, dy: f64) -> anyhow::Result<()> {
        self.scroll.0 += dx;
        self.scroll.1 += dy;
        for (total, back, forward) in [(&mut self.scroll.1, 4, 5), (&mut self.scroll.0, 6, 7)] {
            while total.abs() >= SCROLL_STEP {
                let button = if *total > 0.0 { forward } else { back };
                *total -= SCROLL_STEP.copysign(*total);
                self.conn.xtest_fake_input(BUTTON_PRESS_EVENT, button, 0, NONE, 0, 0, 0)?;
                self.conn.xtest_fake_input(BUTTON_RELEASE_EVENT, button, 0, NONE, 0, 0, 0)?;
            }
        }
        Ok(())
    }

    fn key(&mut self, keysym: Keysym) -> anyhow::Result<()> {
        let code = match self.lent.get(&keysym) {
            Some(code) => *code,
            None => self.lend(keysym)?,
        };
        self.conn.xtest_fake_input(KEY_PRESS_EVENT, code, 0, NONE, 0, 0, 0)?;
        self.conn.xtest_fake_input(KEY_RELEASE_EVENT, code, 0, NONE, 0, 0, 0)?;
        Ok(())
    }

    /// Maps a spare keycode to `keysym` on both levels, so Shift does not change it.
    fn lend(&mut self, keysym: Keysym) -> anyhow::Result<Keycode> {
        let code = self.spare[self.next % self.spare.len()];
        self.next += 1;
        self.lent.retain(|_, lent| *lent != code);
        self.conn.change_keyboard_mapping(1, code, 2, &[keysym, keysym])?;
        // A round trip, so the new mapping is in place before the key is pressed.
        self.conn.get_input_focus()?.reply()?;
        self.lent.insert(keysym, code);
        Ok(code)
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        for code in self.lent.values() {
            let _ = self.conn.change_keyboard_mapping(1, *code, 2, &[0, 0]);
        }
        let _ = self.conn.flush();
    }
}

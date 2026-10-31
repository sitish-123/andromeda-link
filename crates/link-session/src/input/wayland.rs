//! Input through `wlr-virtual-pointer-unstable-v1` and `virtual-keyboard-unstable-v1` (Sway, Hyprland, niri, labwc,
//! river, Wayfire, Andromeda).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write as _;
use std::os::fd::AsFd;
use std::time::Instant;

use anyhow::Context;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_keyboard::{KeyState, KeymapFormat};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
use wayland_client::protocol::{wl_registry::WlRegistry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

use super::{Action, keysym};

/// Xkb keycodes start at 8, and 8 itself is never a key.
const FIRST_KEYCODE: u32 = 9;
/// Codes are recycled past this, since the keymap goes to every focused client.
const MAX_KEYSYMS: usize = 200;
/// An absolute motion's extent: a point is this many steps across its output.
const EXTENT: u32 = 10_000;

/// The outputs' names, as `wl_output` version 4 tells them, by the index each was bound with.
#[derive(Default)]
pub struct State {
    names: HashMap<usize, String>,
}

pub struct Wayland {
    conn: Connection,
    _queue: EventQueue<State>,
    qh: QueueHandle<State>,
    seat: WlSeat,
    pointers: ZwlrVirtualPointerManagerV1,
    pointer: ZwlrVirtualPointerV1,
    /// Each named output, and the pointer bound to it once remote control pointed there.
    outputs: HashMap<String, (WlOutput, Option<ZwlrVirtualPointerV1>)>,
    keyboard: ZwpVirtualKeyboardV1,
    /// The keysym behind each keycode from `FIRST_KEYCODE` on.
    keysyms: Vec<u32>,
    start: Instant,
}

impl Wayland {
    /// `None` when the compositor lacks either protocol.
    pub fn connect() -> anyhow::Result<Option<Self>> {
        if !crate::on_wayland() {
            return Ok(None);
        }
        let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let Ok(seat) = globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()) else { return Ok(None) };
        let Ok(pointers) = globals.bind::<ZwlrVirtualPointerManagerV1, _, _>(&qh, 1..=2, ()) else { return Ok(None) };
        let Ok(keyboards) = globals.bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ()) else { return Ok(None) };
        let pointer = pointers.create_virtual_pointer(Some(&seat), &qh, ());
        let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());
        // Outputs bound now are enough: remote control points only at monitors the portal shared.
        let bound: Vec<WlOutput> = globals
            .contents()
            .clone_list()
            .into_iter()
            .filter(|global| global.interface == WlOutput::interface().name && global.version >= 4)
            .enumerate()
            .map(|(index, global)| globals.registry().bind::<WlOutput, _, _>(global.name, 4, &qh, index))
            .collect();
        let mut state = State::default();
        queue.roundtrip(&mut state)?;
        let outputs = bound
            .into_iter()
            .enumerate()
            .filter_map(|(index, output)| Some((state.names.remove(&index)?, (output, None))))
            .collect();
        let mut input = Self {
            conn,
            _queue: queue,
            qh,
            seat,
            pointers,
            pointer,
            outputs,
            keyboard,
            keysyms: Vec::new(),
            start: Instant::now(),
        };
        input.upload_keymap()?;
        input.conn.flush()?;
        Ok(Some(input))
    }

    pub fn run(&mut self, action: Action) -> anyhow::Result<()> {
        let time = self.time();
        match action {
            Action::Move(dx, dy) => {
                self.pointer.motion(time, dx, dy);
                self.pointer.frame();
            }
            Action::Point(point) => self.point(time, &point),
            Action::Scroll(dx, dy) => {
                // Continuous, so no axis_stop is owed when the fingers lift.
                self.pointer.axis_source(AxisSource::Continuous);
                if dy != 0.0 {
                    self.pointer.axis(time, Axis::VerticalScroll, dy);
                }
                if dx != 0.0 {
                    self.pointer.axis(time, Axis::HorizontalScroll, dx);
                }
                self.pointer.frame();
            }
            Action::Button { code, pressed } => self.button(time, code, pressed),
            Action::Click(code) => {
                self.button(time, code, true);
                self.button(time, code, false);
            }
            Action::Key(keysym) => self.key(keysym)?,
            Action::Text(text) => {
                for keysym in text.chars().filter_map(keysym) {
                    self.key(keysym)?;
                }
            }
            Action::Shortcut(modifiers, keysym) => {
                let mask = modifiers.iter().fold(0, |mask, modifier| mask | modifier.mask());
                self.keyboard.modifiers(mask, 0, 0, 0);
                self.key(keysym)?;
                self.keyboard.modifiers(0, 0, 0, 0);
            }
        }
        self.conn.flush()?;
        Ok(())
    }

    /// Absolute motion through a pointer bound to the point's output, which maps the extent onto that output alone.
    fn point(&mut self, time: u32, point: &super::Point) {
        let bindable = self.pointers.version() >= 2;
        let Some((output, pointer)) =
            point.output.as_ref().and_then(|name| self.outputs.get_mut(name)).filter(|_| bindable)
        else {
            return log::debug!("no output {:?} to point at", point.output);
        };
        let pointer = pointer.get_or_insert_with(|| {
            self.pointers.create_virtual_pointer_with_output(Some(&self.seat), Some(output), &self.qh, ())
        });
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a fraction of the extent")]
        let (x, y) = (
            (point.x.clamp(0.0, 1.0) * f64::from(EXTENT)) as u32,
            (point.y.clamp(0.0, 1.0) * f64::from(EXTENT)) as u32,
        );
        pointer.motion_absolute(time, x, y, EXTENT, EXTENT);
        pointer.frame();
    }

    fn button(&self, time: u32, code: u32, pressed: bool) {
        self.pointer.button(time, code, if pressed { ButtonState::Pressed } else { ButtonState::Released });
        self.pointer.frame();
    }

    fn key(&mut self, keysym: u32) -> anyhow::Result<()> {
        let index = if let Some(index) = self.keysyms.iter().position(|known| *known == keysym) {
            index
        } else {
            if self.keysyms.len() >= MAX_KEYSYMS {
                self.keysyms.clear();
            }
            self.keysyms.push(keysym);
            self.upload_keymap()?;
            self.keysyms.len() - 1
        };
        // The protocol's key codes are evdev codes, xkb's minus 8.
        let evdev = FIRST_KEYCODE + u32::try_from(index)? - 8;
        let time = self.time();
        self.keyboard.key(time, evdev, KeyState::Pressed.into());
        self.keyboard.key(time, evdev, KeyState::Released.into());
        Ok(())
    }

    /// The keymap the compositor compiles: one key per keysym typed so far, no modifiers.
    fn upload_keymap(&mut self) -> anyhow::Result<()> {
        let mut keycodes = String::new();
        let mut symbols = String::new();
        for (index, keysym) in (0u32..).zip(&self.keysyms) {
            writeln!(keycodes, "<K{index}> = {};", FIRST_KEYCODE + index)?;
            writeln!(symbols, "key <K{index}> {{ [ 0x{keysym:x} ] }};")?;
        }
        let maximum = FIRST_KEYCODE + u32::try_from(self.keysyms.len().max(1))?;
        let keymap = format!(
            "xkb_keymap {{\nxkb_keycodes \"link\" {{\nminimum = 8;\nmaximum = {maximum};\n{keycodes}}};\n\
             xkb_types \"link\" {{ include \"complete\" }};\nxkb_compatibility \"link\" {{ include \"complete\" }};\n\
             xkb_symbols \"link\" {{\n{symbols}}};\n}};\n\0"
        );
        let fd = rustix::fs::memfd_create("link-keymap", rustix::fs::MemfdFlags::CLOEXEC)?;
        let mut file = std::fs::File::from(fd);
        file.write_all(keymap.as_bytes())?;
        self.keyboard.keymap(KeymapFormat::XkbV1.into(), file.as_fd(), u32::try_from(keymap.len())?);
        Ok(())
    }

    /// Milliseconds on a monotonic clock, as the protocols' timestamps are.
    fn time(&self) -> u32 {
        #[expect(clippy::cast_possible_truncation, reason = "the protocols' timestamps wrap at 32 bits")]
        let time = self.start.elapsed().as_millis() as u32;
        time
    }
}

impl Drop for Wayland {
    fn drop(&mut self) {
        self.pointer.destroy();
        for pointer in self.outputs.values().filter_map(|(_, pointer)| pointer.as_ref()) {
            pointer.destroy();
        }
        self.keyboard.destroy();
        let _ = self.conn.flush();
    }
}

macro_rules! ignore_events {
    ($($proxy:ty),+) => {
        $(impl Dispatch<$proxy, ()> for State {
            fn event(_: &mut Self, _: &$proxy, _: <$proxy as Proxy>::Event, &(): &(), _: &Connection,
                _: &QueueHandle<Self>) {
            }
        })+
    };
}

ignore_events!(
    WlSeat,
    ZwlrVirtualPointerManagerV1,
    ZwlrVirtualPointerV1,
    ZwpVirtualKeyboardManagerV1,
    ZwpVirtualKeyboardV1
);

impl Dispatch<WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            state.names.insert(*index, name);
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

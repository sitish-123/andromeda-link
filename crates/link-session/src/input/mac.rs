//! The phone as touchpad and keyboard through `CGEvent`. macOS posts the events only after the user lets this program
//! control the computer (System Settings, Privacy & Security, Accessibility).

#![allow(unsafe_code, reason = "FFI to macOS frameworks")]

use std::time::{Duration, Instant};

use anyhow::Context;
use objc2_application_services::{AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained, CGPoint, CGRect};
use objc2_core_graphics::{
    CGDirectDisplayID, CGDisplayBounds, CGEvent, CGEventField, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventType, CGGetActiveDisplayList, CGMouseButton, CGScrollEventUnit,
};

use super::{Action, RETURN, TAB, keysym};

const MAX_DISPLAYS: u32 = 16;
/// `CGEventKeyboardSetUnicodeString` takes at most 20 UTF-16 units per event.
const TEXT_CHUNK: usize = 20;
/// A second click of the same button this soon counts as a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
/// The phone's keysyms as macOS virtual key codes (`kVK_*`).
const KEYCODES: [(u32, u16); 15] = [
    (RETURN, 0x24),
    (0xff08, 0x33),
    (0xffff, 0x75),
    (0xff1b, 0x35),
    (TAB, 0x30),
    (0x20, 0x31),
    (0xff51, 0x7b),
    (0xff53, 0x7c),
    (0xff52, 0x7e),
    (0xff54, 0x7d),
    (0xff50, 0x73),
    (0xff57, 0x77),
    (0xff55, 0x74),
    (0xff56, 0x79),
    (0xffc2, 0x60),
];

struct Button {
    code: u32,
    mouse: CGMouseButton,
    down: CGEventType,
    up: CGEventType,
    dragged: CGEventType,
}

/// The evdev buttons the phone names.
const BUTTONS: [Button; 3] = [
    Button {
        code: 0x110,
        mouse: CGMouseButton::Left,
        down: CGEventType::LeftMouseDown,
        up: CGEventType::LeftMouseUp,
        dragged: CGEventType::LeftMouseDragged,
    },
    Button {
        code: 0x111,
        mouse: CGMouseButton::Right,
        down: CGEventType::RightMouseDown,
        up: CGEventType::RightMouseUp,
        dragged: CGEventType::RightMouseDragged,
    },
    Button {
        code: 0x112,
        mouse: CGMouseButton::Center,
        down: CGEventType::OtherMouseDown,
        up: CGEventType::OtherMouseUp,
        dragged: CGEventType::OtherMouseDragged,
    },
];

pub struct Mac {
    source: CFRetained<CGEventSource>,
    /// The button held down, so a move drags.
    held: Option<&'static Button>,
    /// The last press as (button code, time, click count), as apps read a double click from the count.
    last_press: Option<(u32, Instant, i64)>,
}

impl Mac {
    /// `None` until the user gives the Accessibility permission; without it, the call shows the system's prompt.
    pub fn connect() -> anyhow::Result<Option<Self>> {
        // SAFETY: the key is a static CFString, and the option takes a CFBoolean.
        let options = CFDictionary::from_slices(&[unsafe { kAXTrustedCheckOptionPrompt }], &[CFBoolean::new(true)]);
        // SAFETY: the options are as the call documents them.
        if !unsafe { AXIsProcessTrustedWithOptions(Some(options.as_ref())) } {
            log::warn!("remote input needs the Accessibility permission; give it, then restart link-session");
            return Ok(None);
        }
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).context("creating an event source")?;
        Ok(Some(Self { source, held: None, last_press: None }))
    }

    pub fn run(&mut self, action: Action) -> anyhow::Result<()> {
        match action {
            Action::Move(dx, dy) => {
                let at = location();
                self.move_to(clamp(CGPoint { x: at.x + dx, y: at.y + dy }, at), Some((dx, dy)))?;
            }
            Action::Point(point) => {
                let (x, y, width, height) = point.rect;
                let to = CGPoint {
                    x: f64::from(x) + f64::from(width) * point.x.clamp(0.0, 1.0),
                    y: f64::from(y) + f64::from(height) * point.y.clamp(0.0, 1.0),
                };
                self.move_to(to, None)?;
            }
            Action::Scroll(dx, dy) => {
                // macOS scrolls content down for a positive wheel value, the other way from Wayland's axis.
                #[expect(clippy::cast_possible_truncation, reason = "a scroll from the phone is a few pixels")]
                let event = CGEvent::new_scroll_wheel_event2(
                    Some(&self.source),
                    CGScrollEventUnit::Pixel,
                    2,
                    -dy as i32,
                    -dx as i32,
                    0,
                )
                .context("creating a scroll event")?;
                CGEvent::set_integer_value_field(Some(&event), CGEventField::ScrollWheelEventIsContinuous, 1);
                post(&event);
            }
            Action::Button { code, pressed } => self.button(code, pressed)?,
            Action::Click(code) => {
                self.button(code, true)?;
                self.button(code, false)?;
            }
            Action::Key(keysym) => self.key(keysym)?,
            Action::Text(text) => self.text(&text)?,
        }
        Ok(())
    }

    fn move_to(&self, to: CGPoint, delta: Option<(f64, f64)>) -> anyhow::Result<()> {
        let (kind, button) = match self.held {
            Some(held) => (held.dragged, held.mouse),
            None => (CGEventType::MouseMoved, CGMouseButton::Left),
        };
        let event = CGEvent::new_mouse_event(Some(&self.source), kind, to, button).context("creating a move")?;
        if let Some((dx, dy)) = delta {
            #[expect(clippy::cast_possible_truncation, reason = "a move from the phone is a few pixels")]
            for (field, value) in [(CGEventField::MouseEventDeltaX, dx), (CGEventField::MouseEventDeltaY, dy)] {
                CGEvent::set_integer_value_field(Some(&event), field, value as i64);
            }
        }
        post(&event);
        Ok(())
    }

    fn button(&mut self, code: u32, pressed: bool) -> anyhow::Result<()> {
        let Some(button) = BUTTONS.iter().find(|button| button.code == code) else { return Ok(()) };
        let clicks = if pressed {
            let now = Instant::now();
            let clicks = match self.last_press {
                Some((last, at, clicks)) if last == code && now.duration_since(at) < DOUBLE_CLICK => clicks + 1,
                _ => 1,
            };
            self.last_press = Some((code, now, clicks));
            self.held = Some(button);
            clicks
        } else {
            self.held = None;
            self.last_press.map_or(1, |(_, _, clicks)| clicks)
        };
        let kind = if pressed { button.down } else { button.up };
        let event =
            CGEvent::new_mouse_event(Some(&self.source), kind, location(), button.mouse).context("creating a click")?;
        CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventClickState, clicks);
        post(&event);
        Ok(())
    }

    fn key(&self, keysym: u32) -> anyhow::Result<()> {
        let Some(&(_, code)) = KEYCODES.iter().find(|(known, _)| *known == keysym) else {
            return char::from_u32(keysym).map_or(Ok(()), |c| self.type_units(&c.to_string()));
        };
        for down in [true, false] {
            let event = CGEvent::new_keyboard_event(Some(&self.source), code, down).context("creating a key event")?;
            post(&event);
        }
        Ok(())
    }

    /// Types `text`; Return and Tab go as keys, so apps act on them.
    fn text(&self, text: &str) -> anyhow::Result<()> {
        let mut run = String::new();
        for c in text.chars() {
            match keysym(c) {
                Some(key @ (RETURN | TAB)) => {
                    self.type_units(&std::mem::take(&mut run))?;
                    self.key(key)?;
                }
                Some(_) => run.push(c),
                None => {}
            }
        }
        self.type_units(&run)
    }

    /// Types `text` as Unicode strings on key events, a few characters per event.
    fn type_units(&self, text: &str) -> anyhow::Result<()> {
        let mut units: Vec<u16> = Vec::new();
        let mut chars = text.chars().peekable();
        while chars.peek().is_some() {
            units.clear();
            while let Some(c) = chars.peek() {
                if units.len() + c.len_utf16() > TEXT_CHUNK {
                    break;
                }
                let mut buffer = [0; 2];
                units.extend_from_slice(c.encode_utf16(&mut buffer));
                chars.next();
            }
            for down in [true, false] {
                let event = CGEvent::new_keyboard_event(Some(&self.source), 0, down).context("creating a key event")?;
                // SAFETY: `units` holds the length given.
                unsafe { CGEvent::keyboard_set_unicode_string(Some(&event), units.len() as _, units.as_ptr()) };
                post(&event);
            }
        }
        Ok(())
    }
}

fn post(event: &CGEvent) {
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(event));
}

/// The pointer's place in global display coordinates.
fn location() -> CGPoint {
    CGEvent::location(CGEvent::new(None).as_deref())
}

/// `to`, or when it is on no display, the nearest point to it on the display that holds `from`.
fn clamp(to: CGPoint, from: CGPoint) -> CGPoint {
    let displays = displays();
    let contains = |rect: &CGRect, p: CGPoint| {
        (rect.origin.x..rect.origin.x + rect.size.width).contains(&p.x)
            && (rect.origin.y..rect.origin.y + rect.size.height).contains(&p.y)
    };
    if displays.is_empty() || displays.iter().any(|rect| contains(rect, to)) {
        return to;
    }
    let rect = displays.iter().find(|rect| contains(rect, from)).unwrap_or(&displays[0]);
    CGPoint {
        x: to.x.clamp(rect.origin.x, rect.origin.x + rect.size.width - 1.0),
        y: to.y.clamp(rect.origin.y, rect.origin.y + rect.size.height - 1.0),
    }
}

fn displays() -> Vec<CGRect> {
    let mut ids: [CGDirectDisplayID; MAX_DISPLAYS as usize] = [0; MAX_DISPLAYS as usize];
    let mut count = 0;
    // SAFETY: `ids` holds `MAX_DISPLAYS` entries.
    let error = unsafe { CGGetActiveDisplayList(MAX_DISPLAYS, ids.as_mut_ptr(), &raw mut count) };
    if error.0 != 0 {
        return Vec::new();
    }
    ids.iter().take(count as usize).map(|id| CGDisplayBounds(*id)).collect()
}

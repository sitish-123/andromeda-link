//! The phone as a real touchpad: a uinput multitouch device fed the phone's raw finger contacts, so libinput applies
//! this desktop's own touchpad settings (tap, natural scroll, speed) and the compositor's swipe gestures.

use evdev::uinput::VirtualDevice;
use evdev::{AbsInfo, AbsoluteAxisCode as Abs, AttributeSet, InputEvent, KeyCode, PropType, UinputAbsSetup};

/// Slots the device reports: link-proto's `MAX_TOUCH_CONTACTS`.
const SLOTS: usize = 5;
/// Positions are 1/100 mm, so libinput measures real distances.
const UNITS_PER_MM: i32 = 100;
const MAX: i32 = 30_000;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
/// One, two, three, four and five fingers down.
const TOOLS: [KeyCode; SLOTS] = [
    KeyCode::BTN_TOOL_FINGER,
    KeyCode::BTN_TOOL_DOUBLETAP,
    KeyCode::BTN_TOOL_TRIPLETAP,
    KeyCode::BTN_TOOL_QUADTAP,
    KeyCode::BTN_TOOL_QUINTTAP,
];

/// Which phone finger each slot holds, and the tracking ids handed out so far.
#[derive(Default)]
pub struct Slots {
    held: [Option<u8>; SLOTS],
    tracking: i32,
}

impl Slots {
    /// The evdev events, before `SYN_REPORT`, that move the device from its last frame to `contacts`: every finger
    /// down now as (id, x, y).
    pub fn frame(&mut self, contacts: &[(u8, u32, u32)]) -> Vec<(u16, u16, i32)> {
        let mut events = Vec::new();
        let contacts = &contacts[..contacts.len().min(SLOTS)];
        for (slot, held) in self.held.iter_mut().enumerate() {
            if held.is_some_and(|id| !contacts.iter().any(|contact| contact.0 == id)) {
                *held = None;
                events.push((EV_ABS, Abs::ABS_MT_SLOT.0, index(slot)));
                events.push((EV_ABS, Abs::ABS_MT_TRACKING_ID.0, -1));
            }
        }
        let mut first = None;
        for &(id, x, y) in contacts {
            let (x, y) = (position(x), position(y));
            let held = self.held.iter().position(|held| *held == Some(id));
            let Some(slot) = held.or_else(|| self.held.iter().position(Option::is_none)) else { continue };
            events.push((EV_ABS, Abs::ABS_MT_SLOT.0, index(slot)));
            if held.is_none() {
                self.held[slot] = Some(id);
                self.tracking = self.tracking.wrapping_add(1) & 0xffff;
                events.push((EV_ABS, Abs::ABS_MT_TRACKING_ID.0, self.tracking));
            }
            events.push((EV_ABS, Abs::ABS_MT_POSITION_X.0, x));
            events.push((EV_ABS, Abs::ABS_MT_POSITION_Y.0, y));
            if first.is_none_or(|(at, _, _)| slot < at) {
                first = Some((slot, x, y));
            }
        }
        let down = self.held.iter().flatten().count();
        events.push((EV_KEY, KeyCode::BTN_TOUCH.0, i32::from(down > 0)));
        for (fingers, tool) in TOOLS.iter().enumerate() {
            events.push((EV_KEY, tool.0, i32::from(down == fingers + 1)));
        }
        if let Some((_, x, y)) = first {
            events.push((EV_ABS, Abs::ABS_X.0, x));
            events.push((EV_ABS, Abs::ABS_Y.0, y));
        }
        events
    }
}

fn index(slot: usize) -> i32 {
    i32::try_from(slot).unwrap_or(0)
}

fn position(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(MAX).min(MAX)
}

pub struct Touchpad {
    device: VirtualDevice,
    slots: Slots,
}

impl Touchpad {
    /// Creates the device; fails without write access to `/dev/uinput`.
    pub fn open() -> std::io::Result<Self> {
        let keys: AttributeSet<KeyCode> = [KeyCode::BTN_LEFT, KeyCode::BTN_TOUCH].into_iter().chain(TOOLS).collect();
        let props: AttributeSet<PropType> = [PropType::POINTER, PropType::BUTTONPAD].into_iter().collect();
        let axis = |code: Abs, maximum: i32, resolution: i32| {
            UinputAbsSetup::new(code, AbsInfo::new(0, 0, maximum, 0, 0, resolution))
        };
        let slots = index(SLOTS - 1);
        let device = VirtualDevice::builder()?
            .name("Andromeda Link touchpad")
            .with_keys(&keys)?
            .with_properties(&props)?
            .with_absolute_axis(&axis(Abs::ABS_X, MAX, UNITS_PER_MM))?
            .with_absolute_axis(&axis(Abs::ABS_Y, MAX, UNITS_PER_MM))?
            .with_absolute_axis(&axis(Abs::ABS_MT_SLOT, slots, 0))?
            .with_absolute_axis(&axis(Abs::ABS_MT_TRACKING_ID, 0xffff, 0))?
            .with_absolute_axis(&axis(Abs::ABS_MT_POSITION_X, MAX, UNITS_PER_MM))?
            .with_absolute_axis(&axis(Abs::ABS_MT_POSITION_Y, MAX, UNITS_PER_MM))?
            .build()?;
        Ok(Self { device, slots: Slots::default() })
    }

    pub fn touch(&mut self, contacts: &[(u8, u32, u32)]) -> std::io::Result<()> {
        let events: Vec<InputEvent> = self
            .slots
            .frame(contacts)
            .into_iter()
            .map(|(kind, code, value)| InputEvent::new(kind, code, value))
            .collect();
        self.device.emit(&events)
    }
}

#[cfg(test)]
mod tests {
    use super::{Abs, EV_ABS, EV_KEY, KeyCode, Slots};

    fn value(events: &[(u16, u16, i32)], kind: u16, code: u16) -> Option<i32> {
        events.iter().rev().find(|event| event.0 == kind && event.1 == code).map(|event| event.2)
    }

    #[test]
    fn a_second_finger_gets_its_own_slot_and_lifting_frees_it() {
        let mut slots = Slots::default();
        slots.frame(&[(3, 100, 200)]);
        let two = slots.frame(&[(3, 110, 200), (7, 500, 600)]);
        assert_eq!(value(&two, EV_KEY, KeyCode::BTN_TOOL_DOUBLETAP.0), Some(1));
        assert_eq!(value(&two, EV_KEY, KeyCode::BTN_TOOL_FINGER.0), Some(0));
        assert_eq!(value(&two, EV_ABS, Abs::ABS_MT_SLOT.0), Some(1));
        assert_eq!(value(&two, EV_ABS, Abs::ABS_X.0), Some(110));
        let lifted = slots.frame(&[(7, 500, 600)]);
        assert_eq!(lifted[..2], [(EV_ABS, Abs::ABS_MT_SLOT.0, 0), (EV_ABS, Abs::ABS_MT_TRACKING_ID.0, -1)]);
        assert_eq!(value(&lifted, EV_KEY, KeyCode::BTN_TOOL_FINGER.0), Some(1));
        let none = slots.frame(&[]);
        assert_eq!(value(&none, EV_KEY, KeyCode::BTN_TOUCH.0), Some(0));
        assert_eq!(value(&none, EV_ABS, Abs::ABS_MT_TRACKING_ID.0), Some(-1));
    }
}

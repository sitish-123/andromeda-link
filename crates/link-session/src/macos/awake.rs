//! Keeps the Mac's display from sleeping while a phone mirrors here or presents (the daemon's `KeepAwake`), with an
//! `IOKit` power assertion; macOS drops it too if this process exits.

#![allow(unsafe_code, reason = "FFI to macOS frameworks")]

use objc2_core_foundation::CFString;
use objc2_io_kit::{IOPMAssertionCreateWithName, IOPMAssertionID, IOPMAssertionRelease, kIOPMAssertionLevelOn};

/// `kIOPMAssertPreventUserIdleDisplaySleep`, a macro the bindings leave out.
const PREVENT_DISPLAY_SLEEP: &str = "PreventUserIdleDisplaySleep";

#[derive(Default)]
pub struct Awake {
    assertion: Option<IOPMAssertionID>,
}

impl Awake {
    pub fn set(&mut self, on: bool) {
        match (on, self.assertion) {
            (true, None) => {
                let mut id = 0;
                let kind = CFString::from_static_str(PREVENT_DISPLAY_SLEEP);
                let name = CFString::from_static_str("A phone shows its screen or presents");
                // SAFETY: both strings are valid, and `id` is a valid place for the result.
                let status = unsafe {
                    IOPMAssertionCreateWithName(Some(&kind), kIOPMAssertionLevelOn, Some(&name), &raw mut id)
                };
                if status == 0 {
                    log::info!("keeping the display awake");
                    self.assertion = Some(id);
                } else {
                    log::warn!("IOPMAssertionCreateWithName: {status:#x}");
                }
            }
            (false, Some(id)) => {
                self.assertion = None;
                match IOPMAssertionRelease(id) {
                    0 => log::info!("the display may sleep again"),
                    status => log::warn!("IOPMAssertionRelease: {status:#x}"),
                }
            }
            _ => {}
        }
    }
}

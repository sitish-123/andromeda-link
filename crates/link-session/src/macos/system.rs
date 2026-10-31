//! The Mac's state for phones and the commands they send. Public APIs give the battery (`IOKit` power sources), locked
//! (`CGSession`), idle (`CGEventSource`), sleep and display sleep (`pmset`). Lock presses the Lock Screen shortcut,
//! which needs the Accessibility permission. macOS has no public API to read or set brightness, Focus or the power
//! mode, so those stay out.

#![allow(unsafe_code, reason = "FFI to macOS frameworks")]
#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    clippy::unused_self,
    reason = "the Linux backend's interface, whose calls wait"
)]

use std::time::Duration;

use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{CGEventSource, CGEventSourceStateID, CGEventType, CGSessionCopyCurrentDictionary};
use objc2_io_kit::{IOPSCopyPowerSourcesInfo, IOPSCopyPowerSourcesList, IOPSGetPowerSourceDescription};

/// `IOKit` and `CGSession` send changes only to a run loop, so the battery, locked and idle are read this often.
const READ_EVERY: Duration = Duration::from_secs(2);

/// No input for this long makes the Mac idle.
const IDLE_AFTER_SECONDS: f64 = 300.0;

/// The fields of `desktop-status` after volume and mute; `None` for what this desktop cannot tell.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub battery: Option<(u8, bool)>,
    pub brightness: Option<u8>,
    pub locked: Option<bool>,
    pub idle: Option<bool>,
    pub dnd: Option<bool>,
    pub profile: Option<String>,
}

pub struct System {
    /// Battery, locked, idle.
    seen: (Option<(u8, bool)>, bool, bool),
    tick: tokio::time::Interval,
}

impl System {
    pub fn connect() -> Self {
        let mut tick = tokio::time::interval(READ_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self { seen: (battery(), locked(), idle()), tick }
    }

    /// What phones can ask of this desktop, as `SetSessionCapabilities` lists it.
    pub fn capabilities(&self) -> Vec<&'static str> {
        let mut can = vec!["suspend", "screen-off"];
        if crate::input::mac::trusted() {
            can.push("lock");
        }
        can
    }

    pub async fn read(&self) -> State {
        let (battery, locked, idle) = self.seen;
        State { battery, locked: Some(locked), idle: Some(idle), ..State::default() }
    }

    /// Waits for a change to the battery, locked or idle.
    pub async fn changed(&mut self) {
        loop {
            self.tick.tick().await;
            let now = (battery(), locked(), idle());
            if now != self.seen {
                self.seen = now;
                return;
            }
        }
    }

    /// Carries out a phone's command other than volume and mute; the error is for the user.
    pub async fn run(&mut self, command: &str, _value: u32) -> Result<(), String> {
        match command {
            "suspend" => pmset("sleepnow", "Sleeping").await,
            "screen-off" => pmset("displaysleepnow", "Screen off").await,
            "lock" => crate::input::mac::lock_screen().map_err(|error| format!("Lock failed: {error:#}")),
            other => Err(format!("{other} is not a command this Mac takes")),
        }
    }
}

async fn pmset(argument: &str, what: &str) -> Result<(), String> {
    let output = tokio::process::Command::new("/usr/bin/pmset")
        .arg(argument)
        .output()
        .await
        .map_err(|error| format!("{what} failed: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed: {}", String::from_utf8_lossy(&output.stderr).trim()))
    }
}

/// Whether the login window covers the session.
fn locked() -> bool {
    let Some(session) = CGSessionCopyCurrentDictionary() else { return false };
    // SAFETY: the session dictionary maps string keys to CF values.
    let session = unsafe { CFRetained::cast_unchecked::<CFDictionary<CFString, CFType>>(session) };
    session
        .get(&CFString::from_static_str("CGSSessionScreenIsLocked"))
        .is_some_and(|value| value.downcast_ref::<CFBoolean>().is_some_and(CFBoolean::as_bool))
}

fn idle() -> bool {
    // kCGAnyInputEventType.
    let any_input = CGEventType(u32::MAX);
    CGEventSource::seconds_since_last_event_type(CGEventSourceStateID::CombinedSessionState, any_input)
        >= IDLE_AFTER_SECONDS
}

/// The internal battery's (percent, charging or full), from `IOKit`'s power sources.
fn battery() -> Option<(u8, bool)> {
    let info = IOPSCopyPowerSourcesInfo()?;
    // SAFETY: `info` is the blob the list reads.
    let list = unsafe { IOPSCopyPowerSourcesList(Some(&info)) }?;
    // SAFETY: the list holds power source handles, which are CF types.
    let list = unsafe { CFRetained::cast_unchecked::<CFArray<CFType>>(list) };
    (0..list.len()).find_map(|index| {
        let source = list.get(index)?;
        // SAFETY: `source` comes from `info`'s list.
        let description = unsafe { IOPSGetPowerSourceDescription(Some(&info), Some(&source)) }?;
        // SAFETY: a power source's description maps string keys to CF values.
        let description = unsafe { CFRetained::cast_unchecked::<CFDictionary<CFString, CFType>>(description) };
        let value = |key: &'static str| description.get(&CFString::from_static_str(key));
        let flag =
            |key| value(key).is_some_and(|value| value.downcast_ref::<CFBoolean>().is_some_and(CFBoolean::as_bool));
        let number = |key| value(key)?.downcast_ref::<CFNumber>()?.as_i64();
        if value("Type")?.downcast_ref::<CFString>()?.to_string() != "InternalBattery" {
            return None;
        }
        let (current, max) = (number("Current Capacity")?, number("Max Capacity")?.max(1));
        let percent = u8::try_from((current * 100 / max).clamp(0, 100)).ok()?;
        Some((percent, flag("Is Charging") || flag("Is Charged")))
    })
}

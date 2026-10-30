//! The Mac's state for phones and the commands they send. Public APIs give the battery (`IOKit` power sources), sleep and
//! display sleep (`pmset`). macOS has no public API to lock the screen or to read or set brightness, Focus or the
//! power mode, so those stay out.

#![allow(unsafe_code, reason = "FFI to macOS frameworks")]
#![allow(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    clippy::unused_self,
    reason = "the Linux backend's interface, whose calls wait"
)]

use std::time::Duration;

use objc2_core_foundation::{CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_kit::{IOPSCopyPowerSourcesInfo, IOPSCopyPowerSourcesList, IOPSGetPowerSourceDescription};

/// `IOKit` sends power source changes only to a run loop, so the battery is read this often.
const BATTERY_EVERY: Duration = Duration::from_secs(10);

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
    battery: Option<(u8, bool)>,
    tick: tokio::time::Interval,
}

impl System {
    pub fn connect() -> Self {
        let mut tick = tokio::time::interval(BATTERY_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self { battery: battery(), tick }
    }

    /// What phones can ask of this desktop, as `SetSessionCapabilities` lists it.
    pub fn capabilities(&self) -> Vec<&'static str> {
        vec!["suspend", "screen-off"]
    }

    pub async fn read(&self) -> State {
        State { battery: self.battery, ..State::default() }
    }

    /// Waits for a change to the battery; pending forever on a Mac without one.
    pub async fn changed(&mut self) {
        if self.battery.is_none() {
            return std::future::pending().await;
        }
        loop {
            self.tick.tick().await;
            let now = battery();
            if now != self.battery {
                self.battery = now;
                return;
            }
        }
    }

    /// Carries out a phone's command other than volume and mute; the error is for the user.
    pub async fn run(&mut self, command: &str, _value: u32) -> Result<(), String> {
        match command {
            "suspend" => pmset("sleepnow", "Sleeping").await,
            "screen-off" => pmset("displaysleepnow", "Screen off").await,
            "lock" => Err("macOS lets no app lock the screen".to_owned()),
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

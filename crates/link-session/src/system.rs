//! The desktop's state for phones and the commands they send, each through the service that owns it: logind (lock,
//! suspend, brightness, locked, idle), `UPower` (battery), power-profiles-daemon (profile), and the Andromeda shell
//! (Do Not Disturb, screen off). A part whose service is missing is left out, and its commands say so.

use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use futures_util::stream::{SelectAll, Stream, StreamExt};

/// Profiles in `set-profile` value order.
const PROFILES: [&str; 3] = ["power-saver", "balanced", "performance"];
/// sysfs gives no change events for the backlight, so it is read this often.
const BACKLIGHT_EVERY: Duration = Duration::from_secs(1);

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1/session/auto"
)]
trait Session {
    fn lock(&self) -> zbus::Result<()>;
    fn set_brightness(&self, subsystem: &str, name: &str, brightness: u32) -> zbus::Result<()>;
    #[zbus(property)]
    fn locked_hint(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn idle_hint(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait Battery {
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    /// 1 charging, 4 fully charged.
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
trait Profiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.andromeda.Shell1",
    default_service = "org.andromeda.Shell1",
    default_path = "/org/andromeda/Shell1"
)]
trait Shell {
    #[zbus(property)]
    fn do_not_disturb(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_do_not_disturb(&self, on: bool) -> zbus::Result<()>;
    fn screen_off(&self) -> zbus::Result<()>;
}

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

struct Backlight {
    dir: PathBuf,
    name: String,
    max: u32,
}

type Changes = SelectAll<Pin<Box<dyn Stream<Item = ()> + Send>>>;

pub struct System {
    session: Option<SessionProxy<'static>>,
    manager: Option<ManagerProxy<'static>>,
    battery: Option<BatteryProxy<'static>>,
    profiles: Option<ProfilesProxy<'static>>,
    shell: Option<ShellProxy<'static>>,
    backlight: Option<Backlight>,
    changes: Changes,
    brightness: Option<u8>,
    tick: tokio::time::Interval,
}

impl System {
    /// Each part whose service answers now; the shell's on the session bus `bus`, the rest on the system bus.
    pub async fn connect(bus: &zbus::Connection) -> Self {
        let system = zbus::Connection::system().await.inspect_err(|error| log::info!("system bus: {error}")).ok();
        let mut changes = Changes::new();
        let (mut session, mut manager, mut battery, mut profiles) = (None, None, None, None);
        if let Some(system) = &system {
            // A proxy is kept only when its service answers, so a missing one counts as absent.
            session = match SessionProxy::new(system).await {
                Ok(proxy) if proxy.locked_hint().await.is_ok() => Some(proxy),
                _ => None,
            };
            manager = ManagerProxy::new(system).await.ok();
            battery = match BatteryProxy::new(system).await {
                Ok(proxy) if proxy.is_present().await.unwrap_or(false) => Some(proxy),
                _ => None,
            };
            profiles = match ProfilesProxy::new(system).await {
                Ok(proxy) if proxy.active_profile().await.is_ok() => Some(proxy),
                _ => None,
            };
        }
        let shell = match ShellProxy::new(bus).await {
            Ok(proxy) if proxy.do_not_disturb().await.is_ok() => Some(proxy),
            _ => None,
        };
        if let Some(proxy) = &session {
            changes.push(proxy.receive_locked_hint_changed().await.map(drop).boxed());
            changes.push(proxy.receive_idle_hint_changed().await.map(drop).boxed());
        }
        if let Some(proxy) = &battery {
            changes.push(proxy.receive_percentage_changed().await.map(drop).boxed());
            changes.push(proxy.receive_state_changed().await.map(drop).boxed());
        }
        if let Some(proxy) = &profiles {
            changes.push(proxy.receive_active_profile_changed().await.map(drop).boxed());
        }
        if let Some(proxy) = &shell {
            changes.push(proxy.receive_do_not_disturb_changed().await.map(drop).boxed());
        }
        let backlight = backlight();
        let brightness = backlight.as_ref().and_then(read_brightness);
        let mut tick = tokio::time::interval(BACKLIGHT_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self { session, manager, battery, profiles, shell, backlight, changes, brightness, tick }
    }

    /// What phones can ask of this desktop, as `SetSessionCapabilities` lists it.
    pub fn capabilities(&self) -> Vec<&'static str> {
        let has_session = self.session.is_some();
        [
            (self.backlight.is_some() && has_session, "brightness"),
            (has_session, "lock"),
            (self.manager.is_some(), "suspend"),
            (self.profiles.is_some(), "profile"),
            (self.shell.is_some(), "dnd"),
            (self.shell.is_some(), "screen-off"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect()
    }

    pub async fn read(&self) -> State {
        let battery = match &self.battery {
            Some(proxy) => match (proxy.percentage().await, proxy.state().await) {
                (Ok(percent), Ok(state)) => Some((percent_of(percent), matches!(state, 1 | 4))),
                _ => None,
            },
            None => None,
        };
        let (locked, idle) = match &self.session {
            Some(proxy) => (proxy.locked_hint().await.ok(), proxy.idle_hint().await.ok()),
            None => (None, None),
        };
        let profile = match &self.profiles {
            Some(proxy) => proxy.active_profile().await.ok().filter(|profile| PROFILES.contains(&profile.as_str())),
            None => None,
        };
        let dnd = match &self.shell {
            Some(proxy) => proxy.do_not_disturb().await.ok(),
            None => None,
        };
        State { battery, brightness: self.brightness, locked, idle, dnd, profile }
    }

    /// Waits for a change to any part; pending forever with none to watch.
    pub async fn changed(&mut self) {
        loop {
            tokio::select! {
                Some(()) = self.changes.next(), if !self.changes.is_empty() => return,
                _ = self.tick.tick(), if self.backlight.is_some() => {
                    let now = self.backlight.as_ref().and_then(read_brightness);
                    if now != self.brightness {
                        self.brightness = now;
                        return;
                    }
                }
                else => std::future::pending().await,
            }
        }
    }

    /// Carries out a phone's command other than volume and mute; the error is for the user.
    pub async fn run(&mut self, command: &str, value: u32) -> Result<(), String> {
        let failed = |what: &str, error: zbus::Error| format!("{what} failed: {error}");
        match command {
            "set-brightness" => {
                let (Some(session), Some(backlight)) = (&self.session, &self.backlight) else {
                    return Err("No backlight on this desktop".to_owned());
                };
                let raw = u32::try_from(u64::from(value.min(100)) * u64::from(backlight.max) / 100).unwrap_or(u32::MAX);
                session.set_brightness("backlight", &backlight.name, raw).await.map_err(|e| failed("Brightness", e))?;
                self.brightness = read_brightness(backlight);
                Ok(())
            }
            "lock" => self.session()?.lock().await.map_err(|e| failed("Locking", e)),
            "suspend" => {
                let manager = self.manager.as_ref().ok_or("This desktop cannot suspend")?;
                manager.suspend(false).await.map_err(|e| failed("Suspending", e))
            }
            "screen-off" => {
                self.shell("Turning the screen off")?.screen_off().await.map_err(|e| failed("Screen off", e))
            }
            "set-dnd" => self
                .shell("Do Not Disturb")?
                .set_do_not_disturb(value != 0)
                .await
                .map_err(|e| failed("Do Not Disturb", e)),
            "set-profile" => {
                let profiles = self.profiles.as_ref().ok_or("No power profiles on this desktop")?;
                let profile = PROFILES.get(value as usize).ok_or("Unknown power profile")?;
                profiles.set_active_profile(profile).await.map_err(|e| failed("Power profile", e))
            }
            other => Err(format!("{other} is not a desktop command")),
        }
    }

    fn session(&self) -> Result<&SessionProxy<'static>, String> {
        self.session.as_ref().ok_or_else(|| "This desktop has no logind session".to_owned())
    }

    fn shell(&self, what: &str) -> Result<&ShellProxy<'static>, String> {
        self.shell.as_ref().ok_or_else(|| format!("{what} needs the Andromeda shell"))
    }
}

/// The first backlight under `/sys/class/backlight`, or `ANDROMEDA_LINK_BACKLIGHT` in tests.
fn backlight() -> Option<Backlight> {
    let root =
        std::env::var_os("ANDROMEDA_LINK_BACKLIGHT").map_or_else(|| "/sys/class/backlight".into(), PathBuf::from);
    let dir = std::fs::read_dir(root).ok()?.filter_map(Result::ok).map(|entry| entry.path()).min()?;
    let name = dir.file_name()?.to_str()?.to_owned();
    let max = std::fs::read_to_string(dir.join("max_brightness")).ok()?.trim().parse().ok().filter(|max| *max > 0)?;
    Some(Backlight { dir, name, max })
}

#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "clamped to 0 to 100 first")]
fn percent_of(value: f64) -> u8 {
    value.clamp(0.0, 100.0).round() as u8
}

fn read_brightness(backlight: &Backlight) -> Option<u8> {
    let raw: u64 = std::fs::read_to_string(backlight.dir.join("actual_brightness"))
        .or_else(|_| std::fs::read_to_string(backlight.dir.join("brightness")))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    u8::try_from((raw * 100 + u64::from(backlight.max) / 2) / u64::from(backlight.max))
        .ok()
        .map(|percent| percent.min(100))
}

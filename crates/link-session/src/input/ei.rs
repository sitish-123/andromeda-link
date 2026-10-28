//! Input through libei, on the socket a `RemoteDesktop` portal session gives (GNOME, KDE).

use std::io::Read as _;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;

use anyhow::Context as _;
use futures_util::StreamExt;
use reis::event::{Device, DeviceCapability, EiEvent};
use reis::{ei, tokio::EiConvertEventStream};
use xkbcommon::xkb;

use super::{Action, keysym};

/// The EIS server's keymap, to find the key and level that type a keysym.
struct Keymap {
    keymap: xkb::Keymap,
    shift: Option<u32>,
}

impl Keymap {
    fn load(fd: &OwnedFd) -> anyhow::Result<Self> {
        let mut text = String::new();
        std::fs::File::from(fd.try_clone()?).read_to_string(&mut text)?;
        let text = text.trim_end_matches('\0').to_owned();
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap =
            xkb::Keymap::new_from_string(&context, text, xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS)
                .context("the EIS keymap does not compile")?;
        let mut found = Self { keymap, shift: None };
        found.shift = found.find(xkb::Keysym::Shift_L.raw()).map(|(code, _)| code);
        Ok(found)
    }

    /// The xkb keycode and level, 0 or 1, that type `keysym` in the first layout.
    fn find(&self, keysym: u32) -> Option<(u32, u32)> {
        let (min, max) = (self.keymap.min_keycode().raw(), self.keymap.max_keycode().raw());
        (0..=1).find_map(|level| {
            (min..=max)
                .find(|code| {
                    self.keymap
                        .key_get_syms_by_level(xkb::Keycode::new(*code), 0, level)
                        .iter()
                        .any(|sym| sym.raw() == keysym)
                })
                .map(|code| (code, level))
        })
    }
}

pub struct Ei {
    connection: reis::event::Connection,
    events: EiConvertEventStream,
    /// Devices the server resumed, so events may be sent on them.
    devices: Vec<Device>,
    keymap: Option<Keymap>,
    sequence: u32,
}

impl Ei {
    pub async fn connect(socket: OwnedFd) -> anyhow::Result<Self> {
        let context = ei::Context::new(UnixStream::from(socket))?;
        let (connection, events) =
            context.handshake_tokio("andromeda-link-session", ei::handshake::ContextType::Sender).await?;
        Ok(Self { connection, events, devices: Vec::new(), keymap: None, sequence: 0 })
    }

    /// Handles the server's next event: seats to bind, devices to emulate on.
    pub async fn pump(&mut self) -> anyhow::Result<()> {
        let event = self.events.next().await.context("the EIS server closed the connection")??;
        match event {
            EiEvent::SeatAdded(added) => {
                let wanted = DeviceCapability::Pointer
                    | DeviceCapability::Keyboard
                    | DeviceCapability::Button
                    | DeviceCapability::Scroll
                    | DeviceCapability::Text;
                added.seat.bind_capabilities(wanted);
            }
            EiEvent::DeviceAdded(added) => {
                if let Some(keymap) = added.device.keymap() {
                    match Keymap::load(&keymap.fd) {
                        Ok(keymap) => self.keymap = Some(keymap),
                        Err(error) => log::warn!("{error:#}"),
                    }
                }
            }
            EiEvent::DeviceResumed(resumed) => {
                resumed.device.device().start_emulating(resumed.serial, self.sequence);
                self.sequence = self.sequence.wrapping_add(1);
                self.devices.push(resumed.device);
            }
            EiEvent::DevicePaused(paused) => self.devices.retain(|device| *device != paused.device),
            EiEvent::DeviceRemoved(removed) => self.devices.retain(|device| *device != removed.device),
            EiEvent::Disconnected(disconnected) => {
                anyhow::bail!("the EIS server disconnected: {:?}", disconnected.reason)
            }
            _ => {}
        }
        self.connection.flush()?;
        Ok(())
    }

    pub fn run(&mut self, action: Action) -> anyhow::Result<()> {
        match action {
            Action::Move(dx, dy) => {
                #[expect(clippy::cast_possible_truncation, reason = "libei takes f32 pixels")]
                self.on::<ei::Pointer>(|pointer| pointer.motion_relative(dx as f32, dy as f32));
            }
            Action::Scroll(dx, dy) => {
                #[expect(clippy::cast_possible_truncation, reason = "libei takes f32 pixels")]
                self.on::<ei::Scroll>(|scroll| scroll.scroll(dx as f32, dy as f32));
            }
            Action::Button { code, pressed } => self.button(code, pressed),
            Action::Click(code) => {
                self.button(code, true);
                self.button(code, false);
            }
            Action::Key(keysym) => self.key(keysym),
            // ei_text types any character; keys type only what the server's keymap has.
            Action::Text(text) if self.has::<ei::Text>() => self.on::<ei::Text>(|sender| sender.utf8(&text)),
            Action::Text(text) => {
                for keysym in text.chars().filter_map(keysym) {
                    self.key(keysym);
                }
            }
        }
        self.connection.flush()?;
        Ok(())
    }

    fn button(&self, code: u32, pressed: bool) {
        let state = if pressed { ei::button::ButtonState::Press } else { ei::button::ButtonState::Released };
        self.on::<ei::Button>(|button| button.button(code, state));
    }

    fn key(&self, keysym: u32) {
        let Some((code, level)) = self.keymap.as_ref().and_then(|keymap| keymap.find(keysym)) else {
            log::warn!("keysym 0x{keysym:x} is not in the session's keymap, and its EIS server has no ei_text");
            return;
        };
        let shift = self.keymap.as_ref().and_then(|keymap| keymap.shift).filter(|_| level == 1);
        // libei's key codes are evdev codes, xkb's minus 8.
        self.on::<ei::Keyboard>(|keyboard| {
            if let Some(shift) = shift {
                keyboard.key(shift - 8, ei::keyboard::KeyState::Press);
            }
            keyboard.key(code - 8, ei::keyboard::KeyState::Press);
            keyboard.key(code - 8, ei::keyboard::KeyState::Released);
            if let Some(shift) = shift {
                keyboard.key(shift - 8, ei::keyboard::KeyState::Released);
            }
        });
    }

    fn has<T: ei::Interface>(&self) -> bool {
        self.devices.iter().any(|device| device.interface::<T>().is_some())
    }

    /// Sends `emit` on the first resumed device with interface `T`, then ends the frame.
    fn on<T: ei::Interface>(&self, emit: impl FnOnce(&T)) {
        let Some((device, interface)) =
            self.devices.iter().find_map(|device| device.interface::<T>().map(|interface| (device, interface)))
        else {
            log::debug!("no resumed EIS device has {}", T::NAME);
            return;
        };
        emit(&interface);
        device.device().frame(self.connection.serial(), monotonic_micros());
    }
}

/// `CLOCK_MONOTONIC` in microseconds, the clock libei's frame timestamps use.
fn monotonic_micros() -> u64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    u64::try_from(now.tv_sec).unwrap_or(0) * 1_000_000 + u64::try_from(now.tv_nsec).unwrap_or(0) / 1000
}

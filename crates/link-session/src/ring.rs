//! "Ring desktop" from a phone: a sound on repeat until the phone stops it or [`RING_FOR`] passes. The Andromeda
//! shell rings on its own desktop, so this runs only elsewhere.

use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::link::Link;

/// Players to try, in order, with the sound each plays.
#[cfg(target_os = "macos")]
const PLAYERS: [(&str, &str); 1] = [("/usr/bin/afplay", "/System/Library/Sounds/Sosumi.aiff")];
#[cfg(target_os = "linux")]
const PLAYERS: [(&str, &str); 2] = [
    ("pw-play", "/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga"),
    ("paplay", "/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga"),
];
const RING_FOR: Duration = Duration::from_secs(30);

/// False on the Andromeda desktop, whose shell rings.
#[cfg(target_os = "linux")]
pub fn wanted() -> bool {
    !std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("andromeda"))
}

#[derive(Default)]
pub struct Ring {
    /// The phone that asked, the sound loop, and when it stops on its own.
    now: Option<(String, JoinHandle<()>, Instant)>,
}

impl Ring {
    pub async fn set(&mut self, link: &Link, device: &str, on: bool) {
        self.stop(link).await;
        if !on {
            return;
        }
        let sound = tokio::spawn(async {
            loop {
                // Dropping the child, when the task is aborted, ends the sound at once.
                let child = PLAYERS.iter().find_map(|(player, sound)| {
                    tokio::process::Command::new(player).arg(sound).kill_on_drop(true).spawn().ok()
                });
                match child {
                    // A player that fails would fail again at once.
                    Some(mut child) => match child.wait().await {
                        Ok(status) if status.success() => {}
                        result => return log::warn!("ringing: the player ended with {result:?}"),
                    },
                    None => return log::warn!("ringing: none of {PLAYERS:?} runs"),
                }
            }
        });
        self.now = Some((device.to_owned(), sound, Instant::now() + RING_FOR));
        report(link, device, true).await;
    }

    pub async fn stop(&mut self, link: &Link) {
        if let Some((device, sound, _)) = self.now.take() {
            sound.abort();
            report(link, &device, false).await;
        }
    }

    /// Waits until the ring times out; pending while nothing rings.
    pub async fn timed_out(&self) {
        match &self.now {
            Some((_, _, until)) => tokio::time::sleep_until(*until).await,
            None => std::future::pending().await,
        }
    }
}

async fn report(link: &Link, device: &str, on: bool) {
    if let Err(error) = link.proxy.desktop_ringing(device, on).await {
        log::warn!("reporting the ring to {device}: {error}");
    }
}

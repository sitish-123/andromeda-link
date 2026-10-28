//! The desktop's clipboard: its new selections go to phones, and a phone's offer becomes the selection.

pub mod portal;
pub mod wayland;
pub mod x11;

use std::os::fd::OwnedFd;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

/// Advertised with a selection served for a phone, so a read of it is never sent back. The Andromeda shell's clipboard
/// history knows it too.
pub const REMOTE_MIME: &str = "application/x-andromeda-link-remote";
/// The protocol's limits on a clipboard offer.
pub const MAX_BYTES: u64 = 64 << 20;
const MAX_TYPES: usize = 16;
const TEXT_MIMES: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];
const IMAGE_MIMES: [&str; 3] = ["image/png", "image/jpeg", "image/jxl"];
const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";
/// A selection's owner has this long to write it.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub enum Event {
    /// A new selection; `mimes[0]` is the type of `data`.
    Local { mimes: Vec<String>, data: Vec<u8> },
    /// A paste of the phone's offer wants `mime` written into `sink`; `done` hears whether it was.
    Paste { mime: String, sink: OwnedFd, done: Option<oneshot::Sender<bool>> },
}

pub enum Backend {
    Wayland(Box<wayland::Wayland>),
    Portal(Box<portal::Portal>),
    X11(x11::X11),
}

impl Backend {
    /// Data-control or the X11 selection, whichever the session offers without asking the user.
    pub fn native() -> Option<Self> {
        crate::attempt("data-control clipboard", wayland::Wayland::connect())
            .map(|backend| Self::Wayland(Box::new(backend)))
            .or_else(|| crate::attempt("X11 clipboard", x11::X11::connect()).map(Self::X11))
    }

    pub async fn portal(session: &crate::portal::Session) -> Option<Self> {
        crate::attempt("Clipboard portal", portal::Portal::connect(session).await)
            .map(|backend| Self::Portal(Box::new(backend)))
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Wayland(backend) => backend.name(),
            Self::Portal(_) => "the Clipboard portal",
            Self::X11(_) => "the X11 CLIPBOARD selection",
        }
    }

    /// Makes a phone's offer the selection; pastes of it come back as `Event::Paste`.
    pub async fn offer_remote(&mut self, mimes: &[String]) -> anyhow::Result<()> {
        match self {
            Self::Wayland(backend) => backend.offer_remote(mimes),
            Self::Portal(backend) => backend.offer_remote(mimes).await,
            Self::X11(backend) => backend.offer_remote(mimes),
        }
    }

    pub async fn next(&mut self) -> anyhow::Result<Event> {
        match self {
            Self::Wayland(backend) => backend.next().await,
            Self::Portal(backend) => backend.next().await,
            Self::X11(backend) => backend.next().await,
        }
    }
}

/// Reads a selection's pipe to its end: empty if over `MAX_BYTES`, an error if its writer is too slow.
async fn read_fd(fd: OwnedFd) -> anyhow::Result<Vec<u8>> {
    let read = async {
        let reader = tokio::net::unix::pipe::Receiver::from_owned_fd(fd)?;
        let mut data = Vec::new();
        reader.take(MAX_BYTES + 1).read_to_end(&mut data).await?;
        anyhow::Ok(data)
    };
    let mut data =
        tokio::time::timeout(READ_TIMEOUT, read).await.map_err(|_| anyhow::anyhow!("its owner was too slow"))??;
    if data.len() as u64 > MAX_BYTES {
        data.clear();
    }
    Ok(data)
}

/// The type to read from a selection, or `None` for one that must not leave the desktop.
pub fn choose(mimes: &[String]) -> Option<&str> {
    if mimes.iter().any(|mime| mime == PASSWORD_HINT || mime == REMOTE_MIME) {
        return None;
    }
    TEXT_MIMES
        .iter()
        .chain(&IMAGE_MIMES)
        .find(|wanted| mimes.iter().any(|mime| mime == *wanted))
        .copied()
        .or_else(|| mimes.iter().find(|mime| mime.contains('/')).map(String::as_str))
}

/// The types offered to phones: `chosen` first, then the rest the daemon may offer as alternatives.
pub fn offered(mimes: &[String], chosen: &str) -> Vec<String> {
    std::iter::once(chosen.to_owned())
        .chain(mimes.iter().filter(|mime| *mime != chosen && mime.contains('/')).cloned())
        .take(MAX_TYPES)
        .collect()
}

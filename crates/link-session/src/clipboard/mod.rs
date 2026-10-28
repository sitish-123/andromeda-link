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
const HTML_MIME: &str = "text/html";
const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";
/// A selection's owner has this long to write it.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub enum Event {
    /// A new selection: each read type's bytes with the names it is offered under, the preferred one first.
    Local { types: Vec<(Vec<String>, Vec<u8>)> },
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

/// The names `chosen`'s bytes are offered under: itself, and for text its other text types.
pub fn aliases(mimes: &[String], chosen: &str) -> Vec<String> {
    let text = TEXT_MIMES.contains(&chosen);
    std::iter::once(chosen.to_owned())
        .chain(mimes.iter().filter(|mime| text && *mime != chosen && TEXT_MIMES.contains(&mime.as_str())).cloned())
        .filter(|mime| mime.contains('/'))
        .take(MAX_TYPES)
        .collect()
}

/// The types to read from a selection, each with the names it is offered under: the chosen type first, then
/// `text/html` and an image. Empty for a selection that must not leave the desktop.
pub fn plan(mimes: &[String]) -> Vec<Vec<String>> {
    let Some(chosen) = choose(mimes) else { return Vec::new() };
    let has = |wanted: &str| mimes.iter().any(|mime| mime == wanted);
    let html = has(HTML_MIME).then_some(HTML_MIME);
    let image = IMAGE_MIMES.iter().copied().find(|image| has(image));
    std::iter::once(chosen)
        .chain([html, image].into_iter().flatten().filter(|mime| *mime != chosen))
        .map(|mime| aliases(mimes, mime))
        .filter(|names| !names.is_empty())
        .collect()
}

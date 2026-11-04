//! A phone's image clip pulled as soon as it is offered, so a paste reads bytes already here. A JPEG, which is how
//! phones save screenshots, also pastes as PNG, which many desktop apps ask for; the PNG is made here, not on the phone.

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::os::fd::OwnedFd;
use std::sync::Arc;

use tokio::sync::watch;

use crate::link::Link;

const PNG: &str = "image/png";
const JPEG: &str = "image/jpeg";
/// A larger image is pulled per paste, as other types are.
const MAX_PREFETCH: u64 = 32 << 20;

/// The types to put on the selection for a phone's `mimes`: a JPEG without a PNG also offers PNG.
pub fn offered(mimes: &[String]) -> Vec<String> {
    let mut offered = mimes.to_vec();
    if mimes.iter().any(|mime| mime == JPEG) && !mimes.iter().any(|mime| mime == PNG) {
        offered.push(PNG.to_owned());
    }
    offered
}

/// The pulled image and, for a JPEG, its PNG.
struct Pulled {
    mime: String,
    bytes: Vec<u8>,
    png: Option<Vec<u8>>,
}

#[derive(Clone)]
enum State {
    Pulling,
    Failed,
    Ready(Arc<Pulled>),
}

#[derive(Clone)]
pub struct Prefetch {
    device: String,
    id: u64,
    /// The type pulled.
    mime: String,
    pulled: watch::Receiver<State>,
}

impl Prefetch {
    /// Starts pulling the first image type of `device`'s offer `id`; `None` when it has none or is too large.
    pub fn start(link: &Link, device: &str, id: u64, mimes: &[String], size: u64) -> Option<Self> {
        let mime = mimes.iter().find(|mime| mime.starts_with("image/"))?.clone();
        if size > MAX_PREFETCH {
            return None;
        }
        let (send, pulled) = watch::channel(State::Pulling);
        let (link, owner, pulling) = (link.clone(), device.to_owned(), mime.clone());
        tokio::spawn(async move {
            let result = pull(&link, &owner, id, pulling).await;
            if let Err(error) = &result {
                log::warn!("prefetching {owner}'s image clip: {error:#}");
            }
            send.send_replace(result.map_or(State::Failed, |image| State::Ready(Arc::new(image))));
        });
        Some(Self { device: device.to_owned(), id, mime, pulled })
    }

    /// Serves a paste of `mime` of the selection's phone clip `remote` from the pulled image, with whether it was
    /// written; the sink comes back when this prefetch does not hold that clip and type, for a pull from the phone.
    pub async fn serve(&self, remote: &(String, u64), mime: &str, sink: OwnedFd) -> Result<bool, OwnedFd> {
        if remote.0 != self.device || remote.1 != self.id {
            return Err(sink);
        }
        let wanted = mime.starts_with("image/") && (mime == PNG || self.mime == mime);
        if !wanted {
            return Err(sink);
        }
        let mut pulled = self.pulled.clone();
        let Ok(State::Ready(image)) =
            pulled.wait_for(|state| !matches!(state, State::Pulling)).await.map(|state| state.clone())
        else {
            return Err(sink);
        };
        let png = mime != image.mime;
        // A JPEG's PNG exists only here; the phone has none to pull.
        if png && image.png.is_none() {
            return Ok(false);
        }
        let written = tokio::task::spawn_blocking(move || {
            let data = if png { image.png.as_deref().unwrap_or_default() } else { &image.bytes };
            // The pasting app may hand a non-blocking pipe; a blocking write keeps a large image whole.
            let flags = rustix::fs::fcntl_getfl(&sink)? - rustix::fs::OFlags::NONBLOCK;
            rustix::fs::fcntl_setfl(&sink, flags)?;
            std::fs::File::from(sink).write_all(data)
        })
        .await;
        Ok(matches!(written, Ok(Ok(()))))
    }
}

async fn pull(link: &Link, device: &str, id: u64, mime: String) -> anyhow::Result<Pulled> {
    let mut file = crate::link::anonymous_file()?;
    if !link.pull_clipboard(device, id, &mime, file.try_clone()?.into()).await {
        anyhow::bail!("the pull failed");
    }
    let bytes = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
    .await??;
    if mime != JPEG {
        return Ok(Pulled { mime, bytes, png: None });
    }
    tokio::task::spawn_blocking(move || {
        let png = png_of_jpeg(&bytes).inspect_err(|error| log::warn!("PNG of a JPEG clip: {error:#}")).ok();
        Pulled { mime, bytes, png }
    })
    .await
    .map_err(Into::into)
}

/// Fast compression: a paste waits on it, and the PNG lives only on the clipboard.
fn png_of_jpeg(jpeg: &[u8]) -> anyhow::Result<Vec<u8>> {
    use image::ImageEncoder as _;
    let image = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)?.into_rgb8();
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgb8)?;
    Ok(png)
}

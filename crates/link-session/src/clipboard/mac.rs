//! The macOS general pasteboard. It sends no change events, so its change count is polled. A phone's offer is pulled
//! whole and then written, because a lazy pasteboard owner needs the main run loop, which this process does not run.

#![allow(unsafe_code, reason = "FFI to macOS frameworks")]

use std::collections::VecDeque;
use std::os::fd::OwnedFd;
use std::time::Duration;

use objc2::rc::Retained;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard};
use objc2_foundation::{NSData, NSDictionary, NSString};
use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot};

use super::{Event, MAX_BYTES, PASSWORD_HINT, REMOTE_MIME};

const POLL_EVERY: Duration = Duration::from_millis(500);
/// Marks a pasteboard written for a phone, so it is not sent back.
const REMOTE_TYPE: &str = "org.andromeda.link.remote";
const TEXT_TYPE: &str = "public.utf8-plain-text";
const PNG_MIME: &str = "image/png";
/// Read only when there is no PNG, and sent as one.
const TIFF_TYPE: &str = "public.tiff";
/// Pasteboard types and the MIME types they are offered as; the nspasteboard.org markers keep passwords and
/// transient data on this Mac.
const TYPES: [(&str, &str); 8] = [
    (TEXT_TYPE, "text/plain;charset=utf-8"),
    (TEXT_TYPE, "text/plain"),
    ("public.html", "text/html"),
    ("public.png", PNG_MIME),
    ("public.jpeg", "image/jpeg"),
    ("org.nspasteboard.ConcealedType", PASSWORD_HINT),
    ("org.nspasteboard.TransientType", PASSWORD_HINT),
    (REMOTE_TYPE, REMOTE_MIME),
];

/// An offer's pulled types as (pasteboard type, bytes), tagged with the offer's number.
type Pulled = (u64, Vec<(&'static str, Vec<u8>)>);

pub struct Mac {
    pasteboard: Retained<NSPasteboard>,
    /// The change count last read or written here.
    seen: isize,
    tick: tokio::time::Interval,
    /// Pulls of the newest offer, for the loop to serve.
    pastes: VecDeque<Event>,
    /// The newest offer's number; a slower, older pull is dropped.
    offer: u64,
    pulled_tx: mpsc::UnboundedSender<Pulled>,
    pulled_rx: mpsc::UnboundedReceiver<Pulled>,
}

impl Mac {
    pub fn connect() -> Self {
        let pasteboard = NSPasteboard::generalPasteboard();
        let seen = pasteboard.changeCount();
        let mut tick = tokio::time::interval(POLL_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let (pulled_tx, pulled_rx) = mpsc::unbounded_channel();
        Self { pasteboard, seen, tick, pastes: VecDeque::new(), offer: 0, pulled_tx, pulled_rx }
    }

    /// Pulls each type of the phone's offer that the pasteboard takes; the pasteboard gets them all at once.
    pub fn offer_remote(&mut self, mimes: &[String]) {
        self.offer += 1;
        self.pastes.clear();
        let mut reads = Vec::new();
        for names in super::plan(mimes) {
            let Some(kind) = pasteboard_type(&names[0]) else { continue };
            let (reader, writer) = match std::io::pipe() {
                Ok(pipe) => pipe,
                Err(error) => {
                    log::warn!("a pipe for the phone's clip: {error}");
                    return;
                }
            };
            let (done, pulled) = oneshot::channel();
            self.pastes.push_back(Event::Paste { mime: names[0].clone(), sink: writer.into(), done: Some(done) });
            reads.push(async move {
                let (data, pulled) = tokio::join!(read_pull(reader.into()), pulled);
                match (data, pulled) {
                    (Ok(data), Ok(true)) if !data.is_empty() => Some((kind, data)),
                    (Err(error), _) => {
                        log::warn!("reading the phone's {kind}: {error}");
                        None
                    }
                    _ => None,
                }
            });
        }
        let (offer, sender) = (self.offer, self.pulled_tx.clone());
        let reads: Vec<_> = reads.into_iter().map(tokio::spawn).collect();
        tokio::spawn(async move {
            let mut types = Vec::new();
            for read in reads {
                types.extend(read.await.ok().flatten());
            }
            drop(sender.send((offer, types)));
        });
    }

    pub async fn next(&mut self) -> anyhow::Result<Event> {
        loop {
            if let Some(event) = self.pastes.pop_front() {
                return Ok(event);
            }
            tokio::select! {
                _ = self.tick.tick() => {
                    let count = self.pasteboard.changeCount();
                    if count != self.seen {
                        self.seen = count;
                        let types = self.read();
                        if !types.is_empty() {
                            return Ok(Event::Local { types });
                        }
                    }
                }
                Some((offer, types)) = self.pulled_rx.recv() => {
                    if offer == self.offer && !types.is_empty() {
                        self.write(&types);
                    }
                }
            }
        }
    }

    fn write(&mut self, types: &[(&str, Vec<u8>)]) {
        self.pasteboard.clearContents();
        for (kind, data) in types {
            self.pasteboard.setData_forType(Some(&NSData::with_bytes(data)), &NSString::from_str(kind));
        }
        self.pasteboard.setData_forType(Some(&NSData::new()), &NSString::from_str(REMOTE_TYPE));
        self.seen = self.pasteboard.changeCount();
    }

    /// Each type to send with the names it is offered under, as [`super::plan`] picks them.
    fn read(&self) -> Vec<(Vec<String>, Vec<u8>)> {
        let Some(kinds) = self.pasteboard.types() else { return Vec::new() };
        let kinds: Vec<String> = kinds.to_vec().iter().map(ToString::to_string).collect();
        let has = |wanted: &str| kinds.iter().any(|kind| kind == wanted);
        let mut mimes: Vec<String> =
            TYPES.iter().filter(|(kind, _)| has(kind)).map(|(_, mime)| (*mime).to_owned()).collect();
        if !mimes.iter().any(|mime| mime == PNG_MIME) && has(TIFF_TYPE) {
            mimes.push(PNG_MIME.to_owned());
        }
        super::plan(&mimes)
            .into_iter()
            .filter_map(|names| {
                let data = self.data(&names[0])?;
                Some((names, data))
            })
            .collect()
    }

    /// `mime`'s bytes, or `None` when missing, empty or over [`MAX_BYTES`].
    fn data(&self, mime: &str) -> Option<Vec<u8>> {
        let kind = pasteboard_type(mime)?;
        let data = self.pasteboard.dataForType(&NSString::from_str(kind));
        let data = match data {
            Some(data) => data,
            None if mime == PNG_MIME => self.png_from_tiff()?,
            None => return None,
        };
        let bytes = data.to_vec();
        (!bytes.is_empty() && bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
    }

    fn png_from_tiff(&self) -> Option<Retained<NSData>> {
        let tiff = self.pasteboard.dataForType(&NSString::from_str(TIFF_TYPE))?;
        let image = NSBitmapImageRep::imageRepWithData(&tiff)?;
        // SAFETY: an empty dictionary holds no property of the wrong type.
        unsafe { image.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
    }
}

/// The pasteboard type that holds `mime`, for the types the pasteboard takes from a phone.
fn pasteboard_type(mime: &str) -> Option<&'static str> {
    TYPES.iter().take(5).find(|(_, offered)| *offered == mime).map(|(kind, _)| *kind)
}

/// Reads a phone's pull to its end, which comes when the daemon is done with the pipe; empty if over [`MAX_BYTES`].
async fn read_pull(fd: OwnedFd) -> anyhow::Result<Vec<u8>> {
    let reader = tokio::net::unix::pipe::Receiver::from_owned_fd(fd)?;
    let mut data = Vec::new();
    reader.take(MAX_BYTES + 1).read_to_end(&mut data).await?;
    if data.len() as u64 > MAX_BYTES {
        data.clear();
    }
    Ok(data)
}

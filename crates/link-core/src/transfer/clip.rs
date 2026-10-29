//! Clipboard offers and pulls (`ARCHITECTURE.md`, Clipboard). The transfer actor serves them, since it owns the
//! bulk streams a pull is answered on.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as _;
use std::os::unix::fs::{FileExt, FileTypeExt};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use link_proto::clip::{ClipState, Serve};
use link_proto::message::{ClipOffer, ClipPull, MAX_CLIP_SIZE, MAX_SHARE_LEN, Message, is_text_mime};
use link_proto::transfer::Budget;
use ring::rand::{SecureRandom, SystemRandom};
use tokio::io::AsyncWriteExt;
use tokio::net::unix::pipe;
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::{CANCELLED, CHUNK, Outbound, PROTOCOL_ERROR, TaskEnd, TransferActor, TransferEvent};
use crate::Error;
use crate::control::{STEP_TIMEOUT, write_frame};
use crate::identity::DeviceId;
use crate::session::SessionHandle;
use crate::wire::RecvStream;

/// What this side offers: each type with its own bytes, in order of preference. The first `text/plain` type's text
/// goes inline when short enough.
pub struct LocalClip {
    pub types: Vec<(String, ClipBytes)>,
    /// The origin and copy time (ms since the Unix epoch) of a clip kept across a restart; `None` for a new copy.
    pub copied: Option<(u64, u64)>,
}

pub enum ClipBytes {
    Text(String),
    File(File),
}

/// A peer that just connected: whether it holds the clipboard grant, so it gets the kept copy, and whether its hello
/// carries [`link_proto::message::CLIP_ITEMS`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ClipPeer {
    pub granted: bool,
    pub items: bool,
}

/// A peer's offer older than this side's clip is dropped this long after it connects: both sides offer their kept
/// clip on connect, and the newer one wins. Later offers are new copies, so clock skew never drops them.
const CONNECT_WINDOW: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct Clips {
    peers: HashMap<DeviceId, ClipState>,
    /// Whether each peer takes [`link_proto::message::CLIP_ITEMS`], and when it connected.
    connected: HashMap<DeviceId, (bool, Instant)>,
    /// This side's latest copy, kept while no peer is connected and offered on connect.
    local: Option<Local>,
    pulls: Vec<Pull>,
}

struct Local {
    types: Vec<(String, Content)>,
    inline: Option<String>,
    origin: u64,
    at: u64,
}

#[derive(Clone)]
enum Content {
    Text(Arc<str>),
    File(Arc<File>, u64),
}

struct Pull {
    peer: DeviceId,
    header: ClipPull,
    sink: Option<File>,
    reply: oneshot::Sender<Result<u64, Error>>,
    deadline: Instant,
}

impl Clips {
    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.pulls.iter().filter(|pull| pull.sink.is_some()).map(|pull| pull.deadline).min()
    }
}

impl LocalClip {
    fn local(self) -> Result<Local, Error> {
        if self.types.is_empty() {
            return Err(Error::Unexpected("a clip without content"));
        }
        let mut types = Vec::with_capacity(self.types.len());
        for (mime, bytes) in self.types {
            let content = match bytes {
                ClipBytes::Text(text) => Content::Text(text.into()),
                ClipBytes::File(file) => {
                    let size = file.metadata()?.len();
                    Content::File(Arc::new(file), size)
                }
            };
            if content.size() > MAX_CLIP_SIZE {
                return Err(Error::Unexpected("a clip over 64 MiB"));
            }
            types.push((mime, content));
        }
        let inline = types.iter().find(|(mime, _)| is_text_mime(mime)).and_then(|(_, content)| content.text());
        let (origin, at) = match self.copied {
            Some(copied) => copied,
            None => (random_origin()?, now_ms()),
        };
        Ok(Local { types, inline, origin, at })
    }
}

impl Local {
    fn offer(&self, items: bool) -> ClipOffer {
        let offer = ClipOffer {
            id: 0,
            mimes: self.types.iter().map(|(mime, _)| mime.clone()).collect(),
            size: self.types.first().map_or(0, |(_, content)| content.size()),
            text: self.inline.clone(),
            sizes: Some(self.types.iter().map(|(_, content)| content.size()).collect()),
            origin: Some(self.origin),
            at: Some(self.at),
        };
        if items { offer } else { offer.without_items() }
    }

    fn content(&self, mime: &str) -> Option<Content> {
        self.types.iter().find(|(offered, _)| offered == mime).map(|(_, content)| content.clone())
    }
}

impl Content {
    fn size(&self) -> u64 {
        match self {
            Self::Text(text) => text.len() as u64,
            Self::File(_, size) => *size,
        }
    }

    /// Text up to the inline limit; a file is read only when pulled.
    fn text(&self) -> Option<String> {
        match self {
            Self::Text(text) if text.len() <= MAX_SHARE_LEN && !text.is_empty() => Some(text.to_string()),
            _ => None,
        }
    }
}

fn random_origin() -> Result<u64, Error> {
    let mut bytes = [0; 8];
    SystemRandom::new().fill(&mut bytes).map_err(|_| Error::Unexpected("no randomness for a clip origin"))?;
    Ok(u64::from_be_bytes(bytes))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0))
}

impl TransferActor {
    /// Keeps `clip` as this side's latest copy and offers it to `peers`; with none connected, it waits for one.
    pub(super) async fn offer_clip(&mut self, peers: &[DeviceId], clip: LocalClip) -> Result<(), Error> {
        let mut local = clip.local()?;
        // A clipboard manager that takes a peer's clip offers it again: it keeps that clip's origin, so the peer drops it.
        if let Some(origin) = self
            .clips
            .peers
            .values()
            .filter_map(ClipState::received)
            .find(|offer| offer.text.is_some() && offer.text == local.inline)
            .and_then(|offer| offer.origin)
        {
            local.origin = origin;
        }
        if !local.offer(true).is_valid() {
            return Err(Error::Unexpected("a clip offer that breaks the rules"));
        }
        self.clips.local = Some(local);
        for peer in peers {
            self.send_clip(peer).await;
        }
        Ok(())
    }

    /// Offers the kept copy to a peer that just connected, unless that peer's last offer is newer.
    pub(super) async fn offer_kept_clip(&mut self, peer: &DeviceId, clip: ClipPeer) {
        self.clips.connected.insert(peer.clone(), (clip.items, Instant::now()));
        let theirs = self.clips.peers.get(peer).and_then(ClipState::received).and_then(|offer| offer.at).unwrap_or(0);
        if clip.granted && self.clips.local.as_ref().is_some_and(|local| local.at > theirs) {
            self.send_clip(peer).await;
        }
    }

    async fn send_clip(&mut self, peer: &DeviceId) {
        let items = self.clips.connected.get(peer).is_some_and(|(items, _)| *items);
        let Some(local) = &self.clips.local else { return };
        let offer = self.clips.peers.entry(peer.clone()).or_default().offer(local.offer(items));
        self.send_to(peer, Message::ClipOffer(offer)).await;
    }

    pub(super) fn on_clip_offer(&mut self, peer: &DeviceId, offer: ClipOffer) {
        if let Some(local) = &self.clips.local {
            if offer.origin == Some(local.origin) {
                log::debug!("{peer}: the offer is this side's own clip; dropping it");
                return;
            }
            let connected = self.clips.connected.get(peer).map(|(_, since)| since.elapsed());
            if connected.is_some_and(|since| since < CONNECT_WINDOW) && offer.at.is_some_and(|at| at < local.at) {
                log::info!("{peer}: the kept clip it offered is older than this side's; dropping it");
                return;
            }
        }
        let event = TransferEvent::ClipOffered {
            from: peer.clone(),
            id: offer.id,
            mimes: offer.mimes.clone(),
            size: offer.size,
            text: offer.text.clone(),
        };
        self.clips.peers.entry(peer.clone()).or_default().on_offer(offer);
        self.emit(event);
    }

    /// Answers a pull on a new stream; a stale one gets a stream that is reset at once.
    pub(super) fn on_clip_pull(&mut self, peer: &DeviceId, pull: &ClipPull) {
        let Some(session) = self.sessions.get(peer).filter(|session| session.is_live()).cloned() else { return };
        let served =
            self.clips.peers.get(peer).map_or(Err(link_proto::clip::ClipRefused::Stale), |state| state.on_pull(pull));
        let content = match served {
            Ok(Serve::Text(text)) => Some(Content::Text(text.into())),
            Ok(Serve::Stream { .. }) => self.clips.local.as_ref().and_then(|local| local.content(&pull.mime)),
            Err(refused) => {
                log::info!("{peer}: clipboard pull refused: {refused}");
                None
            }
        };
        let (peer, header) = (peer.clone(), pull.clone());
        self.tasks.spawn(async move {
            let result = serve(&session, header, content).await;
            TaskEnd::ClipServed { peer, result }
        });
    }

    pub(super) async fn pull_clip(
        &mut self,
        peer: DeviceId,
        id: u64,
        mime: String,
        sink: File,
        reply: oneshot::Sender<Result<u64, Error>>,
    ) {
        let started = self
            .clips
            .peers
            .get_mut(&peer)
            .map_or(Err(link_proto::clip::ClipRefused::Stale), |state| state.pull(id, &mime));
        match started {
            Ok(Serve::Text(text)) => {
                self.tasks.spawn(async move {
                    let written = write_sink(sink, text.as_bytes()).await.map(|()| text.len() as u64);
                    drop(reply.send(written));
                    TaskEnd::ClipServed { peer, result: Ok(()) }
                });
            }
            Ok(Serve::Stream { .. }) => {
                let header = ClipPull { id, mime };
                if !self.send_to(&peer, Message::ClipPull(header.clone())).await {
                    drop(reply.send(Err(Error::NotConnected)));
                    return;
                }
                let deadline = Instant::now() + STEP_TIMEOUT;
                self.clips.pulls.push(Pull { peer, header, sink: Some(sink), reply, deadline });
            }
            Err(refused) => drop(reply.send(Err(Error::ClipRefused(refused)))),
        }
    }

    pub(super) fn on_clip_data(&mut self, peer: &DeviceId, mut recv: RecvStream, header: ClipPull) {
        let pending = self.clips.pulls.iter_mut().find(|pull| pull.peer == *peer && pull.header == header);
        let admitted = self.clips.peers.get_mut(peer).map(|state| state.on_data(&header));
        let (Some(pull), Some(Ok(budget))) = (pending, admitted) else {
            log::warn!("{peer}: a clip-data stream nobody pulled");
            recv.stop(PROTOCOL_ERROR);
            return;
        };
        let Some(sink) = pull.sink.take() else { return };
        let peer = peer.clone();
        self.tasks.spawn(async move {
            let result = copy(recv, sink, budget).await;
            TaskEnd::ClipPulled { peer, header, result }
        });
    }

    pub(super) fn clip_pulled(&mut self, peer: &DeviceId, header: &ClipPull, result: Result<u64, Error>) {
        let Some(index) = self.clips.pulls.iter().position(|pull| pull.peer == *peer && pull.header == *header) else {
            return;
        };
        let pull = self.clips.pulls.remove(index);
        drop(pull.reply.send(result));
    }

    pub(super) fn clip_deadlines(&mut self) {
        let now = Instant::now();
        let (expired, waiting): (Vec<Pull>, Vec<Pull>) =
            self.clips.pulls.drain(..).partition(|pull| pull.sink.is_some() && pull.deadline <= now);
        self.clips.pulls = waiting;
        for pull in expired {
            drop(pull.reply.send(Err(Error::Timeout)));
        }
    }
}

async fn serve(session: &SessionHandle, header: ClipPull, content: Option<Content>) -> Result<(), Error> {
    let stream = session.connection().open_uni().await?;
    let mut out = Outbound { stream, finished: false };
    write_frame(&mut out.stream, Message::ClipData(header), session.tap()).await?;
    match content {
        None => {
            out.stream.reset(CANCELLED);
            out.finished = true;
            return Ok(());
        }
        Some(Content::Text(text)) => out.stream.write_all(text.as_bytes()).await?,
        Some(Content::File(file, size)) => {
            let mut buf = vec![0; CHUNK];
            let mut offset = 0;
            while offset < size {
                let want = usize::try_from(size - offset).map_or(CHUNK, |left| left.min(CHUNK));
                let read = file.read_at(&mut buf[..want], offset)?;
                if read == 0 {
                    break;
                }
                out.stream.write_all(&buf[..read]).await?;
                offset += read as u64;
            }
        }
    }
    out.stream.finish()?;
    out.finished = true;
    Ok(())
}

/// A paste target: a pipe (a Wayland data source's) is written without blocking the runtime, a file directly.
enum Sink {
    Pipe(pipe::Sender),
    File(File),
}

impl Sink {
    fn new(file: File) -> Result<Self, Error> {
        if file.metadata()?.file_type().is_fifo() {
            return Ok(Self::Pipe(pipe::Sender::from_file(file)?));
        }
        Ok(Self::File(file))
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), Error> {
        match self {
            Self::Pipe(pipe) => pipe.write_all(bytes).await?,
            Self::File(file) => file.write_all(bytes)?,
        }
        Ok(())
    }
}

async fn write_sink(sink: File, bytes: &[u8]) -> Result<(), Error> {
    Sink::new(sink)?.write_all(bytes).await
}

/// Copies a `clip-data` stream into the sink within its budget, returning the byte count.
async fn copy(mut recv: RecvStream, sink: File, mut budget: Budget) -> Result<u64, Error> {
    let mut sink = Sink::new(sink)?;
    loop {
        let chunk = tokio::time::timeout(STEP_TIMEOUT, recv.read_chunk(CHUNK)).await?;
        let Some(chunk) = chunk.map_err(|_| Error::StreamEnded)? else { break };
        if budget.take(chunk.len()).is_err() {
            recv.stop(PROTOCOL_ERROR);
            return Err(Error::Unexpected("clipboard bytes past the offered size"));
        }
        sink.write_all(&chunk).await?;
    }
    Ok(budget.offset())
}

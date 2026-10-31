//! Mirroring the phone's screen: the streams' framing on both ends. See "Mirroring" in `ARCHITECTURE.md`.
//!
//! The first stream carries every unit, or only the sound when the desktop takes [`MIRROR_STREAMS`]. Then each
//! keyframe opens a new video stream at a higher send priority and ends the one before, so a backlog lost to a slow
//! link is dropped with its stream instead of holding the newer video behind it.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[cfg(doc)]
use link_proto::message::MIRROR_STREAMS;
use link_proto::message::{MAX_MIRROR_UNIT, MIRROR_AUDIO, MIRROR_END, MIRROR_KEYFRAME, Message, MirrorData};
use tokio::sync::{Mutex, mpsc, watch};

use crate::Error;
use crate::control::{Tap, write_frame};
use crate::session::SessionHandle;
use crate::wire::{Connection, RecvStream, SendStream};

/// Length, presentation time, flags.
const UNIT_HEADER: usize = 4 + 8 + 1;
/// A finished video stream not acknowledged by then is reset, so its queued bytes are dropped.
const RETIRE_GRACE: Duration = Duration::from_millis(250);
/// Units each stream's reader holds ahead of the consumer.
const READ_AHEAD: usize = 8;
/// How long the end of the first stream waits for each unit still on the video stream.
const DRAIN: Duration = Duration::from_millis(500);
/// Video streams waiting to be read; one more is stopped at once.
const MAX_PENDING_STREAMS: usize = 2;

/// One H.264 access unit in Annex B form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub pts_us: u64,
    /// [`link_proto::message::MIRROR_KEYFRAME`], [`link_proto::message::MIRROR_CONFIG`].
    pub flags: u8,
    pub data: Vec<u8>,
}

/// The phone's end. Sound and video may be sent at once from different tasks.
pub struct MirrorSender {
    connection: Connection,
    tap: Option<Tap>,
    width: u32,
    height: u32,
    main: Mutex<SendStream>,
    /// None when everything goes on the first stream.
    video: Option<Mutex<Video>>,
    /// Keyframes begun, plus one for [`MirrorSender::finish`]: a video write blocked by flow control gives way to
    /// either, which resets its stream.
    keys: watch::Sender<u64>,
    finished: AtomicBool,
}

#[derive(Default)]
struct Video {
    stream: Option<SendStream>,
    /// The `keys` count of the keyframe that opened `stream`.
    gop: u64,
}

impl MirrorSender {
    /// Opens the first stream; with `streams`, each keyframe opens a video stream of its own.
    pub async fn open(session: &SessionHandle, width: u32, height: u32, streams: bool) -> Result<Self, Error> {
        Self::start(session.connection().clone(), session.tap().cloned(), width, height, streams).await
    }

    async fn start(
        connection: Connection,
        tap: Option<Tap>,
        width: u32,
        height: u32,
        streams: bool,
    ) -> Result<Self, Error> {
        let main = open_stream(&connection, tap.as_ref(), width, height, None).await?;
        if streams {
            main.set_priority(i32::MAX);
        }
        Ok(Self {
            connection,
            tap,
            width,
            height,
            main: Mutex::new(main),
            video: streams.then(|| Mutex::new(Video::default())),
            keys: watch::Sender::new(0),
            finished: AtomicBool::new(false),
        })
    }

    /// Returns once the unit is queued; video before the first keyframe, or of a GOP a newer keyframe ends, is
    /// dropped.
    pub async fn send(&self, unit: &Unit) -> Result<(), Error> {
        if self.finished.load(Ordering::Relaxed) {
            return Err(Error::Stopped);
        }
        let Some(video) = self.video.as_ref().filter(|_| unit.flags & MIRROR_AUDIO == 0) else {
            return write_unit(&mut *self.main.lock().await, unit).await;
        };
        let mut seq = 0;
        if unit.flags & MIRROR_KEYFRAME != 0 {
            self.keys.send_modify(|keys| {
                *keys += 1;
                seq = *keys;
            });
        }
        let mut keys = self.keys.subscribe();
        let mut video = video.lock().await;
        if seq != 0 {
            if let Some(old) = video.stream.take() {
                retire(old);
            }
            if seq < *keys.borrow() {
                return Ok(());
            }
            video.gop = seq;
            let stream = open_stream(&self.connection, self.tap.as_ref(), self.width, self.height, Some(seq)).await?;
            stream.set_priority(i32::try_from(seq).unwrap_or(i32::MAX - 1).min(i32::MAX - 1));
            video.stream = Some(stream);
        }
        let gop = video.gop;
        let Some(stream) = video.stream.as_mut() else { return Ok(()) };
        let written = tokio::select! {
            biased;
            written = write_unit(stream, unit) => Some(written),
            _ = keys.wait_for(|keys| *keys > gop) => None,
        };
        if let Some(written) = written {
            return written;
        }
        if let Some(mut stream) = video.stream.take() {
            stream.reset(0);
        }
        if self.finished.load(Ordering::Relaxed) { Err(Error::Stopped) } else { Ok(()) }
    }

    pub async fn finish(&self) {
        self.finished.store(true, Ordering::Relaxed);
        self.keys.send_modify(|keys| *keys += 1);
        if let Some(video) = &self.video
            && let Some(mut stream) = video.lock().await.stream.take()
        {
            drop(stream.finish());
        }
        if let Err(error) = self.main.lock().await.finish() {
            log::debug!("finishing the mirror stream: {error}");
        }
    }
}

async fn open_stream(
    connection: &Connection,
    tap: Option<&Tap>,
    width: u32,
    height: u32,
    gop: Option<u64>,
) -> Result<SendStream, Error> {
    let mut send = connection.open_uni().await?;
    write_frame(&mut send, Message::MirrorData(MirrorData { width, height, gop }), tap).await?;
    Ok(send)
}

async fn write_unit(send: &mut SendStream, unit: &Unit) -> Result<(), Error> {
    let len = u32::try_from(unit.data.len()).ok().filter(|len| *len as usize <= MAX_MIRROR_UNIT);
    let len = len.ok_or(Error::Unexpected("oversized mirror unit"))?;
    let mut header = [0; UNIT_HEADER];
    header[..4].copy_from_slice(&len.to_be_bytes());
    header[4..12].copy_from_slice(&unit.pts_us.to_be_bytes());
    header[12] = unit.flags;
    send.write_all(&header).await?;
    send.write_all(&unit.data).await
}

/// Finishes a superseded video stream, and resets it unless the desktop has all of it soon.
fn retire(mut stream: SendStream) {
    drop(stream.finish());
    tokio::spawn(async move {
        if tokio::time::timeout(RETIRE_GRACE, stream.stopped()).await.is_err() {
            stream.reset(0);
        }
    });
}

async fn read_unit(recv: &mut RecvStream) -> Result<Option<Unit>, Error> {
    let mut header = [0; UNIT_HEADER];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        Err(Error::StreamEnded) => return Ok(None),
        Err(error) => return Err(error),
    }
    let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if len > MAX_MIRROR_UNIT {
        return Err(Error::Unexpected("oversized mirror unit"));
    }
    let mut pts = [0; 8];
    pts.copy_from_slice(&header[4..12]);
    let mut data = vec![0; len];
    recv.read_exact(&mut data).await?;
    // Only the daemon ends the viewer's stream.
    Ok(Some(Unit { pts_us: u64::from_be_bytes(pts), flags: header[12] & !MIRROR_END, data }))
}

type Units = mpsc::Receiver<Result<Unit, Error>>;

/// Reads a stream's units into a channel; dropping the channel's receiver stops the stream (`STOP_SENDING`).
fn reader(mut recv: RecvStream) -> Units {
    let (tx, rx) = mpsc::channel(READ_AHEAD);
    tokio::spawn(async move {
        loop {
            let unit = tokio::select! {
                () = tx.closed() => break recv.stop(0),
                unit = read_unit(&mut recv) => unit,
            };
            let last = !matches!(unit, Ok(Some(_)));
            let Some(unit) = unit.transpose() else { break };
            if tx.send(unit).await.is_err() {
                break recv.stop(0);
            }
            if last {
                break;
            }
        }
    });
    rx
}

/// Where the transfer actor hands a mirror's later video streams.
pub(crate) struct MirrorParts(mpsc::Sender<(u64, RecvStream)>);

impl MirrorParts {
    pub(crate) fn add(&self, gop: u64, recv: RecvStream) {
        if let Err(refused) = self.0.try_send((gop, recv)) {
            let (_, mut recv) = refused.into_inner();
            log::debug!("refusing mirror video stream {gop}");
            recv.stop(0);
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// The desktop's end, after the transfer actor read the first stream's header: that stream's units merged with
/// those of the newest video stream. An older video stream is stopped, and a reset one only ends its own video.
pub struct MirrorReceiver {
    pub width: u32,
    pub height: u32,
    main: Units,
    video: Option<Units>,
    /// A newer video stream, read once the current one has nothing ready.
    next: Option<Units>,
    gop: u64,
    parts: mpsc::Receiver<(u64, RecvStream)>,
    /// The first stream ended; what the video stream still has comes first.
    ended: bool,
}

impl MirrorReceiver {
    pub(crate) fn new(recv: RecvStream, header: &MirrorData) -> (Self, MirrorParts) {
        let (tx, parts) = mpsc::channel(MAX_PENDING_STREAMS);
        let receiver = Self {
            width: header.width,
            height: header.height,
            main: reader(recv),
            video: None,
            next: None,
            gop: 0,
            parts,
            ended: false,
        };
        (receiver, MirrorParts(tx))
    }

    /// The next unit, or None once the phone finished the first stream and the video stream's tail is read.
    /// Cancel-safe.
    pub async fn next(&mut self) -> Result<Option<Unit>, Error> {
        loop {
            if self.next.is_some() {
                if let Some(Ok(Ok(unit))) = self.video.as_mut().map(mpsc::Receiver::try_recv) {
                    return Ok(Some(unit));
                }
                self.video = self.next.take();
            }
            if self.ended {
                let Some(units) = self.video.as_mut() else { return Ok(None) };
                match tokio::time::timeout(DRAIN, units.recv()).await {
                    Ok(Some(Ok(unit))) => return Ok(Some(unit)),
                    _ => self.video = None,
                }
                continue;
            }
            tokio::select! {
                unit = recv_some(&mut self.video) => match unit {
                    Some(Ok(unit)) => return Ok(Some(unit)),
                    Some(Err(error)) => {
                        log::debug!("mirror video stream {} ended: {error}", self.gop);
                        self.video = None;
                    }
                    None => self.video = None,
                },
                unit = self.main.recv() => match unit {
                    Some(unit) => return unit.map(Some),
                    None => self.ended = true,
                },
                Some((gop, mut recv)) = self.parts.recv() => {
                    if gop > self.gop {
                        self.gop = gop;
                        self.next = Some(reader(recv));
                    } else {
                        log::debug!("stopping mirror video stream {gop}: {} is newer", self.gop);
                        recv.stop(0);
                    }
                }
            }
        }
    }

    pub fn stop(&mut self) {
        self.main.close();
        self.parts.close();
        self.video = None;
        self.next = None;
    }
}

/// The channel's next item; pending forever without a channel.
async fn recv_some(units: &mut Option<Units>) -> Option<Result<Unit, Error>> {
    match units {
        Some(units) => units.recv().await,
        None => std::future::pending().await,
    }
}

/// A peer's mirror as a transfer event carries it: whoever handles the event takes it once.
#[derive(Clone)]
pub struct MirrorStream(Arc<StdMutex<Option<MirrorReceiver>>>);

impl MirrorStream {
    pub(crate) fn new(receiver: MirrorReceiver) -> Self {
        Self(Arc::new(StdMutex::new(Some(receiver))))
    }

    pub fn take(&self) -> Option<MirrorReceiver> {
        self.0.lock().ok()?.take()
    }
}

impl std::fmt::Debug for MirrorStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MirrorStream")
    }
}

impl PartialEq for MirrorStream {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for MirrorStream {}

#[cfg(test)]
mod tests {
    use link_proto::message::Envelope;

    use super::*;
    use crate::control::read_frame;
    use crate::mux::MuxConnection;

    fn unit(pts_us: u64, flags: u8) -> Unit {
        Unit { pts_us, flags, data: pts_us.to_be_bytes().to_vec() }
    }

    async fn open(phone: &Connection, gop: Option<u64>) -> Result<SendStream, Error> {
        let mut send = phone.open_uni().await?;
        write_frame(&mut send, Message::MirrorData(MirrorData { width: 16, height: 16, gop }), None).await?;
        Ok(send)
    }

    async fn accept(desktop: &Connection) -> Result<(RecvStream, MirrorData), Error> {
        let mut recv = desktop.accept_uni().await?;
        let Envelope { message: Message::MirrorData(header), .. } = read_frame(&mut recv, None).await? else {
            return Err(Error::Unexpected("no mirror-data header"));
        };
        Ok((recv, header))
    }

    #[tokio::test]
    async fn the_newest_video_stream_wins_and_older_or_reset_ones_end_nothing() -> Result<(), Error> {
        let (phone, desktop) = tokio::io::duplex(1 << 16);
        let phone = Connection::Stream(MuxConnection::new(phone, true, None));
        let desktop = Connection::Stream(MuxConnection::new(desktop, false, None));
        let mut main = open(&phone, None).await?;
        let (recv, header) = accept(&desktop).await?;
        let (mut receiver, parts) = MirrorReceiver::new(recv, &header);

        let mut first = open(&phone, Some(1)).await?;
        write_unit(&mut first, &unit(1, MIRROR_KEYFRAME)).await?;
        let (recv, header) = accept(&desktop).await?;
        parts.add(header.gop.ok_or(Error::Unexpected("no gop"))?, recv);
        assert_eq!(receiver.next().await?, Some(unit(1, MIRROR_KEYFRAME)));

        first.reset(0);
        write_unit(&mut main, &unit(2, MIRROR_AUDIO)).await?;
        assert_eq!(receiver.next().await?, Some(unit(2, MIRROR_AUDIO)));

        let mut third = open(&phone, Some(3)).await?;
        let mut second = open(&phone, Some(2)).await?;
        write_unit(&mut third, &unit(3, MIRROR_KEYFRAME)).await?;
        write_unit(&mut second, &unit(4, MIRROR_KEYFRAME)).await?;
        for _ in 0..2 {
            let (recv, header) = accept(&desktop).await?;
            parts.add(header.gop.ok_or(Error::Unexpected("no gop"))?, recv);
        }
        assert_eq!(receiver.next().await?, Some(unit(3, MIRROR_KEYFRAME)));
        write_unit(&mut third, &unit(5, 0)).await?;
        assert_eq!(receiver.next().await?, Some(unit(5, 0)));
        main.finish()?;
        assert_eq!(receiver.next().await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn a_keyframe_resets_the_stream_whose_write_flow_control_holds() -> Result<(), Error> {
        let (phone, desktop) = tokio::io::duplex(1 << 16);
        let phone = Connection::Stream(MuxConnection::new(phone, true, None));
        let desktop = Connection::Stream(MuxConnection::new(desktop, false, None));
        let sender = MirrorSender::start(phone, None, 16, 16, true).await?;
        let _main = accept(&desktop).await?;
        sender.send(&unit(1, MIRROR_KEYFRAME)).await?;
        let (mut old, _) = accept(&desktop).await?;

        // The desktop reads no further, so the stream's credit window runs out under this unit.
        let backlog = Unit { pts_us: 2, flags: 0, data: vec![0; 2 * link_proto::mux::WINDOW as usize] };
        let mut blocked = std::pin::pin!(sender.send(&backlog));
        tokio::select! {
            biased;
            _ = &mut blocked => return Err(Error::Unexpected("the backlog was queued at once")),
            () = std::future::ready(()) => {}
        }
        let keyframe = unit(3, MIRROR_KEYFRAME);
        let sent =
            tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(blocked, sender.send(&keyframe)) });
        let (backlog, keyframe) = sent.await.map_err(|_| Error::Unexpected("the keyframe waited on the backlog"))?;
        backlog?;
        keyframe?;

        let ended = loop {
            match read_unit(&mut old).await {
                Ok(Some(_)) => {}
                other => break other,
            }
        };
        assert!(ended.is_err(), "the old stream was reset, not drained: {ended:?}");
        let (mut new, header) = accept(&desktop).await?;
        assert_eq!(header.gop, Some(2));
        assert_eq!(read_unit(&mut new).await?, Some(unit(3, MIRROR_KEYFRAME)));
        sender.finish().await;
        assert_eq!(read_unit(&mut new).await?, None);
        Ok(())
    }
}

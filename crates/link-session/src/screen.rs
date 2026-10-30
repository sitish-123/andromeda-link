//! Remote control: the screen through the `ScreenCast` portal (`PipeWire`), H.264 with the sound in Opus, on a socket
//! the daemon pumps to the phone. The portal asks the user once which monitors and windows to share; its restore token
//! starts later sessions without asking. One phone at a time.

use std::collections::HashMap;
use std::io::Write as _;
use std::os::fd::{AsFd as _, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context as _;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use tokio::sync::mpsc;
use zbus::{OwnedObjectPath, OwnedVariant, Variant};

use crate::input::Point;
use crate::portal::{PATH, SERVICE, request, state_path, string, token};

const SCREEN_CAST: &str = "org.freedesktop.portal.ScreenCast";
const MONITOR: u32 = 1;
const WINDOW: u32 = 2;
/// The cursor drawn into the picture, as a phone shows no pointer of its own.
const CURSOR_EMBEDDED: u32 = 2;
const PERSIST: u32 = 2;
/// The picture's longer side at most, which phones decode in hardware.
const MAX_SIDE: i32 = 1920;
/// The stream's framing, as `OpenMirror`'s.
const KEYFRAME: u8 = 1;
const CONFIG: u8 = 2;
const AUDIO: u8 = 4;
/// `PipeWire` sends a frame only on damage; a still screen repeats its last one this often, in ms.
const KEEPALIVE_MS: i32 = 1000;
const FIRST_FRAME: Duration = Duration::from_secs(5);

pub enum Request {
    Show { device: String, source: Option<u32> },
    Stop { device: String },
}

/// Whether the session bus has the `ScreenCast` portal.
pub async fn available(bus: &zbus::Connection) -> bool {
    let Ok(portal) = zbus::Proxy::new(bus, SERVICE, PATH, SCREEN_CAST).await else { return false };
    portal.get_property::<u32>("AvailableSourceTypes").await.is_ok_and(|types| types & MONITOR != 0)
}

/// Starts the remote-control task; the pointer's target for direct touch is in the returned cell.
pub fn spawn(bus: zbus::Connection, link: crate::link::Link) -> (mpsc::UnboundedSender<Request>, Target) {
    let (requests, receive) = mpsc::unbounded_channel();
    let target = Target::default();
    tokio::spawn(run(bus, link, receive, target.clone()));
    (requests, target)
}

/// The monitor the phone shows, which direct touch points into; None for a window, whose place the portal hides.
#[derive(Clone, Default)]
pub struct Target(Arc<Mutex<Option<Monitor>>>);

/// A monitor's output name and its position and size in the layout.
type Monitor = (Option<String>, (i32, i32, i32, i32));

impl Target {
    /// The phone's point, in 1/10000 of the picture, on the shown monitor.
    pub fn point(&self, dx: i32, dy: i32) -> Option<Point> {
        let (output, rect) = self.0.lock().ok()?.clone()?;
        Some(Point { output, rect, x: f64::from(dx) / 10_000.0, y: f64::from(dy) / 10_000.0 })
    }

    fn set(&self, source: Option<&Source>) {
        if let Ok(mut target) = self.0.lock() {
            *target = source.filter(|source| source.kind == MONITOR).map(|source| (source.output.clone(), source.rect));
        }
    }
}

struct Source {
    node: u32,
    kind: u32,
    name: String,
    output: Option<String>,
    rect: (i32, i32, i32, i32),
}

struct Showing {
    device: String,
    capture: Capture,
    stream: Stream,
}

async fn run(
    bus: zbus::Connection,
    link: crate::link::Link,
    mut requests: mpsc::UnboundedReceiver<Request>,
    target: Target,
) {
    let (ended_send, mut ended) = mpsc::unbounded_channel::<u64>();
    let mut showing: Option<Showing> = None;
    let mut generation = 0;
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                Some(Request::Show { device, source }) => {
                    if let Some(old) = showing.take_if(|old| old.device != device) {
                        link.stop_remote_control(&old.device, "another phone took over the screen").await;
                        old.capture.close().await;
                    }
                    let capture = match showing.take() {
                        Some(showing) => showing.capture,
                        None => match Capture::start(&bus).await {
                            Ok(capture) => capture,
                            Err(error) => {
                                log::warn!("remote control: {error:#}");
                                link.stop_remote_control(&device, "the desktop did not share its screen").await;
                                continue;
                            }
                        },
                    };
                    generation += 1;
                    match show(&link, &device, &capture, source, generation, ended_send.clone()).await {
                        Ok((stream, shown)) => {
                            target.set(capture.sources.get(shown));
                            showing = Some(Showing { device, capture, stream });
                        }
                        Err(error) => {
                            log::warn!("remote control: {error:#}");
                            link.stop_remote_control(&device, "the desktop could not encode its screen").await;
                            target.set(None);
                            capture.close().await;
                        }
                    }
                }
                Some(Request::Stop { device }) => {
                    if let Some(old) = showing.take_if(|old| old.device == device) {
                        let (frames, sound) = old.stream.sent();
                        log::info!("{device}: remote control ended: {frames} frames, {sound} sound packets");
                        target.set(None);
                        old.capture.close().await;
                    }
                }
                None => return,
            },
            Some(ended) = ended.recv() => {
                if ended == generation && let Some(old) = showing.take() {
                    link.stop_remote_control(&old.device, "the shared screen or window went away").await;
                    target.set(None);
                    old.capture.close().await;
                }
            }
        }
    }
}

/// Encodes `sources[source]` (the first when None) and hands the daemon its socket; returns the stream and its index.
async fn show(
    link: &crate::link::Link,
    device: &str,
    capture: &Capture,
    source: Option<u32>,
    generation: u64,
    ended: mpsc::UnboundedSender<u64>,
) -> anyhow::Result<(Stream, usize)> {
    let index = source.and_then(|source| usize::try_from(source).ok()).filter(|index| *index < capture.sources.len());
    let index = index.unwrap_or(0);
    let shown = capture.sources.get(index).context("the portal shared nothing")?;
    let remote = capture.remote.try_clone()?;
    let (node, rect) = (shown.node, shown.rect);
    let (stream, socket, width, height) =
        tokio::task::spawn_blocking(move || Stream::start(&remote, node, rect, generation, &ended)).await??;
    let sources: Vec<(&str, &str)> = capture
        .sources
        .iter()
        .map(|source| (if source.kind == MONITOR { "monitor" } else { "window" }, source.name.as_str()))
        .collect();
    let sources: Vec<&(&str, &str)> = sources.iter().collect();
    let current = u32::try_from(index)?;
    link.proxy
        .serve_remote_control(device, socket.as_fd().into(), width, height, &sources, current)
        .await
        .context("ServeRemoteControl")?;
    log::info!("{device}: showing {} at {width}x{height}", shown.name);
    Ok((stream, index))
}

/// A `ScreenCast` portal session: the shared sources and the `PipeWire` remote that carries them.
struct Capture {
    bus: zbus::Connection,
    handle: OwnedObjectPath,
    remote: OwnedFd,
    sources: Vec<Source>,
}

impl Capture {
    async fn start(bus: &zbus::Connection) -> anyhow::Result<Self> {
        let portal = &zbus::Proxy::new(bus, SERVICE, PATH, SCREEN_CAST).await?;
        let cursors = portal.get_property::<u32>("AvailableCursorModes").await.unwrap_or(0);
        let session_token = token();
        let results = request(bus, "CreateSession", |token| async move {
            let options = HashMap::from([
                ("handle_token", Variant::from(token.as_str())),
                ("session_handle_token", Variant::from(session_token.as_str())),
            ]);
            portal.call("CreateSession", &(options,)).await
        })
        .await?;
        let handle = &OwnedObjectPath::try_from(string(&results, "session_handle")?)?;
        let restore = std::fs::read_to_string(state_path("screencast-restore-token")).unwrap_or_default();
        let restore = restore.trim();
        request(bus, "SelectSources", |token| async move {
            let mut options = HashMap::from([
                ("handle_token", Variant::from(token.as_str())),
                ("types", Variant::from(MONITOR | WINDOW)),
                ("multiple", Variant::from(true)),
                ("persist_mode", Variant::from(PERSIST)),
            ]);
            if cursors & CURSOR_EMBEDDED != 0 {
                options.insert("cursor_mode", Variant::from(CURSOR_EMBEDDED));
            }
            if !restore.is_empty() {
                options.insert("restore_token", Variant::from(restore));
            }
            portal.call("SelectSources", &(handle, options)).await
        })
        .await?;
        let results = request(bus, "Start", |token| async move {
            let options = HashMap::from([("handle_token", Variant::from(token.as_str()))]);
            portal.call("Start", &(handle, "", options)).await
        })
        .await?;
        if let Ok(token) = string(&results, "restore_token") {
            let path = state_path("screencast-restore-token");
            let saved =
                path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, token));
            if let Err(error) = saved {
                log::warn!("saving the ScreenCast restore token: {error}");
            }
        }
        let streams = results.get("streams").context("the portal shared no streams")?;
        let streams = <Vec<(u32, HashMap<String, OwnedVariant>)>>::try_from(streams.clone())?;
        let mut windows = 0;
        let sources: Vec<Source> = streams
            .into_iter()
            .enumerate()
            .map(|(index, (node, properties))| {
                let pair = |key: &str| properties.get(key).and_then(|value| <(i32, i32)>::try_from(value.clone()).ok());
                let kind = properties.get("source_type").and_then(|value| u32::try_from(value).ok()).unwrap_or(MONITOR);
                let output = properties.get("mapping_id").and_then(|value| String::try_from(value.clone()).ok());
                let (x, y) = pair("position").unwrap_or_default();
                let (width, height) = pair("size").unwrap_or_default();
                let name = if kind == MONITOR {
                    output.clone().unwrap_or_else(|| format!("Screen {}", index + 1))
                } else {
                    windows += 1;
                    format!("Window {windows}")
                };
                Source { node, kind, name, output, rect: (x, y, width, height) }
            })
            .collect();
        anyhow::ensure!(!sources.is_empty(), "the portal shared no streams");
        let options: HashMap<&str, Variant<'_>> = HashMap::new();
        let remote: zbus::OwnedFd = portal.call("OpenPipeWireRemote", &(handle, options)).await?;
        log::info!("sharing {}", sources.iter().map(|source| source.name.as_str()).collect::<Vec<_>>().join(", "));
        Ok(Self { bus: bus.clone(), handle: handle.clone(), remote: remote.into(), sources })
    }

    async fn close(self) {
        let closed = async {
            let session = zbus::Proxy::new(&self.bus, SERVICE, &self.handle, "org.freedesktop.portal.Session").await?;
            session.call::<_, _, ()>("Close", &()).await
        };
        if let Err(error) = closed.await {
            log::debug!("closing the ScreenCast session: {error}");
        }
    }
}

/// The video and sound pipelines writing to the daemon's socket; they stop when dropped.
struct Stream {
    pipelines: Vec<gst::Pipeline>,
    /// Video and sound units written so far.
    counts: Arc<(AtomicU64, AtomicU64)>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        for pipeline in &self.pipelines {
            if let Err(error) = pipeline.set_state(gst::State::Null) {
                log::debug!("stopping a pipeline: {error}");
            }
        }
    }
}

impl Stream {
    fn sent(&self) -> (u64, u64) {
        (self.counts.0.load(Ordering::Relaxed), self.counts.1.load(Ordering::Relaxed))
    }

    /// Starts encoding `node` of `remote` and waits for the first frame, whose size the phone needs first. Returns
    /// the daemon's end of the socket. A pipeline that fails or ends sends `generation` on `ended`.
    fn start(
        remote: &OwnedFd,
        node: u32,
        rect: (i32, i32, i32, i32),
        generation: u64,
        ended: &mpsc::UnboundedSender<u64>,
    ) -> anyhow::Result<(Self, OwnedFd, u32, u32)> {
        gst::init()?;
        let (ours, theirs) = UnixStream::pair()?;
        let writer = Arc::new(Mutex::new(ours));
        let fd = rustix::io::dup(remote)?;
        let (width, height) = fit(rect.2, rect.3);
        let encoder = if gst::ElementFactory::find("x264enc").is_some() {
            "x264enc tune=zerolatency speed-preset=ultrafast key-int-max=60 bitrate=8000 \
             ! video/x-h264,profile=constrained-baseline"
        } else {
            "openh264enc complexity=low gop-size=60 bitrate=8000000"
        };
        // The pipeline owns the duplicate descriptor and closes it.
        let video = pipeline(&format!(
            "pipewiresrc fd={} path={node} do-timestamp=true always-copy=true keepalive-time={KEEPALIVE_MS} \
             ! queue leaky=downstream max-size-buffers=2 ! videoconvert ! videoscale \
             ! video/x-raw,format=I420,width={width},height={height},pixel-aspect-ratio=1/1 ! {encoder} \
             ! h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au \
             ! appsink name=sink sync=false",
            std::os::fd::IntoRawFd::into_raw_fd(fd)
        ))?;
        watch(&video, generation, ended.clone());
        let sink = appsink(&video)?;
        video.set_state(gst::State::Playing)?;
        let first = sink
            .try_pull_sample(gst::ClockTime::from_mseconds(u64::try_from(FIRST_FRAME.as_millis())?))
            .context("no frame from the screen in 5 s")?;
        let caps = first.caps().context("a frame without caps")?;
        let structure = caps.structure(0).context("empty caps")?;
        let (width, height) =
            (u32::try_from(structure.get::<i32>("width")?)?, u32::try_from(structure.get::<i32>("height")?)?);
        write_video(&writer, &first).context("writing the first frame")?;
        let counts = Arc::new((AtomicU64::new(1), AtomicU64::new(0)));
        let video_writer = Arc::clone(&writer);
        let video_counts = Arc::clone(&counts);
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    write_video(&video_writer, &sample).map_err(|_| gst::FlowError::Error)?;
                    video_counts.0.fetch_add(1, Ordering::Relaxed);
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        let mut pipelines = vec![video];
        match sound(&writer, &counts) {
            Ok(audio) => pipelines.push(audio),
            Err(error) => log::info!("remote control without sound: {error:#}"),
        }
        Ok((Self { pipelines, counts }, theirs.into(), width, height))
    }
}

/// The desktop's sound from the default output's monitor, one Opus packet per unit; it may fail without the video.
fn sound(writer: &Arc<Mutex<UnixStream>>, counts: &Arc<(AtomicU64, AtomicU64)>) -> anyhow::Result<gst::Pipeline> {
    let audio = pipeline(
        "pulsesrc device=@DEFAULT_MONITOR@ do-timestamp=true ! audioconvert ! audioresample \
         ! audio/x-raw,rate=48000,channels=2 ! opusenc bitrate=128000 frame-size=20 ! appsink name=sink sync=false",
    )?;
    let (writer, counts) = (Arc::clone(writer), Arc::clone(counts));
    appsink(&audio)?.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let pts = buffer.pts().map_or(0, gst::ClockTime::useconds);
                write_unit(&writer, pts, AUDIO, &map).map_err(|_| gst::FlowError::Error)?;
                counts.1.fetch_add(1, Ordering::Relaxed);
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    audio.set_state(gst::State::Playing)?;
    Ok(audio)
}

/// A keyframe goes as its parameter sets (a configuration unit), then the rest, as Android's decoder takes them.
fn write_video(writer: &Mutex<UnixStream>, sample: &gst::Sample) -> std::io::Result<()> {
    let Some(buffer) = sample.buffer() else { return Ok(()) };
    let map = buffer.map_readable().map_err(std::io::Error::other)?;
    let pts = buffer.pts().map_or(0, gst::ClockTime::useconds);
    if buffer.flags().contains(gst::BufferFlags::DELTA_UNIT) {
        return write_unit(writer, pts, 0, &map);
    }
    let (config, frame) = split_config(&map);
    if !config.is_empty() {
        write_unit(writer, pts, CONFIG, &config)?;
    }
    write_unit(writer, pts, KEYFRAME, &frame)
}

fn write_unit(writer: &Mutex<UnixStream>, pts_us: u64, flags: u8, data: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(data.len()).map_err(std::io::Error::other)?;
    let mut header = [0; 13];
    header[..4].copy_from_slice(&len.to_be_bytes());
    header[4..12].copy_from_slice(&pts_us.to_be_bytes());
    header[12] = flags;
    let mut socket = writer.lock().map_err(|_| std::io::Error::other("a writer panicked"))?;
    socket.write_all(&header)?;
    socket.write_all(data)
}

/// Splits an Annex B access unit into its SPS and PPS NAL units and the rest, each with its start codes.
fn split_config(unit: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut starts: Vec<usize> =
        unit.windows(3).enumerate().filter(|(_, w)| *w == [0, 0, 1]).map(|(at, _)| at).collect();
    starts.push(unit.len());
    let (mut config, mut frame) = (Vec::new(), Vec::new());
    for pair in starts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        // A four-byte start code's leading zero ends the previous unit; each unit goes out with a four-byte code.
        let end = if end < unit.len() && end > 0 && unit[end - 1] == 0 { end - 1 } else { end };
        let nal = &unit[start + 3..end];
        let target = if matches!(nal.first().map(|byte| byte & 0x1f), Some(7 | 8)) { &mut config } else { &mut frame };
        target.extend_from_slice(&[0, 0, 0, 1]);
        target.extend_from_slice(nal);
    }
    (config, frame)
}

/// The size the picture is encoded at: the source's aspect, the longer side at most [`MAX_SIDE`], even sides.
fn fit(width: i32, height: i32) -> (i32, i32) {
    let (width, height) = (width.max(2), height.max(2));
    let scale = f64::from(MAX_SIDE) / f64::from(width.max(height));
    let scale = scale.min(1.0);
    #[expect(clippy::cast_possible_truncation, reason = "a size under MAX_SIDE")]
    let even = |side: i32| ((f64::from(side) * scale) as i32 / 2 * 2).max(2);
    (even(width), even(height))
}

fn pipeline(description: &str) -> anyhow::Result<gst::Pipeline> {
    gst::parse::launch(description)?.downcast::<gst::Pipeline>().map_err(|_| anyhow::anyhow!("not a pipeline"))
}

fn appsink(pipeline: &gst::Pipeline) -> anyhow::Result<gst_app::AppSink> {
    pipeline
        .by_name("sink")
        .and_then(|sink| sink.downcast::<gst_app::AppSink>().ok())
        .context("the pipeline has no appsink")
}

/// Sends `generation` on `ended` when the pipeline fails or ends.
fn watch(pipeline: &gst::Pipeline, generation: u64, ended: mpsc::UnboundedSender<u64>) {
    let Some(bus) = pipeline.bus() else { return };
    bus.set_sync_handler(move |_, message| {
        match message.view() {
            gst::MessageView::Error(error) => {
                log::info!("remote control stream: {}", error.error());
                ended.send(generation).unwrap_or_default();
            }
            gst::MessageView::Eos(_) => ended.send(generation).unwrap_or_default(),
            _ => {}
        }
        gst::BusSyncReply::Drop
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keyframe_splits_into_its_parameter_sets_and_the_slice() {
        let unit = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4, 5];
        let (config, frame) = split_config(&unit);
        assert_eq!(config, [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3]);
        assert_eq!(frame, [0, 0, 0, 1, 0x65, 4, 5]);
    }
}

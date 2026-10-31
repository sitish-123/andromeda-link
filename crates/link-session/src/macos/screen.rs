//! Remote control on macOS: a display through `ScreenCaptureKit`, H.264 from `VideoToolbox`, on a socket the daemon
//! pumps to the phone, framed as on Linux. Needs the Screen Recording permission. The Mac's sound is not sent. One phone
//! at a time.

#![allow(unsafe_code, reason = "FFI to ScreenCaptureKit, VideoToolbox, CoreMedia and CoreFoundation")]

use std::ffi::c_void;
use std::io::Write as _;
use std::os::fd::AsFd as _;
use std::os::unix::net::UnixStream;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AllocAnyThread as _, DefinedClass as _, define_class, msg_send};
use objc2_core_foundation::{CFBoolean, CFNumber, CFString, CFType};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput, SCStreamOutputType,
};
use tokio::sync::{mpsc, oneshot};

use crate::input::Point;
use crate::link::Link;

/// The picture's longer side at most, which phones decode in hardware.
const MAX_SIDE: usize = 1920;
const FRAMES_PER_SECOND: i32 = 30;
/// The stream's framing, as `OpenMirror`'s.
const KEYFRAME: u8 = 1;
const CONFIG: u8 = 2;

pub enum Request {
    Show { device: String, source: Option<u32> },
    Stop { device: String },
}

/// Whether this program may record the screen; without it, asks, so the helper shows in the Screen Recording list.
pub fn available() -> bool {
    // SAFETY: both calls take no arguments.
    granted() || unsafe { CGRequestScreenCaptureAccess() }
}

/// Whether this program may record the screen, without a prompt.
pub fn granted() -> bool {
    // SAFETY: the call takes no arguments.
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// Starts the remote-control task; the pointer's target for direct touch is in the returned cell.
pub fn spawn(link: Link) -> (mpsc::UnboundedSender<Request>, Target) {
    let (requests, receive) = mpsc::unbounded_channel();
    let target = Target::default();
    tokio::spawn(run(link, receive, target.clone()));
    (requests, target)
}

/// The shown display's bounds in global points, which direct touch points into.
#[derive(Clone, Default)]
pub struct Target(Arc<Mutex<Option<Rect>>>);

/// x, y, width, height in global points.
type Rect = (i32, i32, i32, i32);

impl Target {
    /// The phone's point, in 1/10000 of the picture, on the shown display.
    pub fn point(&self, dx: i32, dy: i32) -> Option<Point> {
        let rect = (*self.0.lock().ok()?)?;
        Some(Point { output: None, rect, x: f64::from(dx) / 10_000.0, y: f64::from(dy) / 10_000.0 })
    }

    fn set(&self, rect: Option<(i32, i32, i32, i32)>) {
        if let Ok(mut target) = self.0.lock() {
            *target = rect;
        }
    }
}

struct Showing {
    device: String,
    capture: Capture,
}

async fn run(link: Link, mut requests: mpsc::UnboundedReceiver<Request>, target: Target) {
    let (ended_send, mut ended) = mpsc::unbounded_channel::<u64>();
    let mut showing: Option<Showing> = None;
    let mut generation = 0;
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                Some(Request::Show { device, source }) => {
                    if let Some(old) = showing.take_if(|old| old.device != device) {
                        link.stop_remote_control(&old.device, "another phone took over the screen").await;
                    }
                    // Switching sources starts a new capture of the other display.
                    drop(showing.take());
                    target.set(None);
                    generation += 1;
                    match show(&link, &device, source, generation, ended_send.clone()).await {
                        Ok((capture, rect)) => {
                            target.set(Some(rect));
                            showing = Some(Showing { device, capture });
                        }
                        Err(error) => {
                            log::warn!("remote control: {error:#}");
                            link.stop_remote_control(&device, "the Mac could not share its screen").await;
                        }
                    }
                }
                Some(Request::Stop { device }) => {
                    if let Some(old) = showing.take_if(|old| old.device == device) {
                        log::info!("{device}: remote control ended: {} frames", old.capture.frames());
                        target.set(None);
                    }
                }
                None => return,
            },
            Some(ended) = ended.recv() => {
                if ended == generation && let Some(old) = showing.take() {
                    link.stop_remote_control(&old.device, "the screen stream stopped").await;
                    target.set(None);
                }
            }
        }
    }
}

/// Captures display `source` (the first when None) and hands the daemon its socket; returns the capture and the
/// display's bounds.
async fn show(
    link: &Link,
    device: &str,
    source: Option<u32>,
    generation: u64,
    ended: mpsc::UnboundedSender<u64>,
) -> anyhow::Result<(Capture, (i32, i32, i32, i32))> {
    let displays = displays().await?;
    let count = displays.0.len();
    let index = source.and_then(|source| usize::try_from(source).ok()).filter(|index| *index < count);
    let index = index.unwrap_or(0);
    let (ours, theirs) = UnixStream::pair()?;
    // The displays are not Send, so they go before the next await.
    let (capture, width, height, rect) = {
        let display = displays.0.get(index).context("no display to share")?;
        // SAFETY: a plain getter of a display ScreenCaptureKit gave.
        let frame = unsafe { display.frame() };
        #[expect(clippy::cast_possible_truncation, reason = "display bounds in points")]
        let rect = (frame.origin.x as i32, frame.origin.y as i32, frame.size.width as i32, frame.size.height as i32);
        let (capture, width, height) = Capture::start(display, ours, generation, ended)?;
        drop(displays);
        (capture, width, height, rect)
    };
    let names: Vec<String> = (1..=count).map(|n| format!("Display {n}")).collect();
    let sources: Vec<(&str, &str)> = names.iter().map(|name| ("monitor", name.as_str())).collect();
    let sources: Vec<&(&str, &str)> = sources.iter().collect();
    let (width, height) = (u32::try_from(width)?, u32::try_from(height)?);
    link.proxy
        .serve_remote_control(device, theirs.as_fd().into(), width, height, &sources, u32::try_from(index)?)
        .await
        .context("ServeRemoteControl")?;
    log::info!("{device}: showing display {} at {width}x{height}", index + 1);
    Ok((capture, rect))
}

struct Displays(Vec<Retained<SCDisplay>>);

// SAFETY: an `SCDisplay` is an immutable description, safe to read from any thread.
unsafe impl Send for Displays {}

async fn displays() -> anyhow::Result<Displays> {
    let (send, receive) = oneshot::channel();
    // In a block: ScreenCaptureKit copies the handler, and ours is not Send, so it ends before the await.
    {
        let send = Mutex::new(Some(send));
        let handler = block2::RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: ScreenCaptureKit passes a valid content or error, either possibly null.
            let result = match unsafe { (content.as_ref(), error.as_ref()) } {
                (Some(content), _) => Ok(Displays(unsafe { content.displays() }.to_vec())),
                (None, Some(error)) => Err(anyhow::anyhow!("{}", error.localizedDescription())),
                (None, None) => Err(anyhow::anyhow!("no shareable content")),
            };
            if let Some(send) = send.lock().ok().and_then(|mut send| send.take()) {
                drop(send.send(result));
            }
        });
        // SAFETY: the handler is a block of the declared type.
        unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    }
    receive.await.context("ScreenCaptureKit never answered")?
}

/// A running `SCStream` of one display and its encoder.
struct Capture {
    stream: Retained<SCStream>,
    output: Retained<Output>,
}

impl Capture {
    fn start(
        display: &SCDisplay,
        socket: UnixStream,
        generation: u64,
        ended: mpsc::UnboundedSender<u64>,
    ) -> anyhow::Result<(Self, usize, usize)> {
        // SAFETY: getters of a valid display.
        let (width, height) = fit(unsafe { display.width() }, unsafe { display.height() });
        let encoder = Encoder::new(width, height, socket, generation, ended)?;
        let output = Output::new(encoder);
        // SAFETY: ScreenCaptureKit objects built from valid arguments; the setters take plain values.
        let stream = unsafe {
            let filter =
                SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), display, &NSArray::new());
            let config = SCStreamConfiguration::new();
            config.setWidth(width);
            config.setHeight(height);
            config.setPixelFormat(NV12);
            config.setShowsCursor(true);
            config.setMinimumFrameInterval(CMTime {
                value: 1,
                timescale: FRAMES_PER_SECOND,
                flags: CMTimeFlags::Valid,
                epoch: 0,
            });
            let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None);
            let queue = DispatchQueue::new("org.andromeda.link.screen", None);
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*output),
                    SCStreamOutputType::Screen,
                    Some(&queue),
                )
                .map_err(|error| anyhow::anyhow!("adding the stream output: {}", error.localizedDescription()))?;
            stream.startCaptureWithCompletionHandler(Some(&block2::RcBlock::new(|error: *mut NSError| {
                if let Some(error) = error.as_ref() {
                    log::warn!("starting the screen capture: {}", error.localizedDescription());
                }
            })));
            stream
        };
        Ok((Self { stream, output }, width, height))
    }

    fn frames(&self) -> u64 {
        self.output.ivars().shared.frames.load(Ordering::Relaxed)
    }
}

// SAFETY: an `SCStream` may be stopped from any thread, and the output's encoder state is atomics, a mutex, and a
// VideoToolbox session, which is thread-safe.
unsafe impl Send for Capture {}

impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: stopping a started stream; no completion handler.
        unsafe { self.stream.stopCaptureWithCompletionHandler(None) };
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing rules, and `Output` adds no Drop.
    #[unsafe(super(NSObject))]
    #[name = "AndromedaLinkScreenOutput"]
    #[ivars = Encoder]
    struct Output;

    unsafe impl NSObjectProtocol for Output {}

    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn did_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            if kind == SCStreamOutputType::Screen {
                self.ivars().encode(sample);
            }
        }
    }
);

impl Output {
    fn new(encoder: Encoder) -> Retained<Self> {
        let this = Self::alloc().set_ivars(encoder);
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }
}

/// What the encoder's callback writes with; boxed so its address stays put.
struct Shared {
    socket: Mutex<UnixStream>,
    generation: u64,
    ended: mpsc::UnboundedSender<u64>,
    failed: AtomicBool,
    frames: AtomicU64,
}

struct Encoder {
    session: CfRef,
    shared: Box<Shared>,
}

impl Encoder {
    fn new(
        width: usize,
        height: usize,
        socket: UnixStream,
        generation: u64,
        ended: mpsc::UnboundedSender<u64>,
    ) -> anyhow::Result<Self> {
        let shared = Box::new(Shared {
            socket: Mutex::new(socket),
            generation,
            ended,
            failed: AtomicBool::new(false),
            frames: AtomicU64::new(0),
        });
        let mut session = null();
        let refcon = std::ptr::from_ref(&*shared).cast_mut().cast();
        // SAFETY: plain sizes and codec; `refcon` points at `shared`, which lives as long as the session.
        let status = unsafe {
            VTCompressionSessionCreate(
                null(),
                i32::try_from(width)?,
                i32::try_from(height)?,
                AVC1,
                null(),
                null(),
                null(),
                encoded,
                refcon,
                &raw mut session,
            )
        };
        anyhow::ensure!(status == 0, "creating the H.264 encoder: OSStatus {status}");
        let number = |value: i32| CFNumber::new_i32(value);
        // SAFETY: the keys and level are VideoToolbox's static CFStrings, the values CF objects.
        unsafe {
            let set = |key: CfRef, value: &CFType| {
                let status = VTSessionSetProperty(session, key, std::ptr::from_ref(value).cast());
                if status != 0 {
                    log::debug!("setting an encoder property: OSStatus {status}");
                }
            };
            set(kVTCompressionPropertyKey_RealTime, CFBoolean::new(true));
            // Phones decode without reordering; B-frames would arrive late.
            set(kVTCompressionPropertyKey_AllowFrameReordering, CFBoolean::new(false));
            set(kVTCompressionPropertyKey_ProfileLevel, &*kVTProfileLevel_H264_Main_AutoLevel.cast::<CFString>());
            set(kVTCompressionPropertyKey_MaxKeyFrameInterval, &number(FRAMES_PER_SECOND * 2));
            set(kVTCompressionPropertyKey_AverageBitRate, &number(8_000_000));
        }
        Ok(Self { session, shared })
    }

    fn encode(&self, sample: &CMSampleBuffer) {
        let sample = std::ptr::from_ref(sample).cast::<c_void>();
        // SAFETY: `sample` is ScreenCaptureKit's buffer; idle frames carry no image.
        unsafe {
            let image = CMSampleBufferGetImageBuffer(sample);
            if image.is_null() {
                return;
            }
            let pts = CMSampleBufferGetPresentationTimeStamp(sample);
            let status =
                VTCompressionSessionEncodeFrame(self.session, image, pts, INVALID_TIME, null(), null_mut(), null_mut());
            if status != 0 {
                log::debug!("encoding a screen frame: OSStatus {status}");
            }
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: the session was created in `new` and is released once.
        unsafe {
            VTCompressionSessionInvalidate(self.session);
            CFRelease(self.session);
        }
    }
}

/// Writes one encoded frame: a keyframe goes as its parameter sets (a configuration unit), then the picture, in
/// Annex B form.
unsafe extern "C-unwind" fn encoded(
    refcon: *mut c_void,
    _frame: *mut c_void,
    status: OsStatus,
    _info: u32,
    sample: CfRef,
) {
    // SAFETY: `refcon` is the encoder's `Shared`, alive while its session is.
    let shared = unsafe { &*refcon.cast::<Shared>() };
    if status != 0 || sample.is_null() || shared.failed.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: `sample` is VideoToolbox's encoded buffer.
    let (keyframe, pts_us, picture, config) = unsafe {
        let pts = CMSampleBufferGetPresentationTimeStamp(sample);
        let pts_us = if pts.timescale > 0 {
            u64::try_from(pts.value.saturating_mul(1_000_000) / i64::from(pts.timescale)).unwrap_or(0)
        } else {
            0
        };
        let keyframe = is_keyframe(sample);
        let block = CMSampleBufferGetDataBuffer(sample);
        let len = CMBlockBufferGetDataLength(block);
        let mut avcc = vec![0u8; len];
        if CMBlockBufferCopyDataBytes(block, 0, len, avcc.as_mut_ptr().cast()) != 0 {
            return;
        }
        let config = if keyframe { parameter_sets(CMSampleBufferGetFormatDescription(sample)) } else { Vec::new() };
        (keyframe, pts_us, annex_b(&avcc), config)
    };
    let result = (|| {
        if !config.is_empty() {
            write_unit(&shared.socket, pts_us, CONFIG, &config)?;
        }
        write_unit(&shared.socket, pts_us, if keyframe { KEYFRAME } else { 0 }, &picture)
    })();
    match result {
        Ok(()) => drop(shared.frames.fetch_add(1, Ordering::Relaxed)),
        Err(error) => {
            if !shared.failed.swap(true, Ordering::Relaxed) {
                log::info!("the screen socket closed: {error}");
                let _ = shared.ended.send(shared.generation);
            }
        }
    }
}

/// True unless the sample's attachments mark it not a sync sample.
unsafe fn is_keyframe(sample: CfRef) -> bool {
    // SAFETY: the attachments array holds CFDictionaries; the key is CoreMedia's static CFString.
    unsafe {
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, false);
        if attachments.is_null() || CFArrayGetCount(attachments) < 1 {
            return true;
        }
        let not_sync = CFDictionaryGetValue(CFArrayGetValueAtIndex(attachments, 0), kCMSampleAttachmentKey_NotSync);
        not_sync.is_null() || !CFBooleanGetValue(not_sync)
    }
}

/// The format's SPS and PPS, each after a 4-byte start code.
unsafe fn parameter_sets(format: CfRef) -> Vec<u8> {
    let mut config = Vec::new();
    let (mut count, mut index) = (0, 0);
    loop {
        let (mut pointer, mut size) = (null(), 0);
        // SAFETY: `format` is an H.264 format description; the outputs are plain pointers.
        let status = unsafe {
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                format,
                index,
                &raw mut pointer,
                &raw mut size,
                &raw mut count,
                null_mut(),
            )
        };
        if status != 0 || pointer.is_null() {
            break;
        }
        config.extend_from_slice(&[0, 0, 0, 1]);
        // SAFETY: CoreMedia gives `size` bytes at `pointer`, alive with the format.
        config.extend_from_slice(unsafe { std::slice::from_raw_parts(pointer, size) });
        index += 1;
        if index >= count {
            break;
        }
    }
    config
}

/// AVCC (4-byte big-endian lengths) to Annex B (4-byte start codes).
fn annex_b(avcc: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(avcc.len());
    let mut rest = avcc;
    while let [a, b, c, d, tail @ ..] = rest {
        let len = u32::from_be_bytes([*a, *b, *c, *d]) as usize;
        let Some((nal, next)) = tail.split_at_checked(len) else { break };
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal);
        rest = next;
    }
    out
}

fn write_unit(socket: &Mutex<UnixStream>, pts_us: u64, flags: u8, data: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(data.len()).map_err(std::io::Error::other)?;
    let mut header = [0; 13];
    header[..4].copy_from_slice(&len.to_be_bytes());
    header[4..12].copy_from_slice(&pts_us.to_be_bytes());
    header[12] = flags;
    let mut socket = socket.lock().map_err(|_| std::io::Error::other("a writer panicked"))?;
    socket.write_all(&header)?;
    socket.write_all(data)
}

/// The encoded size: the display's aspect, the longer side at most [`MAX_SIDE`], even sides.
fn fit(width: isize, height: isize) -> (usize, usize) {
    let (width, height) = (width.unsigned_abs().max(2), height.unsigned_abs().max(2));
    let longer = width.max(height);
    let even = |side: usize| (if longer > MAX_SIDE { side * MAX_SIDE / longer } else { side } / 2 * 2).max(2);
    (even(width), even(height))
}

type OsStatus = i32;
type CfRef = *const c_void;

/// `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange`, `420v`.
const NV12: u32 = 0x3432_3076;
/// `kCMVideoCodecType_H264`, `avc1`.
const AVC1: u32 = 0x6176_6331;
const INVALID_TIME: CMTime = CMTime { value: 0, timescale: 0, flags: CMTimeFlags(0), epoch: 0 };

type EncodedCallback = unsafe extern "C-unwind" fn(*mut c_void, *mut c_void, OsStatus, u32, CfRef);

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    static kVTCompressionPropertyKey_RealTime: CfRef;
    static kVTCompressionPropertyKey_AllowFrameReordering: CfRef;
    static kVTCompressionPropertyKey_ProfileLevel: CfRef;
    static kVTCompressionPropertyKey_MaxKeyFrameInterval: CfRef;
    static kVTCompressionPropertyKey_AverageBitRate: CfRef;
    static kVTProfileLevel_H264_Main_AutoLevel: CfRef;
    fn VTCompressionSessionCreate(
        allocator: CfRef,
        width: i32,
        height: i32,
        codec: u32,
        encoder_specification: CfRef,
        source_attributes: CfRef,
        compressed_allocator: CfRef,
        callback: EncodedCallback,
        refcon: *mut c_void,
        out: *mut CfRef,
    ) -> OsStatus;
    fn VTSessionSetProperty(session: CfRef, key: CfRef, value: CfRef) -> OsStatus;
    fn VTCompressionSessionEncodeFrame(
        session: CfRef,
        image: CfRef,
        pts: CMTime,
        duration: CMTime,
        properties: CfRef,
        frame_refcon: *mut c_void,
        info: *mut u32,
    ) -> OsStatus;
    fn VTCompressionSessionInvalidate(session: CfRef);
}

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    static kCMSampleAttachmentKey_NotSync: CfRef;
    fn CMSampleBufferGetImageBuffer(sample: CfRef) -> CfRef;
    fn CMSampleBufferGetPresentationTimeStamp(sample: CfRef) -> CMTime;
    fn CMSampleBufferGetDataBuffer(sample: CfRef) -> CfRef;
    fn CMSampleBufferGetFormatDescription(sample: CfRef) -> CfRef;
    fn CMSampleBufferGetSampleAttachmentsArray(sample: CfRef, create: bool) -> CfRef;
    fn CMBlockBufferGetDataLength(block: CfRef) -> usize;
    fn CMBlockBufferCopyDataBytes(block: CfRef, offset: usize, length: usize, destination: *mut c_void) -> OsStatus;
    fn CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
        format: CfRef,
        index: usize,
        pointer: *mut *const u8,
        size: *mut usize,
        count: *mut usize,
        nal_header_length: *mut i32,
    ) -> OsStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: CfRef);
    fn CFArrayGetCount(array: CfRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CfRef, index: isize) -> CfRef;
    fn CFDictionaryGetValue(dictionary: CfRef, key: CfRef) -> CfRef;
    fn CFBooleanGetValue(boolean: CfRef) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avcc_lengths_become_start_codes() {
        let avcc = [0, 0, 0, 2, 0x65, 0xaa, 0, 0, 0, 1, 0x41];
        assert_eq!(annex_b(&avcc), [0, 0, 0, 1, 0x65, 0xaa, 0, 0, 0, 1, 0x41]);
    }
}

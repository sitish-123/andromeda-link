//! The daemon's mirror socket decoded by `VideoToolbox` into NV12 pictures for the Screen page, and the phone's Opus
//! sound decoded by `AudioToolbox` to the speakers.

#![allow(unsafe_code, reason = "FFI to VideoToolbox, CoreMedia, CoreVideo and AudioToolbox")]

use std::ffi::c_void;
use std::os::unix::net::UnixStream;
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex};

use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString};

use crate::mirror_unit::{AUDIO, END, read_unit};

/// One decoded picture: `height` rows of luma, `stride` bytes each, then the chroma rows.
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub data: Vec<u8>,
}

pub struct Stream {
    socket: UnixStream,
}

impl Stream {
    /// Decodes `socket` until it ends. `frame` gets each picture on the reading thread, `ended` runs once after the
    /// socket closes, with whether it broke instead of ending.
    pub fn start(
        socket: UnixStream,
        frame: impl Fn(Frame) + Send + Sync + 'static,
        ended: impl FnOnce(bool) + Send + 'static,
    ) -> Result<Self, String> {
        let reader = socket.try_clone().map_err(|error| error.to_string())?;
        let frame: Arc<FrameFn> = Arc::new(frame);
        std::thread::spawn(move || {
            ended(feed(reader, &frame));
        });
        Ok(Self { socket })
    }
}

impl Drop for Stream {
    /// Closing the socket tells the daemon and then the phone to stop.
    fn drop(&mut self) {
        if let Err(error) = self.socket.shutdown(std::net::Shutdown::Both) {
            log::debug!("closing the mirror socket: {error}");
        }
    }
}

type FrameFn = dyn Fn(Frame) + Send + Sync;

/// Whether the stream broke.
fn feed(mut socket: UnixStream, frame: &Arc<FrameFn>) -> bool {
    let mut decoder: Option<Decoder> = None;
    let mut sound: Option<Option<Speaker>> = None;
    loop {
        let (pts_us, flags, data) = match read_unit(&mut socket) {
            Ok(Some((_, flags, _))) if flags & END != 0 => return false,
            Ok(Some(unit)) => unit,
            Ok(None) => return true,
            Err(error) => {
                log::info!("reading the mirror stream: {error}");
                return true;
            }
        };
        if flags & AUDIO != 0 {
            // Built at the first packet: phones without playback capture send none.
            if let Some(speaker) = sound.get_or_insert_with(Speaker::new) {
                speaker.play(&data);
            }
            continue;
        }
        let units = nal_units(&data);
        let sps = units.iter().find(|unit| unit.first().is_some_and(|b| b & 0x1f == 7));
        let pps = units.iter().find(|unit| unit.first().is_some_and(|b| b & 0x1f == 8));
        // New parameter sets come with each keyframe; a rotated phone sends different ones.
        if let (Some(sps), Some(pps)) = (sps, pps)
            && decoder.as_ref().is_none_or(|decoder| decoder.sps != *sps || decoder.pps != *pps)
        {
            decoder = match Decoder::new(sps, pps, frame) {
                Ok(decoder) => Some(decoder),
                Err(status) => {
                    log::warn!("starting the H.264 decoder: OSStatus {status}");
                    None
                }
            };
        }
        let Some(decoder) = decoder.as_mut() else { continue };
        // VideoToolbox takes the picture's slices with 4-byte lengths, without parameter sets or delimiters.
        let mut avcc = Vec::with_capacity(data.len() + 16);
        for unit in units.iter().filter(|unit| unit.first().is_some_and(|b| !matches!(b & 0x1f, 7..=9))) {
            avcc.extend_from_slice(&u32::try_from(unit.len()).unwrap_or(0).to_be_bytes());
            avcc.extend_from_slice(unit);
        }
        if !avcc.is_empty()
            && let Err(status) = decoder.decode(&avcc, pts_us)
        {
            log::debug!("decoding a mirror picture: OSStatus {status}");
        }
    }
}

/// The NAL units of an Annex B access unit, without their start codes.
fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let mut end = starts.get(n + 1).map_or(data.len(), |next| next - 3);
            // A 4-byte start code leaves a zero before the next 3-byte one.
            while end > start && data[end - 1] == 0 && n + 1 < starts.len() {
                end -= 1;
            }
            &data[start..end]
        })
        .collect()
}

type OsStatus = i32;
type CfRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CmTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}

#[repr(C)]
struct CmSampleTimingInfo {
    duration: CmTime,
    presentation: CmTime,
    decode: CmTime,
}

type OutputCallback = unsafe extern "C-unwind" fn(
    refcon: *mut c_void,
    frame_refcon: *mut c_void,
    status: OsStatus,
    info: u32,
    image: CfRef,
    pts: CmTime,
    duration: CmTime,
);

#[repr(C)]
struct OutputCallbackRecord {
    callback: OutputCallback,
    refcon: *mut c_void,
}

const INVALID_TIME: CmTime = CmTime { value: 0, timescale: 0, flags: 0, epoch: 0 };
const TIME_VALID: u32 = 1;
/// `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange`, `420v`.
const NV12: i32 = 0x3432_3076;
const LOCK_READ_ONLY: u64 = 1;

#[link(name = "CoreMedia", kind = "framework")]
unsafe extern "C" {
    fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
        allocator: CfRef,
        count: usize,
        pointers: *const *const u8,
        sizes: *const usize,
        nal_length_size: i32,
        out: *mut CfRef,
    ) -> OsStatus;
    fn CMBlockBufferCreateWithMemoryBlock(
        allocator: CfRef,
        memory: *mut c_void,
        length: usize,
        block_allocator: CfRef,
        source: *const c_void,
        offset: usize,
        data_length: usize,
        flags: u32,
        out: *mut CfRef,
    ) -> OsStatus;
    fn CMBlockBufferReplaceDataBytes(source: *const c_void, block: CfRef, offset: usize, length: usize) -> OsStatus;
    fn CMSampleBufferCreateReady(
        allocator: CfRef,
        data: CfRef,
        format: CfRef,
        samples: isize,
        timing_entries: isize,
        timing: *const CmSampleTimingInfo,
        size_entries: isize,
        sizes: *const usize,
        out: *mut CfRef,
    ) -> OsStatus;
}

#[link(name = "VideoToolbox", kind = "framework")]
unsafe extern "C" {
    fn VTDecompressionSessionCreate(
        allocator: CfRef,
        format: CfRef,
        decoder_specification: CfRef,
        image_attributes: CfRef,
        callback: *const OutputCallbackRecord,
        out: *mut CfRef,
    ) -> OsStatus;
    fn VTDecompressionSessionDecodeFrame(
        session: CfRef,
        sample: CfRef,
        flags: u32,
        frame_refcon: *mut c_void,
        info: *mut u32,
    ) -> OsStatus;
    fn VTDecompressionSessionInvalidate(session: CfRef);
}

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    static kCVPixelBufferPixelFormatTypeKey: CfRef;
    fn CVPixelBufferLockBaseAddress(buffer: CfRef, flags: u64) -> i32;
    fn CVPixelBufferUnlockBaseAddress(buffer: CfRef, flags: u64) -> i32;
    fn CVPixelBufferGetWidth(buffer: CfRef) -> usize;
    fn CVPixelBufferGetHeight(buffer: CfRef) -> usize;
    fn CVPixelBufferGetBaseAddressOfPlane(buffer: CfRef, plane: usize) -> *const u8;
    fn CVPixelBufferGetBytesPerRowOfPlane(buffer: CfRef, plane: usize) -> usize;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(object: CfRef);
}

struct Decoder {
    sps: Vec<u8>,
    pps: Vec<u8>,
    format: CfRef,
    session: CfRef,
    /// The callback's refcon points here; boxed so it stays put.
    _sink: Box<Sink>,
}

struct Sink {
    frame: Arc<FrameFn>,
}

impl Decoder {
    fn new(sps: &[u8], pps: &[u8], frame: &Arc<FrameFn>) -> Result<Self, OsStatus> {
        let mut format = null();
        let sets = [sps.as_ptr(), pps.as_ptr()];
        let sizes = [sps.len(), pps.len()];
        // SAFETY: two parameter sets with their sizes; `format` receives a retained description.
        let status = unsafe {
            CMVideoFormatDescriptionCreateFromH264ParameterSets(
                null(),
                2,
                sets.as_ptr(),
                sizes.as_ptr(),
                4,
                &raw mut format,
            )
        };
        if status != 0 {
            return Err(status);
        }
        // SAFETY: the key is CoreVideo's static CFString.
        let key: &CFString = unsafe { &*kCVPixelBufferPixelFormatTypeKey.cast::<CFString>() };
        let attributes: CFRetained<CFDictionary<CFString, CFNumber>> =
            CFDictionary::from_slices(&[key], &[&*CFNumber::new_i32(NV12)]);
        let sink = Box::new(Sink { frame: Arc::clone(frame) });
        let record = OutputCallbackRecord { callback: output, refcon: std::ptr::from_ref(&*sink).cast_mut().cast() };
        let mut session = null();
        // SAFETY: `format` is valid, the attributes are a CFDictionary, the record outlives the call.
        let status = unsafe {
            VTDecompressionSessionCreate(
                null(),
                format,
                null(),
                CFRetained::as_ptr(&attributes).as_ptr().cast(),
                &raw const record,
                &raw mut session,
            )
        };
        if status != 0 {
            // SAFETY: `format` was created above.
            unsafe { CFRelease(format) };
            return Err(status);
        }
        Ok(Self { sps: sps.to_vec(), pps: pps.to_vec(), format, session, _sink: sink })
    }

    /// Decodes one picture; without the asynchronous flag the callback runs before this returns.
    fn decode(&mut self, avcc: &[u8], pts_us: u64) -> Result<(), OsStatus> {
        let mut block = null();
        // SAFETY: a null memory block makes CoreMedia allocate `avcc.len()` bytes on first use.
        let mut status = unsafe {
            CMBlockBufferCreateWithMemoryBlock(
                null(),
                null_mut(),
                avcc.len(),
                null(),
                null(),
                0,
                avcc.len(),
                0,
                &raw mut block,
            )
        };
        if status == 0 {
            // SAFETY: `block` holds `avcc.len()` bytes.
            status = unsafe { CMBlockBufferReplaceDataBytes(avcc.as_ptr().cast(), block, 0, avcc.len()) };
        }
        let timing = CmSampleTimingInfo {
            duration: INVALID_TIME,
            presentation: CmTime {
                value: i64::try_from(pts_us).unwrap_or(0),
                timescale: 1_000_000,
                flags: TIME_VALID,
                epoch: 0,
            },
            decode: INVALID_TIME,
        };
        let mut sample = null();
        if status == 0 {
            let size = avcc.len();
            // SAFETY: `block` and `format` are valid; one sample of `size` bytes with one timing entry.
            status = unsafe {
                CMSampleBufferCreateReady(
                    null(),
                    block,
                    self.format,
                    1,
                    1,
                    &raw const timing,
                    1,
                    &raw const size,
                    &raw mut sample,
                )
            };
        }
        if status == 0 {
            // SAFETY: the session and sample are valid.
            status = unsafe { VTDecompressionSessionDecodeFrame(self.session, sample, 0, null_mut(), null_mut()) };
        }
        for object in [sample, block] {
            if !object.is_null() {
                // SAFETY: each was created above and is released once.
                unsafe { CFRelease(object) };
            }
        }
        if status == 0 { Ok(()) } else { Err(status) }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: both were created in `new` and are released once.
        unsafe {
            VTDecompressionSessionInvalidate(self.session);
            CFRelease(self.session);
            CFRelease(self.format);
        }
    }
}

/// Copies the decoded NV12 picture into the Screen page's packed layout: the luma stride rounded up to 4.
unsafe extern "C-unwind" fn output(
    refcon: *mut c_void,
    _frame_refcon: *mut c_void,
    status: OsStatus,
    _info: u32,
    image: CfRef,
    _pts: CmTime,
    _duration: CmTime,
) {
    if status != 0 || image.is_null() {
        return;
    }
    // SAFETY: `refcon` is the decoder's boxed `Sink`, alive while it decodes; `image` is an NV12 CVPixelBuffer.
    unsafe {
        let sink = &*refcon.cast::<Sink>();
        if CVPixelBufferLockBaseAddress(image, LOCK_READ_ONLY) != 0 {
            return;
        }
        let (width, height) = (CVPixelBufferGetWidth(image), CVPixelBufferGetHeight(image));
        let stride = (width + 3) & !3;
        let mut data = vec![0; stride * (height + height.div_ceil(2))];
        for (plane, rows, offset) in [(0, height, 0), (1, height.div_ceil(2), stride * height)] {
            let base = CVPixelBufferGetBaseAddressOfPlane(image, plane);
            let pitch = CVPixelBufferGetBytesPerRowOfPlane(image, plane);
            if base.is_null() {
                continue;
            }
            let row_len = width.min(pitch);
            for row in 0..rows {
                let from = std::slice::from_raw_parts(base.add(row * pitch), row_len);
                data[offset + row * stride..][..row_len].copy_from_slice(from);
            }
        }
        CVPixelBufferUnlockBaseAddress(image, LOCK_READ_ONLY);
        let (Ok(width), Ok(height), Ok(stride)) = (i32::try_from(width), i32::try_from(height), i32::try_from(stride))
        else {
            return;
        };
        (sink.frame)(Frame { width, height, stride, data });
    }
}

#[repr(C)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AudioStreamPacketDescription {
    start_offset: i64,
    variable_frames: u32,
    size: u32,
}

#[repr(C)]
struct AudioBuffer {
    channels: u32,
    size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    count: u32,
    buffers: [AudioBuffer; 1],
}

#[repr(C)]
struct AudioQueueBuffer {
    capacity: u32,
    data: *mut c_void,
    size: u32,
    user_data: *mut c_void,
    description_capacity: u32,
    descriptions: *mut AudioStreamPacketDescription,
    description_count: u32,
}

type QueueBuffer = *mut AudioQueueBuffer;
type InputProc = unsafe extern "C-unwind" fn(
    converter: CfRef,
    packets: *mut u32,
    data: *mut AudioBufferList,
    descriptions: *mut *mut AudioStreamPacketDescription,
    user: *mut c_void,
) -> OsStatus;
type OutputProc = unsafe extern "C-unwind" fn(user: *mut c_void, queue: CfRef, buffer: QueueBuffer);

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioConverterNew(
        from: *const AudioStreamBasicDescription,
        to: *const AudioStreamBasicDescription,
        out: *mut CfRef,
    ) -> OsStatus;
    fn AudioConverterFillComplexBuffer(
        converter: CfRef,
        input: InputProc,
        user: *mut c_void,
        packets: *mut u32,
        output: *mut AudioBufferList,
        descriptions: *mut AudioStreamPacketDescription,
    ) -> OsStatus;
    fn AudioConverterDispose(converter: CfRef) -> OsStatus;
    fn AudioQueueNewOutput(
        format: *const AudioStreamBasicDescription,
        callback: OutputProc,
        user: *mut c_void,
        run_loop: CfRef,
        mode: CfRef,
        flags: u32,
        out: *mut CfRef,
    ) -> OsStatus;
    fn AudioQueueAllocateBuffer(queue: CfRef, capacity: u32, out: *mut QueueBuffer) -> OsStatus;
    fn AudioQueueEnqueueBuffer(
        queue: CfRef,
        buffer: QueueBuffer,
        descriptions: u32,
        description: *const AudioStreamPacketDescription,
    ) -> OsStatus;
    fn AudioQueueStart(queue: CfRef, start: *const c_void) -> OsStatus;
    fn AudioQueueDispose(queue: CfRef, immediate: u8) -> OsStatus;
}

const SAMPLE_RATE: f64 = 48_000.0;
const CHANNELS: u32 = 2;
const OPUS: u32 = u32::from_be_bytes(*b"opus");
const LPCM: u32 = u32::from_be_bytes(*b"lpcm");
/// `kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked`.
const SIGNED_PACKED: u32 = 0x4 | 0x8;
/// Interleaved 16-bit stereo.
const BYTES_PER_FRAME: u32 = 4;
/// The phone sends 20 ms packets; Opus allows up to 120 ms.
const PACKET_FRAMES: u32 = 960;
const MAX_PACKET_FRAMES: u32 = 5760;
/// 200 ms of 20 ms packets, the buffer `autoaudiosink` keeps on Linux. With all of them queued a packet is dropped,
/// so the sound never falls behind the picture.
const BUFFERS: usize = 10;
/// The input callback's answer once it has handed over its one packet.
const NO_MORE_INPUT: OsStatus = i32::from_be_bytes(*b"more");

/// The phone's Opus sound to the default output, each packet queued as soon as it is decoded, like `opusdec !
/// autoaudiosink sync=false` on Linux.
struct Speaker {
    converter: CfRef,
    queue: CfRef,
    /// Buffers the queue has played, handed back on its own thread; boxed so its pointer stays put.
    free: Box<Mutex<Vec<QueueBuffer>>>,
    started: bool,
}

struct Packet {
    data: *const u8,
    description: AudioStreamPacketDescription,
    given: bool,
}

impl Speaker {
    fn new() -> Option<Self> {
        Self::open().map_err(|status| log::info!("no sound: OSStatus {status}")).ok()
    }

    fn open() -> Result<Self, OsStatus> {
        let opus = AudioStreamBasicDescription {
            sample_rate: SAMPLE_RATE,
            format: OPUS,
            format_flags: 0,
            bytes_per_packet: 0,
            frames_per_packet: PACKET_FRAMES,
            bytes_per_frame: 0,
            channels_per_frame: CHANNELS,
            bits_per_channel: 0,
            reserved: 0,
        };
        let pcm = AudioStreamBasicDescription {
            sample_rate: SAMPLE_RATE,
            format: LPCM,
            format_flags: SIGNED_PACKED,
            bytes_per_packet: BYTES_PER_FRAME,
            frames_per_packet: 1,
            bytes_per_frame: BYTES_PER_FRAME,
            channels_per_frame: CHANNELS,
            bits_per_channel: 16,
            reserved: 0,
        };
        let mut converter = null();
        // SAFETY: both descriptions are complete; `converter` receives a new converter.
        let status = unsafe { AudioConverterNew(&raw const opus, &raw const pcm, &raw mut converter) };
        if status != 0 {
            return Err(status);
        }
        let mut speaker = Self { converter, queue: null(), free: Box::new(Mutex::new(Vec::new())), started: false };
        let user = std::ptr::from_ref(&*speaker.free).cast_mut().cast();
        // SAFETY: a null run loop calls `played` on the queue's own thread; `free` outlives the queue, which `drop`
        // disposes first.
        let status =
            unsafe { AudioQueueNewOutput(&raw const pcm, played, user, null(), null(), 0, &raw mut speaker.queue) };
        if status != 0 {
            return Err(status);
        }
        for _ in 0..BUFFERS {
            let mut buffer = null_mut();
            // SAFETY: the queue is valid; `buffer` receives a buffer the queue owns.
            let status = unsafe {
                AudioQueueAllocateBuffer(speaker.queue, MAX_PACKET_FRAMES * BYTES_PER_FRAME, &raw mut buffer)
            };
            if status != 0 {
                return Err(status);
            }
            speaker.free.lock().map_err(|_| -1)?.push(buffer);
        }
        Ok(speaker)
    }

    /// Decodes one Opus packet into a free buffer and queues it.
    fn play(&mut self, packet: &[u8]) {
        let Some(buffer) = self.free.lock().ok().and_then(|mut free| free.pop()) else {
            return log::debug!("the speakers are 200 ms behind; dropping a sound packet");
        };
        let Ok(size) = u32::try_from(packet.len()) else { return self.give_back(buffer) };
        let mut input = Packet {
            data: packet.as_ptr(),
            description: AudioStreamPacketDescription { start_offset: 0, variable_frames: 0, size },
            given: false,
        };
        // SAFETY: the buffer belongs to this queue and is not queued.
        let (data, capacity) = unsafe { ((*buffer).data, (*buffer).capacity) };
        let mut output =
            AudioBufferList { count: 1, buffers: [AudioBuffer { channels: CHANNELS, size: capacity, data }] };
        let mut frames = capacity / BYTES_PER_FRAME;
        // SAFETY: `supply` hands over `input`'s one packet, which outlives the call; `output` holds `frames` frames.
        let status = unsafe {
            AudioConverterFillComplexBuffer(
                self.converter,
                supply,
                (&raw mut input).cast(),
                &raw mut frames,
                &raw mut output,
                null_mut(),
            )
        };
        if (status != 0 && status != NO_MORE_INPUT) || frames == 0 {
            if status != NO_MORE_INPUT {
                log::debug!("decoding a sound packet: OSStatus {status}");
            }
            return self.give_back(buffer);
        }
        // SAFETY: as above; the converter wrote `frames` frames.
        unsafe { (*buffer).size = frames * BYTES_PER_FRAME };
        // SAFETY: the buffer belongs to this queue and holds PCM in its format.
        let mut status = unsafe { AudioQueueEnqueueBuffer(self.queue, buffer, 0, null()) };
        if status == 0 && !self.started {
            // SAFETY: the queue is valid; a null time starts it at once.
            status = unsafe { AudioQueueStart(self.queue, null()) };
            self.started = status == 0;
        }
        if status != 0 {
            log::debug!("playing a sound packet: OSStatus {status}");
        }
    }

    fn give_back(&self, buffer: QueueBuffer) {
        if let Ok(mut free) = self.free.lock() {
            free.push(buffer);
        }
    }
}

impl Drop for Speaker {
    fn drop(&mut self) {
        // SAFETY: both were created in `open` and are disposed once; disposing the queue frees its buffers and stops
        // its callbacks before `free` drops.
        unsafe {
            if !self.queue.is_null() {
                AudioQueueDispose(self.queue, 1);
            }
            AudioConverterDispose(self.converter);
        }
    }
}

/// Hands the converter the one packet, then reports that no more input is ready.
unsafe extern "C-unwind" fn supply(
    _converter: CfRef,
    packets: *mut u32,
    data: *mut AudioBufferList,
    descriptions: *mut *mut AudioStreamPacketDescription,
    user: *mut c_void,
) -> OsStatus {
    // SAFETY: `user` is `play`'s `Packet`; the converter passes valid out-pointers.
    unsafe {
        let input = &mut *user.cast::<Packet>();
        if std::mem::replace(&mut input.given, true) {
            *packets = 0;
            return NO_MORE_INPUT;
        }
        *packets = 1;
        (*data).buffers[0] =
            AudioBuffer { channels: CHANNELS, size: input.description.size, data: input.data.cast_mut().cast() };
        if !descriptions.is_null() {
            *descriptions = &raw mut input.description;
        }
    }
    0
}

/// Takes a played buffer back for the next packet.
unsafe extern "C-unwind" fn played(user: *mut c_void, _queue: CfRef, buffer: QueueBuffer) {
    // SAFETY: `user` is the speaker's boxed free list, alive until the queue is disposed.
    let free = unsafe { &*user.cast::<Mutex<Vec<QueueBuffer>>>() };
    if let Ok(mut free) = free.lock() {
        free.push(buffer);
    }
}

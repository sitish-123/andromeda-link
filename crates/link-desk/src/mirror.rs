//! The daemon's mirror socket decoded into NV12 pictures for the Screen page, and the phone's sound to the speakers.
//! The socket's framing: a big-endian u32 length and u64 presentation time in microseconds, a flags byte, then H.264 in
//! Annex B form, or with the audio flag one Opus packet of 48 kHz stereo.

use std::io::Read as _;
use std::os::unix::net::UnixStream;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

const UNIT_HEADER: usize = 13;
const MAX_UNIT: usize = 4 * 1024 * 1024;
const KEYFRAME: u8 = 1;
const AUDIO: u8 = 4;
const CAPS: &str = "video/x-h264,stream-format=byte-stream,alignment=au";
const AUDIO_CAPS: &str = "audio/x-opus,channel-mapping-family=(int)0,channels=(int)2,rate=(int)48000";

/// One decoded picture: `height` rows of luma, `stride` bytes each, then the chroma rows.
pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub data: gst::MappedBuffer<gst::buffer::Readable>,
}

pub struct Stream {
    pipeline: gst::Pipeline,
    socket: UnixStream,
}

impl Stream {
    /// Decodes `socket` until it ends: VA-API when the GPU has it. `frame` gets each picture on a streaming thread,
    /// `ended` runs once after the socket closes.
    pub fn start(
        socket: UnixStream,
        frame: impl Fn(Frame) + Send + Sync + 'static,
        ended: impl FnOnce() + Send + 'static,
    ) -> Result<Self, String> {
        gst::init().map_err(|error| error.to_string())?;
        let decoder =
            if gst::ElementFactory::find("vah264dec").is_some() { "vah264dec" } else { "avdec_h264 ! videoconvert" };
        let (pipeline, src) = build(&format!(
            "appsrc name=src is-live=true format=time caps={CAPS} ! h264parse ! {decoder} ! video/x-raw,format=NV12 \
             ! appsink name=sink sync=false max-buffers=1 drop=true"
        ))?;
        let sink = pipeline
            .by_name("sink")
            .and_then(|sink| sink.downcast::<gst_app::AppSink>().ok())
            .ok_or("the pipeline has no appsink")?;
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if let Some(picture) = picture(&sample) {
                        frame(picture);
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        pipeline.set_state(gst::State::Playing).map_err(|error| error.to_string())?;
        let reader = socket.try_clone().map_err(|error| error.to_string())?;
        std::thread::spawn(move || {
            feed(reader, &src);
            ended();
        });
        Ok(Self { pipeline, socket })
    }
}

impl Drop for Stream {
    /// Closing the socket tells the daemon and then the phone to stop.
    fn drop(&mut self) {
        if let Err(error) = self.socket.shutdown(std::net::Shutdown::Both) {
            log::debug!("closing the mirror socket: {error}");
        }
        if let Err(error) = self.pipeline.set_state(gst::State::Null) {
            log::debug!("stopping the mirror pipeline: {error}");
        }
    }
}

/// The sample's NV12 picture, in the default layout: appsink takes no video meta, so the planes are packed
/// with the luma stride rounded up to 4.
fn picture(sample: &gst::Sample) -> Option<Frame> {
    let structure = sample.caps()?.structure(0)?;
    let (width, height) = (structure.get::<i32>("width").ok()?, structure.get::<i32>("height").ok()?);
    let stride = (width + 3) & !3;
    let data = sample.buffer_owned()?.into_mapped_buffer_readable().ok()?;
    let needed = usize::try_from(stride * (height + (height + 1) / 2)).ok()?;
    (data.len() >= needed).then_some(Frame { width, height, stride, data })
}

fn read_unit(socket: &mut UnixStream) -> std::io::Result<Option<(u64, u8, Vec<u8>)>> {
    let mut header = [0; UNIT_HEADER];
    match socket.read_exact(&mut header) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let [a, b, c, d, pts @ .., flags] = header;
    let len = u32::from_be_bytes([a, b, c, d]) as usize;
    if len > MAX_UNIT {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "an access unit over 4 MiB"));
    }
    let mut data = vec![0; len];
    socket.read_exact(&mut data)?;
    Ok(Some((u64::from_be_bytes(pts), flags, data)))
}

/// Pushes the socket's units into the video and sound pipelines until it ends.
fn feed(mut socket: UnixStream, src: &gst_app::AppSrc) {
    let mut sound: Option<Option<(gst::Pipeline, gst_app::AppSrc)>> = None;
    let mut first = None;
    loop {
        let (pts_us, flags, data) = match read_unit(&mut socket) {
            Ok(Some(unit)) => unit,
            Ok(None) => break,
            Err(error) => {
                log::info!("reading the mirror stream: {error}");
                break;
            }
        };
        let base = *first.get_or_insert(pts_us);
        let mut buffer = gst::Buffer::from_mut_slice(data);
        if let Some(buffer) = buffer.get_mut() {
            buffer.set_pts(gst::ClockTime::from_useconds(pts_us.saturating_sub(base)));
            if flags & (KEYFRAME | AUDIO) == 0 {
                buffer.set_flags(gst::BufferFlags::DELTA_UNIT);
            }
        }
        if flags & AUDIO != 0 {
            // Built at the first packet: phones without playback capture send none.
            if let Some((_, src)) = sound.get_or_insert_with(speakers)
                && src.push_buffer(buffer).is_err()
            {
                log::debug!("the sound pipeline stopped");
            }
        } else if src.push_buffer(buffer).is_err() {
            break;
        }
    }
    if let Some(Some((pipeline, _))) = sound {
        let _ = pipeline.set_state(gst::State::Null);
    }
}

fn speakers() -> Option<(gst::Pipeline, gst_app::AppSrc)> {
    let description = format!(
        "appsrc name=src is-live=true format=time caps={AUDIO_CAPS} ! opusdec ! audioconvert ! audioresample \
         ! autoaudiosink sync=false"
    );
    build(&description)
        .and_then(|(pipeline, src)| {
            pipeline.set_state(gst::State::Playing).map_err(|error| error.to_string())?;
            Ok((pipeline, src))
        })
        .map_err(|error| log::info!("no sound: {error}"))
        .ok()
}

fn build(description: &str) -> Result<(gst::Pipeline, gst_app::AppSrc), String> {
    let pipeline = gst::parse::launch(description)
        .map_err(|error| error.to_string())?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "not a pipeline")?;
    let src = pipeline
        .by_name("src")
        .and_then(|src| src.downcast::<gst_app::AppSrc>().ok())
        .ok_or("the pipeline has no appsrc")?;
    Ok((pipeline, src))
}

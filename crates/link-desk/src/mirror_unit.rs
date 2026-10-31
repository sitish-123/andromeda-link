//! The daemon's mirror socket framing: a big-endian u32 length and u64 presentation time in microseconds, a flags
//! byte, then H.264 in Annex B form, or with the audio flag one Opus packet of 48 kHz stereo.

use std::io::Read as _;
use std::os::unix::net::UnixStream;

const UNIT_HEADER: usize = 13;
const MAX_UNIT: usize = 4 * 1024 * 1024;
#[cfg_attr(target_os = "macos", allow(dead_code, reason = "VideoToolbox finds keyframes itself"))]
pub const KEYFRAME: u8 = 1;
pub const AUDIO: u8 = 4;
/// The daemon's last unit, empty, when the phone ended mirroring on purpose; a socket closing without it
/// lost the stream.
pub const END: u8 = 8;

/// The next `(pts µs, flags, data)`; None when the socket ended.
pub fn read_unit(socket: &mut UnixStream) -> std::io::Result<Option<(u64, u8, Vec<u8>)>> {
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

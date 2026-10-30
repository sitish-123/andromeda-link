//! The Screen page's mirror on platforms without the GStreamer build: a mirror reports that it cannot start.

use std::os::unix::net::UnixStream;

pub struct Frame {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub data: Vec<u8>,
}

pub struct Stream;

impl Stream {
    pub fn start(
        _socket: UnixStream,
        _frame: impl Fn(Frame) + Send + Sync + 'static,
        _ended: impl FnOnce() + Send + 'static,
    ) -> Result<Self, String> {
        Err("this build has no video decoder".into())
    }
}

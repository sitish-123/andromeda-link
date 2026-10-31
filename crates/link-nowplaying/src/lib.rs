//! What the Mac plays, for andromeda-linkd. macOS 15.4 serves its private `MediaRemote` "now playing" only to Apple's
//! own programs, so the daemon runs `/usr/bin/perl`, which loads this library and calls [`link_now_playing`] as a Perl
//! sub. It writes one JSON line per change to stdout and takes commands on stdin, one per line: `play`, `pause`,
//! `play-pause`, `next`, `previous`, or `seek <ms>`. It exits when stdin closes.

#![cfg(target_os = "macos")]

use std::ffi::{CStr, c_char, c_void};
use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2_core_foundation::{CFData, CFDate, CFDictionary, CFNumber, CFString, CFType};

const MEDIA_REMOTE: &CStr = c"/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote";
const POLL: Duration = Duration::from_secs(1);
/// A position further than this from where it should be is a seek, sent at once.
const SEEK_TOLERANCE_MS: f64 = 2000.0;

type CfRef = *const c_void;
type GetInfo = unsafe extern "C" fn(*const c_void, &block2::DynBlock<dyn Fn(CfRef)>);
type GetPlaying = unsafe extern "C" fn(*const c_void, &block2::DynBlock<dyn Fn(u8)>);
type GetClient = unsafe extern "C" fn(*const c_void, &block2::DynBlock<dyn Fn(CfRef)>);
type ClientName = unsafe extern "C" fn(CfRef) -> CfRef;
type SendCommand = unsafe extern "C" fn(u32, CfRef) -> u8;
type SetElapsed = unsafe extern "C" fn(f64);

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFDateGetAbsoluteTime(date: CfRef) -> f64;
}

struct Remote {
    info: GetInfo,
    playing: GetPlaying,
    client: Option<(GetClient, ClientName)>,
    send: SendCommand,
    seek: SetElapsed,
}

impl Remote {
    fn load() -> Option<Self> {
        // SAFETY: dlopen and dlsym with valid C strings; each symbol is cast to the type MediaRemote gives it.
        unsafe {
            let handle = dlopen(MEDIA_REMOTE.as_ptr(), 1);
            if handle.is_null() {
                return None;
            }
            let find = |name: &CStr| Some(dlsym(handle, name.as_ptr())).filter(|symbol| !symbol.is_null());
            let client = find(c"MRMediaRemoteGetNowPlayingClient").zip(find(c"MRNowPlayingClientGetDisplayName"));
            Some(Self {
                info: std::mem::transmute::<*mut c_void, GetInfo>(find(c"MRMediaRemoteGetNowPlayingInfo")?),
                playing: std::mem::transmute::<*mut c_void, GetPlaying>(find(
                    c"MRMediaRemoteGetNowPlayingApplicationIsPlaying",
                )?),
                client: client.map(|(get, name)| {
                    (
                        std::mem::transmute::<*mut c_void, GetClient>(get),
                        std::mem::transmute::<*mut c_void, ClientName>(name),
                    )
                }),
                send: std::mem::transmute::<*mut c_void, SendCommand>(find(c"MRMediaRemoteSendCommand")?),
                seek: std::mem::transmute::<*mut c_void, SetElapsed>(find(c"MRMediaRemoteSetElapsedTime")?),
            })
        }
    }

    fn command(&self, line: &str) {
        // MRMediaRemoteCommand values.
        let code = match line.split_whitespace().collect::<Vec<_>>()[..] {
            ["play"] => 0,
            ["pause"] => 1,
            ["play-pause"] => 2,
            ["next"] => 4,
            ["previous"] => 5,
            ["seek", ms] => {
                if let Ok(ms) = ms.parse::<f64>() {
                    // SAFETY: a plain number of seconds.
                    unsafe { (self.seek)(ms / 1000.0) };
                }
                return;
            }
            _ => return,
        };
        // SAFETY: a known command without options.
        unsafe { (self.send)(code, std::ptr::null()) };
    }
}

/// The state written as JSON; `None` while nothing plays or pauses.
#[derive(Clone, PartialEq)]
struct Playing {
    app: String,
    title: String,
    artist: String,
    album: String,
    playing: bool,
    duration_ms: f64,
    position_ms: f64,
    artwork: Option<PathBuf>,
}

/// Called by Perl as `run()`; never returns.
///
/// # Safety
/// Perl passes its interpreter and the sub's CV, which this ignores.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn link_now_playing(_perl: *mut c_void, _cv: *mut c_void) {
    let Some(remote) = Remote::load() else {
        eprintln!("MediaRemote did not load");
        std::process::exit(2);
    };
    let (commands, received) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if commands.send(line).is_err() {
                break;
            }
        }
        std::process::exit(0);
    });
    let queue = DispatchQueue::new("org.andromeda.link.nowplaying", None);
    let mut last: Option<Playing> = None;
    let mut sent_at = std::time::Instant::now();
    let mut art: Option<(u64, PathBuf)> = None;
    loop {
        while let Ok(line) = received.try_recv() {
            remote.command(&line);
        }
        let now = read(&remote, &queue, &mut art);
        if changed(last.as_ref(), now.as_ref(), sent_at.elapsed()) {
            write(now.as_ref());
            sent_at = std::time::Instant::now();
            last = now;
        }
        match received.recv_timeout(POLL) {
            Ok(line) => remote.command(&line),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => std::process::exit(0),
        }
    }
}

/// True when anything but the advancing position differs, or the position jumped.
fn changed(last: Option<&Playing>, now: Option<&Playing>, since: Duration) -> bool {
    let (Some(last), Some(now)) = (last, now) else { return last.is_some() != now.is_some() };
    let expected = last.position_ms + if last.playing { since.as_secs_f64() * 1000.0 } else { 0.0 };
    let same = Playing { position_ms: now.position_ms, ..last.clone() };
    same != *now || (now.position_ms - expected).abs() > SEEK_TOLERANCE_MS
}

fn write(now: Option<&Playing>) {
    let line = match now {
        None => serde_json::json!({}),
        Some(now) => serde_json::json!({
            "app": now.app,
            "title": now.title,
            "artist": now.artist,
            "album": now.album,
            "playing": now.playing,
            "duration_ms": now.duration_ms.max(0.0).round(),
            "position_ms": now.position_ms.max(0.0).round(),
            "artwork": now.artwork.as_ref().map(|path| path.display().to_string()),
        }),
    };
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
        std::process::exit(0);
    }
}

fn read(remote: &Remote, queue: &DispatchQueue, art: &mut Option<(u64, PathBuf)>) -> Option<Playing> {
    let queue_ptr = std::ptr::from_ref(queue).cast::<c_void>();
    let (send, receive) = mpsc::channel::<Option<Playing>>();
    let info = RcBlock::new(move |info: CfRef| {
        let _ = send.send(parse(info));
    });
    // SAFETY: a dispatch queue and a block of the type MediaRemote calls.
    unsafe { (remote.info)(queue_ptr, &info) };
    let mut now = receive.recv_timeout(POLL).ok().flatten()?;
    let (send, receive) = mpsc::channel();
    let playing = RcBlock::new(move |playing: u8| {
        let _ = send.send(playing != 0);
    });
    // SAFETY: as above.
    unsafe { (remote.playing)(queue_ptr, &playing) };
    now.playing = receive.recv_timeout(POLL).unwrap_or(now.playing);
    if let Some((get, name)) = remote.client {
        let (send, receive) = mpsc::channel();
        let client = RcBlock::new(move |client: CfRef| {
            // SAFETY: the client object is valid during the call; its display name is a CFString or null.
            let name = unsafe { (!client.is_null()).then(|| name(client)).filter(|name| !name.is_null()) };
            let name = name.map(|name| unsafe { &*name.cast::<CFString>() }.to_string());
            let _ = send.send(name);
        });
        // SAFETY: as above.
        unsafe { get(queue_ptr, &client) };
        if let Ok(Some(app)) = receive.recv_timeout(POLL) {
            now.app = app;
        }
    }
    now.artwork = now.artwork.take().and_then(|_| art.as_ref().map(|(_, path)| path.clone()));
    // The artwork came as bytes in `parse`; it is kept in a file named by its hash.
    if let Some(bytes) = take_artwork() {
        let hash = fnv(&bytes);
        if art.as_ref().is_none_or(|(old, _)| *old != hash) {
            let path = std::env::temp_dir().join(format!("andromeda-link-nowplaying-{hash:016x}"));
            if std::fs::write(&path, &bytes).is_ok()
                && let Some((_, old)) = art.replace((hash, path))
            {
                let _ = std::fs::remove_file(old);
            }
        }
        now.artwork = art.as_ref().map(|(_, path)| path.clone());
    }
    Some(now)
}

/// The last artwork `parse` saw, which runs on the dispatch queue.
static PARSED_ART: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

fn take_artwork() -> Option<Vec<u8>> {
    PARSED_ART.lock().ok()?.take()
}

/// The now-playing dictionary; None without a title.
fn parse(info: CfRef) -> Option<Playing> {
    if info.is_null() {
        return None;
    }
    // SAFETY: MediaRemote's info is a CFDictionary of CFString keys, alive during the handler.
    let info = unsafe { &*info.cast::<CFDictionary<CFString, CFType>>() };
    let value = |key: &str| info.get(&CFString::from_str(&format!("kMRMediaRemoteNowPlayingInfo{key}")));
    let text = |key| value(key).and_then(|value| value.downcast_ref::<CFString>().map(ToString::to_string));
    let number = |key| value(key).and_then(|value| value.downcast_ref::<CFNumber>().and_then(CFNumber::as_f64));
    let title = text("Title").filter(|title| !title.is_empty())?;
    let rate = number("PlaybackRate").unwrap_or(0.0);
    let mut elapsed = number("ElapsedTime").unwrap_or(0.0);
    if rate > 0.0
        && let Some(stamp) = value("Timestamp").and_then(|value| {
            value.downcast_ref::<CFDate>().map(|date| {
                // SAFETY: a CFDate.
                unsafe { CFDateGetAbsoluteTime(std::ptr::from_ref(date).cast()) }
            })
        })
    {
        // SAFETY: no arguments.
        elapsed += (unsafe { CFAbsoluteTimeGetCurrent() } - stamp) * rate;
    }
    let artwork = value("ArtworkData").and_then(|value| value.downcast_ref::<CFData>().map(|data| data.to_vec()));
    let has_art = artwork.is_some();
    if let (Some(bytes), Ok(mut slot)) = (artwork, PARSED_ART.lock()) {
        *slot = Some(bytes);
    }
    Some(Playing {
        app: "Mac".to_owned(),
        title,
        artist: text("Artist").unwrap_or_default(),
        album: text("Album").unwrap_or_default(),
        playing: rate > 0.0,
        duration_ms: number("Duration").unwrap_or(0.0) * 1000.0,
        position_ms: elapsed * 1000.0,
        artwork: has_art.then(PathBuf::new),
    })
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

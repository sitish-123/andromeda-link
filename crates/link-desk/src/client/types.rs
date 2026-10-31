use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub connected: bool,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub battery: u32,
    pub charging: bool,
    /// wifi, cellular, ethernet, none, or other.
    pub network: String,
    /// Cellular bars, 0 to 4, or -1 without service.
    pub signal: i32,
}

#[derive(Clone, Debug)]
pub struct Controls {
    /// Percent, or -1.
    pub media_volume: i32,
    /// normal, vibrate, silent, or empty.
    pub ringer: String,
    /// 1, 0, or -1 when the phone does not say.
    pub dnd: i32,
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub device: String,
    pub id: String,
    pub app: String,
    pub title: String,
    pub text: String,
    pub icon: Option<PathBuf>,
    /// (id, label, takes text)
    pub actions: Vec<(String, String, bool)>,
}

#[derive(Clone, Debug, Default)]
pub struct Player {
    /// The phone app, from the MPRIS Identity "<app> on <phone>".
    pub app: String,
    pub title: String,
    pub artist: String,
    /// A file:// URL the service wrote.
    pub art: String,
    pub playing: bool,
    /// Microseconds, as MPRIS gives them.
    pub position: i64,
    pub length: i64,
    /// 0.0 to 1.0.
    pub volume: f64,
}

#[derive(Clone, Debug)]
pub struct Text {
    pub thread: u64,
    pub address: String,
    pub body: String,
    /// Unix milliseconds.
    pub date: u64,
    pub sent: bool,
}

#[derive(Clone, Debug)]
pub struct LoggedCall {
    pub number: String,
    /// Empty when the log has none.
    pub name: String,
    /// Unix milliseconds.
    pub date: u64,
    /// Seconds.
    pub duration: u32,
    /// incoming, outgoing, or missed.
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub folder: bool,
    pub size: u64,
    /// Unix seconds.
    pub mtime: u64,
}

/// What a phone file is copied to this computer for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetch {
    /// Into the Downloads folder.
    Save,
    /// Into the cache, for the clipboard.
    Copy,
    /// Into the cache, for the default app.
    Open,
    /// A photo the phone's camera took, into Pictures/Phone, then the default app.
    Photo,
}

#[derive(Clone, Debug)]
pub enum Event {
    /// The service is reachable (true) or gone (false).
    Service(bool),
    Devices(Vec<Device>),
    Status(HashMap<String, Status>),
    /// Device id to (last seen unix seconds, wifi, bluetooth, or empty).
    Info(HashMap<String, (i64, String)>),
    Controls(HashMap<String, Controls>),
    Notification(Notification),
    NotificationRemoved {
        device: String,
        id: String,
    },
    /// The phone offers its hotspot; joining takes this computer off its Wi-Fi. An empty `ssid` withdraws it.
    HotspotAsk {
        device: String,
        ssid: String,
    },
    /// Files the device offers, as (name, size); they wait for consent unless it is auto accepted.
    Offered {
        transfer: String,
        device: String,
        files: Vec<(String, u64)>,
    },
    Sending {
        transfer: String,
        device: String,
        files: Vec<(String, u64)>,
    },
    Progress {
        transfer: String,
        bytes: u64,
        total: u64,
    },
    Finished {
        transfer: String,
        status: String,
        paths: Vec<String>,
    },
    Received {
        device: String,
        text: String,
    },
    /// The phone's player changed; None when it has none.
    Player {
        device: String,
        player: Option<Player>,
    },
    ClipSent(String),
    /// Device id to the features it holds.
    Grants(HashMap<String, Vec<String>>),
    AutoAccept(Vec<String>),
    LocalSend(bool),
    QuickShare(bool),
    NotifyLocked(bool),
    NotificationsOff(Vec<String>),
    /// What link-session can do now; empty when it does not run.
    SessionCapabilities(Vec<String>),
    /// (name, command line)
    Commands(Vec<(String, String)>),
    /// Device id to its texts, newest first.
    Texts(HashMap<String, Vec<Text>>),
    /// Device id to its contacts as (name, numbers).
    Contacts(HashMap<String, Vec<(String, Vec<String>)>>),
    TextReceived {
        device: String,
        address: String,
    },
    /// Device id to its calls, newest first.
    Calls(HashMap<String, Vec<LoggedCall>>),
    /// Device id to why its phone refused the call log.
    CallsRefused(HashMap<String, String>),
    /// Files copied into a folder on the phone, or one deleted from it: the folder changed.
    PhoneChanged {
        device: String,
        folder: String,
        /// The names copied in; empty for a delete.
        copied: Vec<String>,
    },
    /// A folder on the phone, or the phone's reason it could not list it.
    Listed {
        device: String,
        path: String,
        entries: Result<Vec<Entry>, String>,
    },
    /// The mirror's stream socket and video size, or why the phone did not start.
    MirrorOpened {
        device: String,
        opened: Result<(Arc<OwnedFd>, u32, u32), String>,
    },
    /// A PNG thumbnail of a phone file, written to the cache; None when the phone gave none.
    Thumbnail {
        device: String,
        path: String,
        file: Option<PathBuf>,
    },
    Fetched {
        device: String,
        /// The page that asked: "photos" or "files".
        page: &'static str,
        purpose: Fetch,
        paths: Vec<PathBuf>,
    },
    /// A pairing window opened: the code the phone shows and the URI its QR code holds.
    Pairing {
        code: String,
        uri: String,
    },
    /// The name of the newly paired device.
    Paired(String),
    PairingFailed(String),
    /// `Link.currentPhone` changed; sent by the app, not the service.
    Phone(String),
    /// Ages shown in the lists need a refresh.
    Tick,
    Failed(String),
}

#[derive(Debug)]
pub enum Command {
    /// Sends the service's state again, for a new subscriber.
    Resync,
    Ring(String),
    Reply {
        device: String,
        id: String,
        action: String,
        text: String,
    },
    Dismiss {
        device: String,
        id: String,
    },
    SendFiles {
        device: String,
        paths: Vec<PathBuf>,
    },
    /// Copies files into a folder on the phone, each under a free name.
    CopyToPhone {
        device: String,
        folder: String,
        paths: Vec<PathBuf>,
    },
    DeleteFile {
        device: String,
        path: String,
    },
    /// Has the phone take a photo, then copies it into this folder.
    TakePhoto {
        device: String,
        folder: PathBuf,
    },
    /// Offers this text as the desktop's clipboard to the phones holding the clipboard grant.
    OfferClipboard(String),
    /// set-media-volume, set-ringer, or set-dnd.
    System {
        device: String,
        command: &'static str,
        value: u32,
    },
    Player {
        device: String,
        action: PlayerAction,
    },
    StartService,
    Accept(String),
    Decline(String),
    AnswerHotspot {
        device: String,
        allow: bool,
    },
    Cancel(String),
    SetGrant {
        device: String,
        feature: String,
        granted: bool,
    },
    SetAutoAccept {
        device: String,
        enabled: bool,
    },
    SetLocalSend(bool),
    SetQuickShare(bool),
    SetNotifyLocked(bool),
    SetCommands(Vec<(String, String)>),
    Unpair(String),
    List {
        device: String,
        path: String,
    },
    /// A thumbnail at most this many pixels on its longer side.
    Thumbnail {
        device: String,
        path: String,
        size: u16,
    },
    /// Copies phone files into this folder.
    Fetch {
        device: String,
        paths: Vec<String>,
        folder: PathBuf,
        page: &'static str,
        purpose: Fetch,
    },
    SendText {
        device: String,
        to: String,
        body: String,
    },
    Dial {
        device: String,
        number: String,
    },
    Pair,
    CancelPairing,
    /// Asks the phone to mirror its screen; the answer comes as [`Event::MirrorOpened`].
    OpenMirror(String),
    /// Input on the mirrored screen, as `MirrorInput` takes it: x, y, x2, y2 in 1/10000 of the screen.
    MirrorInput {
        device: String,
        action: &'static str,
        points: Vec<(&'static str, i32)>,
        ms: Option<u32>,
        text: Option<String>,
    },
    MirrorKeyframe(String),
    /// Shows a local file selected in the file manager.
    ShowInFolder(PathBuf),
}

#[derive(Debug)]
pub enum PlayerAction {
    PlayPause,
    Next,
    Previous,
    /// Microseconds.
    SetPosition(i64),
    /// 0.0 to 1.0.
    SetVolume(f64),
}

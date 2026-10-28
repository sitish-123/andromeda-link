//! The clipboard through X11's CLIPBOARD selection, watched with `XFixes`. A thread owns the connection, since x11rb's
//! calls block.

use std::collections::HashMap;
use std::io::{PipeReader, PipeWriter, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Context;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT,
    SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

use super::{Event, MAX_BYTES, REMOTE_MIME, choose, offered};

/// A selection's owner, or a phone, has this long to deliver.
const TIMEOUT: Duration = Duration::from_secs(10);
/// X11 names for UTF-8 text that phones know as `text/plain;charset=utf-8`.
const X_TEXT: [&str; 3] = ["UTF8_STRING", "TEXT", "STRING"];
const UTF8_MIME: &str = "text/plain;charset=utf-8";

pub struct X11 {
    events: UnboundedReceiver<Event>,
    commands: mpsc::Sender<Vec<String>>,
    wake: PipeWriter,
}

impl X11 {
    /// `None` outside an X11 session; under Xwayland it would see only X11 clients' copies.
    pub fn connect() -> anyhow::Result<Option<Self>> {
        if crate::on_wayland() || std::env::var_os("DISPLAY").is_none() {
            return Ok(None);
        }
        let (conn, screen) = RustConnection::connect(None).context("connecting to the X server")?;
        conn.xfixes_query_version(5, 0)?.reply().context("the X server has no XFixes")?;
        let (events, receiver) = unbounded_channel();
        let (commands, command_receiver) = mpsc::channel();
        let (wake_reader, wake) = std::io::pipe()?;
        let mut worker = Worker::new(conn, screen, events, command_receiver, wake_reader)?;
        std::thread::Builder::new().name("x11-clipboard".to_owned()).spawn(move || {
            if let Err(error) = worker.run() {
                log::warn!("the X11 clipboard stopped: {error:#}");
            }
        })?;
        Ok(Some(Self { events: receiver, commands, wake }))
    }

    pub fn offer_remote(&mut self, mimes: &[String]) -> anyhow::Result<()> {
        self.commands.send(mimes.to_vec())?;
        self.wake.write_all(&[0])?;
        Ok(())
    }

    pub async fn next(&mut self) -> anyhow::Result<Event> {
        self.events.recv().await.context("the X11 clipboard thread ended")
    }
}

enum Reading {
    Idle,
    Targets,
    Data { mime: String, mimes: Vec<String> },
    Incremental { mime: String, mimes: Vec<String>, data: Vec<u8> },
}

struct Worker {
    conn: RustConnection,
    window: Window,
    atoms: HashMap<String, Atom>,
    names: HashMap<Atom, String>,
    events: UnboundedSender<Event>,
    commands: mpsc::Receiver<Vec<String>>,
    wake: PipeReader,
    /// The phone's types while the selection serves its clip.
    owned: Option<Vec<String>>,
    read: Reading,
}

impl Worker {
    fn new(
        conn: RustConnection,
        screen: usize,
        events: UnboundedSender<Event>,
        commands: mpsc::Receiver<Vec<String>>,
        wake: PipeReader,
    ) -> anyhow::Result<Self> {
        let root = conn.setup().roots[screen].root;
        let window = conn.generate_id()?;
        let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        conn.create_window(0, window, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &aux)?;
        let mut worker = Self {
            conn,
            window,
            atoms: HashMap::new(),
            names: HashMap::new(),
            events,
            commands,
            wake,
            owned: None,
            read: Reading::Idle,
        };
        let clipboard = worker.atom("CLIPBOARD")?;
        worker.conn.xfixes_select_selection_input(window, clipboard, SelectionEventMask::SET_SELECTION_OWNER)?;
        worker.conn.flush()?;
        Ok(worker)
    }

    fn run(&mut self) -> anyhow::Result<()> {
        loop {
            while let Some(event) = self.conn.poll_for_event()? {
                self.handle(&event)?;
            }
            self.conn.flush()?;
            let mut fds = [PollFd::new(self.conn.stream(), PollFlags::IN), PollFd::new(&self.wake, PollFlags::IN)];
            poll(&mut fds, None)?;
            if fds[1].revents().contains(PollFlags::IN) {
                self.wake.read_exact(&mut [0])?;
                while let Ok(mimes) = self.commands.try_recv() {
                    self.own(mimes)?;
                }
            }
            if self.events.is_closed() {
                return Ok(());
            }
        }
    }

    fn own(&mut self, mimes: Vec<String>) -> anyhow::Result<()> {
        let clipboard = self.atom("CLIPBOARD")?;
        self.conn.set_selection_owner(self.window, clipboard, CURRENT_TIME)?;
        self.owned = Some(mimes);
        Ok(())
    }

    fn handle(&mut self, event: &XEvent) -> anyhow::Result<()> {
        let clipboard = self.atom("CLIPBOARD")?;
        let property = self.atom("LINK_SELECTION")?;
        match event {
            XEvent::XfixesSelectionNotify(notify) if notify.selection == clipboard && notify.owner != self.window => {
                self.owned = None;
                let targets = self.atom("TARGETS")?;
                self.conn.convert_selection(self.window, clipboard, targets, property, CURRENT_TIME)?;
                self.read = Reading::Targets;
            }
            XEvent::SelectionNotify(notify) if notify.requestor == self.window => {
                if notify.property == NONE {
                    self.read = Reading::Idle;
                    return Ok(());
                }
                self.selection_arrived(clipboard, property)?;
            }
            XEvent::PropertyNotify(notify)
                if notify.window == self.window && notify.atom == property && notify.state == Property::NEW_VALUE =>
            {
                if matches!(self.read, Reading::Incremental { .. }) {
                    self.chunk_arrived(property)?;
                }
            }
            XEvent::SelectionRequest(request) => self.serve(request)?,
            XEvent::SelectionClear(clear) if clear.selection == clipboard => self.owned = None,
            _ => {}
        }
        Ok(())
    }

    fn selection_arrived(&mut self, clipboard: Atom, property: Atom) -> anyhow::Result<()> {
        let reply = self.conn.get_property(true, self.window, property, AtomEnum::ANY, 0, u32::MAX / 4)?.reply()?;
        let incr = self.atom("INCR")?;
        match std::mem::replace(&mut self.read, Reading::Idle) {
            Reading::Targets => {
                let names: Vec<String> = match reply.value32() {
                    Some(atoms) => atoms.filter_map(|atom| self.name(atom).ok()).collect(),
                    None => Vec::new(),
                };
                if let Some(mime) = choose(&names).map(str::to_owned) {
                    let target = self.atom(&mime)?;
                    self.conn.convert_selection(self.window, clipboard, target, property, CURRENT_TIME)?;
                    self.read = Reading::Data { mime, mimes: names };
                }
            }
            Reading::Data { mime, mimes } if reply.type_ == incr => {
                self.read = Reading::Incremental { mime, mimes, data: Vec::new() };
            }
            Reading::Data { mime, mimes } => self.deliver(&mime, &mimes, reply.value),
            other => self.read = other,
        }
        Ok(())
    }

    fn chunk_arrived(&mut self, property: Atom) -> anyhow::Result<()> {
        let reply = self.conn.get_property(true, self.window, property, AtomEnum::ANY, 0, u32::MAX / 4)?.reply()?;
        let Reading::Incremental { mime, mimes, mut data } = std::mem::replace(&mut self.read, Reading::Idle) else {
            return Ok(());
        };
        if reply.value.is_empty() {
            self.deliver(&mime, &mimes, data);
        } else if data.len() + reply.value.len() <= usize::try_from(MAX_BYTES)? {
            data.extend_from_slice(&reply.value);
            self.read = Reading::Incremental { mime, mimes, data };
        } else {
            log::warn!("discarding an oversized X11 selection");
        }
        Ok(())
    }

    fn deliver(&self, mime: &str, mimes: &[String], data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        // Phones know X11's text targets by their MIME type.
        let phone_mimes: Vec<String> = mimes
            .iter()
            .map(|name| if X_TEXT.contains(&name.as_str()) { UTF8_MIME.to_owned() } else { name.clone() })
            .collect();
        let mime = if X_TEXT.contains(&mime) { UTF8_MIME } else { mime };
        let mut offered = offered(&phone_mimes, mime);
        offered.dedup();
        let _ = self.events.send(Event::Local { mimes: offered, data });
    }

    /// Answers a paste of the phone's clip, pulling its bytes through the main loop.
    fn serve(&mut self, request: &SelectionRequestEvent) -> anyhow::Result<()> {
        let property = if request.property == NONE { request.target } else { request.property };
        let answered = match self.owned.clone() {
            Some(mimes) if request.selection == self.atom("CLIPBOARD")? => self.answer(request, property, &mimes)?,
            _ => false,
        };
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: if answered { property } else { NONE },
        };
        self.conn.send_event(false, request.requestor, EventMask::NO_EVENT, notify)?;
        self.conn.flush()?;
        Ok(())
    }

    fn answer(&mut self, request: &SelectionRequestEvent, property: Atom, mimes: &[String]) -> anyhow::Result<bool> {
        let target = self.name(request.target)?;
        let text = mimes.iter().find(|mime| mime.starts_with("text/plain")).cloned();
        if target == "TARGETS" {
            let mut atoms = vec![self.atom("TARGETS")?];
            for mime in mimes.iter().map(String::as_str).chain([REMOTE_MIME]) {
                atoms.push(self.atom(mime)?);
            }
            if text.is_some() {
                for name in X_TEXT {
                    atoms.push(self.atom(name)?);
                }
            }
            self.conn.change_property32(PropMode::REPLACE, request.requestor, property, AtomEnum::ATOM, &atoms)?;
            return Ok(true);
        }
        let mime = if X_TEXT.contains(&target.as_str()) { text } else { mimes.iter().find(|m| **m == target).cloned() };
        let Some(mime) = mime else { return Ok(false) };
        let (reader, writer) = std::io::pipe()?;
        let (done, pulled) = tokio::sync::oneshot::channel();
        if self.events.send(Event::Paste { mime, sink: OwnedFd::from(writer), done: Some(done) }).is_err() {
            return Ok(false);
        }
        let data = match read_until(reader, TIMEOUT) {
            // A failed pull also closes the pipe, so only its outcome tells it from an empty clip.
            Ok(data) if !data.is_empty() && pulled.blocking_recv().unwrap_or(false) => data,
            Ok(_) => return Ok(false),
            Err(error) => {
                log::warn!("pulling the phone's clip for an X11 paste: {error}");
                return Ok(false);
            }
        };
        // Pastes larger than one request would need INCR; phones' text and screenshots fit.
        if data.len() + 64 > self.conn.maximum_request_bytes() {
            log::warn!("the phone's clip of {} bytes is too large for one X11 property", data.len());
            return Ok(false);
        }
        let kind = request.target;
        self.conn.change_property8(PropMode::REPLACE, request.requestor, property, kind, &data)?;
        Ok(true)
    }

    fn atom(&mut self, name: &str) -> anyhow::Result<Atom> {
        if let Some(atom) = self.atoms.get(name) {
            return Ok(*atom);
        }
        let atom = self.conn.intern_atom(false, name.as_bytes())?.reply()?.atom;
        self.atoms.insert(name.to_owned(), atom);
        self.names.insert(atom, name.to_owned());
        Ok(atom)
    }

    fn name(&mut self, atom: Atom) -> anyhow::Result<String> {
        if let Some(name) = self.names.get(&atom) {
            return Ok(name.clone());
        }
        let name = String::from_utf8(self.conn.get_atom_name(atom)?.reply()?.name)?;
        self.atoms.insert(name.clone(), atom);
        self.names.insert(atom, name.clone());
        Ok(name)
    }
}

/// Reads `reader` to its end, at most `MAX_BYTES`, failing after `timeout`.
fn read_until(mut reader: PipeReader, timeout: Duration) -> std::io::Result<Vec<u8>> {
    rustix::io::ioctl_fionbio(&reader, true)?;
    let deadline = Instant::now() + timeout;
    let mut data = Vec::new();
    let mut buffer = vec![0u8; 65536];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(data),
            Ok(read) => {
                data.extend_from_slice(&buffer[..read]);
                if data.len() as u64 > MAX_BYTES {
                    return Err(std::io::Error::other("the clip is over the size limit"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                let left = Timespec {
                    tv_sec: left.as_secs().try_into().unwrap_or(i64::MAX),
                    tv_nsec: left.subsec_nanos().into(),
                };
                poll(&mut [PollFd::new(&reader.as_fd(), PollFlags::IN)], Some(&left))?;
            }
            Err(error) => return Err(error),
        }
    }
}

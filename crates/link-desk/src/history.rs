//! Transfers and clips since the app started, newest last; the service keeps no history.

use crate::client::{Event, Fetch};
use crate::format::{file_kind, now};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Received,
    Sent,
    CopiedFrom,
    CopiedTo,
    Saved,
}

#[derive(Clone, PartialEq, Eq)]
pub enum State {
    /// Waits for the user's consent.
    Offered,
    Active,
    Done,
    /// With the service's status.
    Failed(String),
}

pub struct Item {
    /// The transfer id, with "/<n>" per file once done; a local id for clips and saves.
    pub id: String,
    /// The transfer id to answer or cancel; empty for clips and saves.
    pub transfer: String,
    pub device: String,
    pub direction: Direction,
    pub name: String,
    /// The number of files a live transfer holds.
    pub count: usize,
    pub at: u64,
    pub state: State,
    pub bytes: u64,
    pub total: u64,
    /// The local file, once done.
    pub path: String,
    /// The text of a clip.
    pub text: String,
}

impl Item {
    /// photo, file, or clip.
    pub fn kind(&self) -> &'static str {
        match (self.direction, file_kind(&self.name)) {
            (Direction::CopiedFrom | Direction::CopiedTo, _) => "clip",
            (_, "image") => "photo",
            _ => "file",
        }
    }
}

#[derive(Default)]
pub struct History {
    pub items: Vec<Item>,
    serial: u64,
}

impl History {
    fn local_id(&mut self) -> String {
        self.serial += 1;
        format!("local-{}", self.serial)
    }

    fn push(&mut self, device: &str, direction: Direction, name: String, text: String, path: String) {
        let id = self.local_id();
        let state = State::Done;
        let (transfer, device) = (String::new(), device.to_owned());
        let (count, at, bytes, total) = (1, now(), 0, 0);
        self.items.push(Item { id, transfer, device, direction, name, count, at, state, bytes, total, path, text });
    }

    fn transfer(&mut self, transfer: &str, device: &str, direction: Direction, files: &[(String, u64)]) {
        let state = if direction == Direction::Received { State::Offered } else { State::Active };
        self.items.push(Item {
            id: transfer.to_owned(),
            transfer: transfer.to_owned(),
            device: device.to_owned(),
            direction,
            name: files.first().map(|(name, _)| name.clone()).unwrap_or_default(),
            count: files.len(),
            at: now(),
            state,
            bytes: 0,
            total: files.iter().map(|(_, size)| size).sum(),
            path: String::new(),
            text: String::new(),
        });
    }

    /// Applies the event; false when it changes nothing here. `phone` is the device a sent clip went to.
    pub fn apply(&mut self, event: &Event, phone: &str) -> bool {
        match event {
            Event::Offered { transfer, device, files } => self.transfer(transfer, device, Direction::Received, files),
            Event::Sending { transfer, device, files } => self.transfer(transfer, device, Direction::Sent, files),
            Event::Progress { transfer, bytes, total } => {
                let Some(item) = self.items.iter_mut().find(|item| item.id == *transfer) else { return false };
                (item.state, item.bytes, item.total) = (State::Active, *bytes, *total);
            }
            Event::Finished { transfer, status, paths } => {
                let Some(index) = self.items.iter().position(|item| item.id == *transfer) else { return false };
                if status != "done" {
                    self.items[index].state = State::Failed(status.clone());
                    return true;
                }
                if self.items[index].direction != Direction::Received {
                    self.items[index].state = State::Done;
                    return true;
                }
                let item = self.items.remove(index);
                for (n, path) in paths.iter().enumerate() {
                    let name = path.rsplit('/').next().unwrap_or(path).to_owned();
                    let (id, transfer, device) = (format!("{}/{n}", item.id), String::new(), item.device.clone());
                    let total = std::fs::metadata(path).map_or(0, |meta| meta.len());
                    let (count, at, state, bytes) = (1, now(), State::Done, total);
                    let (path, text, direction) = (path.clone(), String::new(), item.direction);
                    self.items.push(Item {
                        id,
                        transfer,
                        device,
                        direction,
                        name,
                        count,
                        at,
                        state,
                        bytes,
                        total,
                        path,
                        text,
                    });
                }
            }
            Event::Received { device, text } => {
                let name = text.lines().next().unwrap_or_default().to_owned();
                self.push(device, Direction::CopiedFrom, name, text.clone(), String::new());
            }
            Event::ClipSent(text) => {
                let name = text.lines().next().unwrap_or_default().to_owned();
                self.push(phone, Direction::CopiedTo, name, text.clone(), String::new());
            }
            Event::Fetched { device, purpose: Fetch::Save, paths, .. } => {
                for path in paths {
                    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    self.push(device, Direction::Saved, name, String::new(), path.to_string_lossy().into_owned());
                }
            }
            _ => return false,
        }
        true
    }
}

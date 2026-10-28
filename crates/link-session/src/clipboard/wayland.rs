//! The clipboard through `ext-data-control-v1`, or `wlr-data-control-unstable-v1` where only that exists.

use std::collections::VecDeque;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use anyhow::Context;
use tokio::io::unix::AsyncFd;
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry::WlRegistry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};

use super::{Event, REMOTE_MIME, choose, offered, read_fd};

trait OfferProxy {
    fn receive(&self, mime: String, fd: BorrowedFd<'_>);
    fn destroy(&self);
}

struct Selection {
    mimes: Vec<String>,
    offer: Box<dyn OfferProxy>,
}

impl Drop for Selection {
    fn drop(&mut self) {
        self.offer.destroy();
    }
}

enum Raw {
    Selection(Selection),
    Paste { mime: String, fd: OwnedFd },
}

#[derive(Default)]
pub struct State {
    pending: VecDeque<Raw>,
    finished: bool,
}

impl State {
    fn selection(&mut self, selection: Selection) {
        // Only the newest selection is worth reading.
        self.pending.retain(|raw| !matches!(raw, Raw::Selection(_)));
        self.pending.push_back(Raw::Selection(selection));
    }
}

macro_rules! data_control {
    ($module:ident, $($base:ident)::+, $manager:ident :: $Manager:ident, $device:ident :: $Device:ident,
     $source:ident :: $Source:ident, $offer:ident :: $Offer:ident) => {
        mod $module {
            use std::os::fd::BorrowedFd;
            use std::sync::Mutex;

            use wayland_client::globals::GlobalList;
            use wayland_client::protocol::wl_seat::WlSeat;
            use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
            use $($base)::+::$device::{self, $Device};
            use $($base)::+::$manager::$Manager;
            use $($base)::+::$offer::{self, $Offer};
            use $($base)::+::$source::{self, $Source};

            use super::{OfferProxy, Raw, Selection, State};

            pub struct Control {
                manager: $Manager,
                device: $Device,
                source: Option<$Source>,
            }

            impl Control {
                pub fn bind(globals: &GlobalList, seat: &WlSeat, qh: &QueueHandle<State>) -> Option<Self> {
                    let manager: $Manager = globals.bind(qh, 1..=1, ()).ok()?;
                    let device = manager.get_data_device(seat, qh, ());
                    Some(Self { manager, device, source: None })
                }

                pub fn offer(&mut self, mimes: &[String], qh: &QueueHandle<State>) {
                    let source = self.manager.create_data_source(qh, ());
                    for mime in mimes {
                        source.offer(mime.clone());
                    }
                    self.device.set_selection(Some(&source));
                    if let Some(old) = self.source.replace(source) {
                        old.destroy();
                    }
                }
            }

            impl OfferProxy for $Offer {
                fn receive(&self, mime: String, fd: BorrowedFd<'_>) {
                    $Offer::receive(self, mime, fd);
                }

                fn destroy(&self) {
                    $Offer::destroy(self);
                }
            }

            impl Dispatch<$Manager, ()> for State {
                fn event(_: &mut Self, _: &$Manager, _: <$Manager as Proxy>::Event, &(): &(), _: &Connection,
                    _: &QueueHandle<Self>) {
                }
            }

            impl Dispatch<$Device, ()> for State {
                fn event(state: &mut Self, _: &$Device, event: $device::Event, &(): &(), _: &Connection,
                    _: &QueueHandle<Self>) {
                    match event {
                        $device::Event::Selection { id: Some(offer) } => {
                            let mimes = offer.data::<Mutex<Vec<String>>>()
                                .map(|mimes| mimes.lock().unwrap_or_else(|poison| poison.into_inner()).clone())
                                .unwrap_or_default();
                            state.selection(Selection { mimes, offer: Box::new(offer) });
                        }
                        $device::Event::PrimarySelection { id: Some(offer) } => offer.destroy(),
                        $device::Event::Finished => state.finished = true,
                        _ => {}
                    }
                }

                event_created_child!(State, $Device, [
                    $device::EVT_DATA_OFFER_OPCODE => ($Offer, Mutex::new(Vec::new())),
                ]);
            }

            impl Dispatch<$Offer, Mutex<Vec<String>>> for State {
                fn event(_: &mut Self, _: &$Offer, event: $offer::Event, mimes: &Mutex<Vec<String>>,
                    _: &Connection, _: &QueueHandle<Self>) {
                    if let $offer::Event::Offer { mime_type } = event {
                        mimes.lock().unwrap_or_else(|poison| poison.into_inner()).push(mime_type);
                    }
                }
            }

            impl Dispatch<$Source, ()> for State {
                fn event(state: &mut Self, source: &$Source, event: $source::Event, &(): &(), _: &Connection,
                    _: &QueueHandle<Self>) {
                    match event {
                        $source::Event::Send { mime_type, fd } => {
                            state.pending.push_back(Raw::Paste { mime: mime_type, fd });
                        }
                        $source::Event::Cancelled => source.destroy(),
                        _ => {}
                    }
                }
            }
        }
    };
}

data_control!(
    ext,
    wayland_protocols::ext::data_control::v1::client,
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_device_v1::ExtDataControlDeviceV1,
    ext_data_control_source_v1::ExtDataControlSourceV1,
    ext_data_control_offer_v1::ExtDataControlOfferV1
);
data_control!(
    wlr,
    wayland_protocols_wlr::data_control::v1::client,
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
    zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1
);

enum Control {
    Ext(ext::Control),
    Wlr(wlr::Control),
}

pub struct Wayland {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    control: Control,
    fd: AsyncFd<OwnedFd>,
}

impl Wayland {
    /// `None` when the compositor offers neither data-control protocol.
    pub fn connect() -> anyhow::Result<Option<Self>> {
        if !crate::on_wayland() {
            return Ok(None);
        }
        let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
        let (globals, queue) = registry_queue_init::<State>(&conn)?;
        let qh = queue.handle();
        let Ok(seat) = globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()) else { return Ok(None) };
        // ANDROMEDA_LINK_DATA_CONTROL=wlr skips ext, so tests reach the fallback on compositors that offer both.
        let ext = std::env::var_os("ANDROMEDA_LINK_DATA_CONTROL").is_none_or(|choice| choice != "wlr");
        let control = match ext.then(|| ext::Control::bind(&globals, &seat, &qh)).flatten() {
            Some(control) => Control::Ext(control),
            None => match wlr::Control::bind(&globals, &seat, &qh) {
                Some(control) => Control::Wlr(control),
                None => return Ok(None),
            },
        };
        // Sound for OwnedFd, whose descriptor never changes; `register` would need unsafe, which the workspace forbids.
        #[allow(deprecated)]
        let fd = AsyncFd::new(conn.as_fd().try_clone_to_owned()?)?;
        Ok(Some(Self { conn, queue, state: State::default(), control, fd }))
    }

    pub fn name(&self) -> &'static str {
        match self.control {
            Control::Ext(_) => "ext-data-control-v1",
            Control::Wlr(_) => "wlr-data-control-unstable-v1",
        }
    }

    /// Makes a phone's offer the selection; pastes of it come back as `Event::Paste`.
    pub fn offer_remote(&mut self, mimes: &[String]) -> anyhow::Result<()> {
        let qh = self.queue.handle();
        let mimes: Vec<String> = mimes.iter().cloned().chain([REMOTE_MIME.to_owned()]).collect();
        match &mut self.control {
            Control::Ext(control) => control.offer(&mimes, &qh),
            Control::Wlr(control) => control.offer(&mimes, &qh),
        }
        self.conn.flush()?;
        Ok(())
    }

    pub async fn next(&mut self) -> anyhow::Result<Event> {
        loop {
            // A selection leaves the queue only once read, so a read cancelled by the caller runs again.
            while let Some(raw) = self.state.pending.front() {
                let event = match raw {
                    Raw::Paste { .. } => None,
                    Raw::Selection(selection) => read(&self.conn, selection).await,
                };
                match (self.state.pending.pop_front(), event) {
                    (Some(Raw::Paste { mime, fd }), _) => return Ok(Event::Paste { mime, sink: fd, done: None }),
                    (_, Some(event)) => return Ok(event),
                    _ => {}
                }
            }
            anyhow::ensure!(!self.state.finished, "the compositor ended the data-control device");
            self.queue.dispatch_pending(&mut self.state)?;
            if !self.state.pending.is_empty() {
                continue;
            }
            self.conn.flush()?;
            let Some(guard) = self.conn.prepare_read() else { continue };
            let mut ready = self.fd.readable().await?;
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => ready.clear_ready(),
                Err(error) => return Err(error.into()),
            }
        }
    }
}

async fn read(conn: &Connection, selection: &Selection) -> Option<Event> {
    let mime = choose(&selection.mimes)?.to_owned();
    let read = async {
        let (reader, writer) = std::io::pipe()?;
        selection.offer.receive(mime.clone(), writer.as_fd());
        drop(writer);
        conn.flush()?;
        read_fd(OwnedFd::from(reader)).await
    };
    let data = match read.await {
        Ok(data) if !data.is_empty() => data,
        Ok(_) => return None,
        Err(error) => {
            log::warn!("reading the selection's {mime}: {error:#}");
            return None;
        }
    };
    Some(Event::Local { mimes: offered(&selection.mimes, &mime), data })
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as wayland_client::Proxy>::Event,
        &(): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

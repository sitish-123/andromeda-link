//! One client thread shared by every QML singleton: each subscribes to its events and sends it commands.

use std::sync::{Mutex, OnceLock, PoisonError};

use tokio::sync::mpsc::UnboundedSender;

use crate::client::{self, Command, Event};

type Sink = Box<dyn Fn(&Event) + Send>;

struct Hub {
    commands: UnboundedSender<Command>,
    sinks: Mutex<Vec<Sink>>,
    /// The phone `Link.currentPhone` names; every page follows it.
    phone: Mutex<String>,
}

fn hub() -> &'static Hub {
    static HUB: OnceLock<Hub> = OnceLock::new();
    HUB.get_or_init(|| Hub {
        commands: client::start(publish),
        sinks: Mutex::new(Vec::new()),
        phone: Mutex::new(String::new()),
    })
}

/// Adds a sink for every later event and asks the client to send the service's state again for it.
pub fn subscribe(sink: impl Fn(&Event) + Send + 'static) {
    let hub = hub();
    hub.sinks.lock().unwrap_or_else(PoisonError::into_inner).push(Box::new(sink));
    send(Command::Resync);
}

/// Gives the event to every sink, on the calling thread.
#[expect(clippy::needless_pass_by_value, reason = "the client's sink owns each event")]
pub fn publish(event: Event) {
    for sink in hub().sinks.lock().unwrap_or_else(PoisonError::into_inner).iter() {
        sink(&event);
    }
}

pub fn send(command: Command) {
    let _ = hub().commands.send(command);
}

pub fn phone() -> String {
    hub().phone.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Records the current phone and tells every page.
pub fn set_phone(id: &str) {
    id.clone_into(&mut hub().phone.lock().unwrap_or_else(PoisonError::into_inner));
    publish(Event::Phone(id.to_owned()));
}

/// Sends the command built for the current phone, or says why there is none.
pub fn to_phone(command: impl FnOnce(String) -> Command) {
    let id = phone();
    if id.is_empty() {
        publish(Event::Failed("No phone is paired".to_owned()));
    } else {
        send(command(id));
    }
}

//! `LinkCalls`: the current phone's call log by day, contacts that match a search, and calls placed through it.

use std::collections::HashMap;
use std::pin::Pin;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QVariant};

use crate::client::{Command, Event, LoggedCall};
use crate::format::{clock, day_text, text};
use crate::hub;
use crate::link::new_list_model;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("link-desk/src/link.cxxqt.h");
        type ListModel = crate::link::qobject::ListModel;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, query, READ, WRITE = search, NOTIFY)]
        #[qproperty(bool, ready)]
        #[qproperty(QString, refused)]
        #[qproperty(*mut ListModel, calls, READ, CONSTANT)]
        type LinkCalls = super::CallsRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        fn search(self: Pin<&mut LinkCalls>, query: QString);
        /// Places a call on the phone with the digits, `+`, `*`, and `#` of `number`; false when none is left.
        #[qinvokable]
        fn dial(self: Pin<&mut LinkCalls>, number: &QString) -> bool;
    }

    impl cxx_qt::Threading for LinkCalls {}
    impl cxx_qt::Initialize for LinkCalls {}
}

const ROLES: &[&str] = &["number", "name", "initial", "kind", "detail", "section"];

pub struct CallsRust {
    query: QString,
    ready: bool,
    refused: QString,
    calls: *mut qobject::ListModel,
    model: UniquePtr<qobject::ListModel>,
    phone: String,
    /// Device id to its calls, newest first.
    log: HashMap<String, Vec<LoggedCall>>,
    /// Device id to why its phone refused the log.
    refusals: HashMap<String, String>,
    /// Device id to (name, numbers) per contact.
    contacts: HashMap<String, Vec<(String, Vec<String>)>>,
}

impl Default for CallsRust {
    fn default() -> Self {
        let model = new_list_model(ROLES);
        Self {
            query: QString::default(),
            ready: false,
            refused: QString::default(),
            calls: model.as_mut_ptr(),
            model,
            phone: hub::phone(),
            log: HashMap::new(),
            refusals: HashMap::new(),
            contacts: HashMap::new(),
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkCalls {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            if matches!(event, Event::Calls(_) | Event::CallsRefused(_) | Event::Contacts(_) | Event::Phone(_)) {
                let event = event.clone();
                let _ = thread.queue(move |calls| calls.apply(event));
            }
        });
    }
}

/// What a phone can dial out of `number`.
fn dialable(number: &str) -> String {
    number.chars().filter(|c| c.is_ascii_digit() || "+*#".contains(*c)).collect()
}

/// The initial of a name, or "#" for a number.
fn initial(name: &str) -> String {
    let first = name.chars().next().unwrap_or('#');
    if first.is_alphabetic() { first.to_uppercase().to_string() } else { "#".to_owned() }
}

/// "3 min", "45 s".
fn duration(secs: u32) -> String {
    if secs < 60 { format!("{secs} s") } else { format!("{} min", secs.div_ceil(60)) }
}

impl CallsRust {
    fn row(call: &LoggedCall) -> Vec<QVariant> {
        let at = call.date / 1000;
        let shown = if call.name.is_empty() { &call.number } else { &call.name };
        let detail = match call.kind.as_str() {
            "missed" => format!("Missed, {}", clock(at)),
            "outgoing" => format!("Outgoing, {}, {}", clock(at), duration(call.duration)),
            _ => format!("Incoming, {}, {}", clock(at), duration(call.duration)),
        };
        vec![
            text(&call.number),
            text(shown),
            text(&initial(shown)),
            text(&call.kind),
            text(&detail),
            text(&day_text(at)),
        ]
    }

    /// Contacts whose name or number holds the query, one row per number, then the matching calls.
    fn rows(&self) -> Vec<Vec<QVariant>> {
        let query = self.query.to_string().trim().to_lowercase();
        let digits = dialable(&query);
        let holds = |name: &str, number: &str| {
            query.is_empty()
                || name.to_lowercase().contains(&query)
                || (!digits.is_empty() && dialable(number).contains(&digits))
        };
        let mut rows = Vec::new();
        if !query.is_empty() {
            let contacts = self.contacts.get(&self.phone).map_or(&[][..], Vec::as_slice);
            for (name, numbers) in contacts {
                for number in numbers.iter().filter(|number| holds(name, number)) {
                    let row = [number.as_str(), name, &initial(name), "contact", number, "Contacts"];
                    rows.push(row.iter().map(|value| text(value)).collect());
                }
            }
        }
        let calls = self.log.get(&self.phone).map_or(&[][..], Vec::as_slice);
        rows.extend(calls.iter().filter(|call| holds(&call.name, &call.number)).map(Self::row));
        rows
    }
}

impl qobject::LinkCalls {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        let mut state = self.as_mut().rust_mut();
        match event {
            Event::Calls(log) => state.log = log,
            Event::CallsRefused(refusals) => state.refusals = refusals,
            Event::Contacts(contacts) => state.contacts = contacts,
            Event::Phone(id) => state.phone = id,
            _ => return,
        }
        self.refresh();
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        let rows = state.rows();
        let ready = state.log.contains_key(&state.phone);
        let refused = QString::from(state.refusals.get(&state.phone).map_or("", String::as_str));
        if let Some(model) = state.model.as_mut() {
            model.replace(rows);
        }
        self.as_mut().set_ready(ready);
        self.as_mut().set_refused(refused);
    }

    fn search(mut self: Pin<&mut Self>, query: QString) {
        if self.query == query {
            return;
        }
        self.as_mut().rust_mut().query = query;
        self.as_mut().query_changed();
        self.refresh();
    }

    fn dial(self: Pin<&mut Self>, number: &QString) -> bool {
        let number = dialable(&number.to_string());
        if number.is_empty() {
            return false;
        }
        hub::to_phone(|device| Command::Dial { device, number });
        true
    }
}

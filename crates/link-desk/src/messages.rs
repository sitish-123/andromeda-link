//! `LinkMessages`: the current phone's texts by thread, and sending through it.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use cxx::UniquePtr;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QVariant};

use crate::client::{Command, Event, Text};
use crate::format::{clock, day_text, short_when, text, variant};
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
        #[qproperty(i32, unread_count)]
        #[qproperty(QString, current_thread, READ, WRITE = open_thread, NOTIFY)]
        #[qproperty(QString, thread_name)]
        #[qproperty(QString, thread_address)]
        #[qproperty(QString, query, READ, WRITE = search, NOTIFY)]
        #[qproperty(*mut ListModel, threads, READ, CONSTANT)]
        #[qproperty(*mut ListModel, conversation, READ, CONSTANT)]
        type LinkMessages = super::MessagesRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        fn open_thread(self: Pin<&mut LinkMessages>, thread: QString);
        fn search(self: Pin<&mut LinkMessages>, query: QString);
        #[qinvokable]
        fn send(self: Pin<&mut LinkMessages>, body: &QString);
        #[qinvokable]
        fn send_to(self: Pin<&mut LinkMessages>, address: &QString, body: &QString);
    }

    impl cxx_qt::Threading for LinkMessages {}
    impl cxx_qt::Initialize for LinkMessages {}
}

const THREAD_ROLES: &[&str] = &["thread", "name", "address", "initial", "snippet", "timeText", "unread", "section"];
const TEXT_ROLES: &[&str] = &["body", "sent", "timeText", "dayText"];

pub struct MessagesRust {
    unread_count: i32,
    current_thread: QString,
    thread_name: QString,
    thread_address: QString,
    query: QString,
    threads: *mut qobject::ListModel,
    conversation: *mut qobject::ListModel,
    thread_model: UniquePtr<qobject::ListModel>,
    text_model: UniquePtr<qobject::ListModel>,
    phone: String,
    /// Device id to its texts, newest first.
    texts: HashMap<String, Vec<Text>>,
    /// Device id to the contact name of each `number_key`.
    names: HashMap<String, HashMap<String, String>>,
    /// (device, address) with a text that arrived since the app started and was not opened since.
    unread: HashSet<(String, String)>,
}

impl Default for MessagesRust {
    fn default() -> Self {
        let thread_model = new_list_model(THREAD_ROLES);
        let text_model = new_list_model(TEXT_ROLES);
        Self {
            unread_count: 0,
            current_thread: QString::default(),
            thread_name: QString::default(),
            thread_address: QString::default(),
            query: QString::default(),
            threads: thread_model.as_mut_ptr(),
            conversation: text_model.as_mut_ptr(),
            thread_model,
            text_model,
            phone: hub::phone(),
            texts: HashMap::new(),
            names: HashMap::new(),
            unread: HashSet::new(),
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkMessages {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            if matches!(event, Event::Texts(_) | Event::Contacts(_) | Event::TextReceived { .. } | Event::Phone(_)) {
                let event = event.clone();
                let _ = thread.queue(move |messages| messages.apply(event));
            }
        });
    }
}

/// The last nine digits of a number, enough to match "+44 7700 900123" with "07700900123".
fn number_key(address: &str) -> String {
    let digits: Vec<char> = address.chars().filter(char::is_ascii_digit).collect();
    digits[digits.len().saturating_sub(9)..].iter().collect()
}

/// Each `number_key` to the first contact with that number.
fn names(contacts: Vec<(String, Vec<String>)>) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for (name, numbers) in contacts {
        for key in numbers.iter().map(|number| number_key(number)).filter(|key| !key.is_empty()) {
            names.entry(key).or_insert_with(|| name.clone());
        }
    }
    names
}

fn snippet(text: &Text) -> &str {
    text.body.lines().next().unwrap_or_default()
}

fn seconds(ms: u64) -> u64 {
    ms / 1000
}

impl MessagesRust {
    fn name_of(&self, address: &str) -> Option<String> {
        self.names.get(&self.phone)?.get(&number_key(address)).cloned()
    }

    /// The newest text of each thread, newest first.
    fn heads(&self) -> Vec<&Text> {
        let mut seen = HashSet::new();
        let texts = self.texts.get(&self.phone).map_or(&[][..], Vec::as_slice);
        texts.iter().filter(|text| seen.insert(text.thread)).collect()
    }

    fn is_unread(&self, address: &str) -> bool {
        self.unread.contains(&(self.phone.clone(), address.to_owned()))
    }
}

impl qobject::LinkMessages {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        let mut state = self.as_mut().rust_mut();
        match event {
            Event::Texts(texts) => state.texts = texts,
            Event::Contacts(contacts) => {
                state.names = contacts.into_iter().map(|(device, list)| (device, names(list))).collect();
            }
            Event::TextReceived { device, address } => drop(state.unread.insert((device, address))),
            Event::Phone(id) => {
                state.phone = id;
                self.as_mut().open_thread(QString::default());
            }
            _ => return,
        }
        self.refresh();
    }

    fn refresh(mut self: Pin<&mut Self>) {
        let mut state = self.as_mut().rust_mut();
        let mut heads = state.heads();
        heads.sort_by_key(|text| (!state.is_unread(&text.address), std::cmp::Reverse(text.date)));
        let query = state.query.to_string().trim().to_lowercase();
        let rows: Vec<Vec<QVariant>> = heads
            .iter()
            .map(|head| (head, state.name_of(&head.address).unwrap_or_else(|| head.address.clone())))
            .filter(|(head, name)| {
                query.is_empty() || format!("{name} {} {}", head.address, snippet(head)).to_lowercase().contains(&query)
            })
            .map(|(head, name)| {
                let first = name.chars().next().unwrap_or('#');
                let initial = if first.is_alphabetic() { first.to_uppercase().to_string() } else { "#".to_owned() };
                let unread = state.is_unread(&head.address);
                vec![
                    text(&head.thread.to_string()),
                    text(&name),
                    text(&head.address),
                    text(&initial),
                    text(snippet(head)),
                    text(&short_when(seconds(head.date))),
                    variant(&unread),
                    text(if unread { "Unread" } else { "Earlier" }),
                ]
            })
            .collect();
        let unread_count =
            i32::try_from(heads.iter().filter(|head| state.is_unread(&head.address)).count()).unwrap_or(0);
        let open = state.current_thread.to_string();
        let texts = state.texts.get(&state.phone).map_or(&[][..], Vec::as_slice);
        let conversation: Vec<&Text> = texts.iter().rev().filter(|text| text.thread.to_string() == open).collect();
        let address = conversation.first().map(|text| text.address.clone()).unwrap_or_default();
        let name = state.name_of(&address).unwrap_or_else(|| address.clone());
        let lines = conversation
            .iter()
            .map(|line| {
                let at = seconds(line.date);
                vec![text(&line.body), variant(&line.sent), text(&clock(at)), text(&day_text(at))]
            })
            .collect();
        if let Some(model) = state.thread_model.as_mut() {
            model.replace(rows);
        }
        if let Some(model) = state.text_model.as_mut() {
            model.replace(lines);
        }
        self.as_mut().set_unread_count(unread_count);
        self.as_mut().set_thread_name(QString::from(&name));
        self.as_mut().set_thread_address(QString::from(&address));
    }

    fn open_thread(mut self: Pin<&mut Self>, thread: QString) {
        if self.current_thread == thread {
            return;
        }
        let address = {
            let state = self.as_mut().rust_mut();
            let open = thread.to_string();
            let texts = state.texts.get(&state.phone).map_or(&[][..], Vec::as_slice);
            texts.iter().find(|text| text.thread.to_string() == open).map(|text| text.address.clone())
        };
        let mut state = self.as_mut().rust_mut();
        if let Some(address) = address {
            let key = (state.phone.clone(), address);
            state.unread.remove(&key);
        }
        state.current_thread = thread;
        self.as_mut().current_thread_changed();
        self.refresh();
    }

    fn search(mut self: Pin<&mut Self>, query: QString) {
        if self.query != query {
            self.as_mut().rust_mut().query = query;
            self.as_mut().query_changed();
            self.refresh();
        }
    }

    fn send(self: Pin<&mut Self>, body: &QString) {
        let to = self.thread_address.to_string();
        if to.is_empty() {
            hub::publish(Event::Failed("Open a conversation first".to_owned()));
            return;
        }
        let body = body.to_string();
        hub::to_phone(|device| Command::SendText { device, to, body });
    }

    fn send_to(self: Pin<&mut Self>, address: &QString, body: &QString) {
        let (to, body) = (address.to_string(), body.to_string());
        hub::to_phone(|device| Command::SendText { device, to, body });
    }
}

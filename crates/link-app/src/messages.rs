//! Messages: the phone's texts by conversation, newest first, and a field to answer through the phone.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::{self as gtk, Align, Orientation, glib, prelude::*};

use crate::link::{Command, Text};
use crate::view::{Ctx, Gate};

#[derive(Clone)]
pub struct Messages(Rc<Inner>);

pub struct Inner {
    pub gate: Gate,
    ctx: Ctx,
    threads: gtk::ListBox,
    title: gtk::Label,
    talk: gtk::Box,
    scroll: gtk::ScrolledWindow,
    entry: gtk::Entry,
    /// The conversation on show.
    thread: Cell<Option<u64>>,
    texts: RefCell<Vec<Text>>,
    /// (name, numbers) per contact.
    contacts: RefCell<Vec<(String, Vec<String>)>>,
}

impl Messages {
    pub fn new(ctx: &Ctx) -> Self {
        let page = gtk::Box::new(Orientation::Horizontal, 0);
        page.add_css_class("messages");
        let threads = gtk::ListBox::new();
        threads.add_css_class("threads");
        let side = gtk::ScrolledWindow::builder()
            .child(&threads)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(320)
            .build();
        page.append(&side);
        let right = gtk::Box::new(Orientation::Vertical, 0);
        right.set_hexpand(true);
        let title = gtk::Label::builder().xalign(0.0).build();
        title.add_css_class("thread-title");
        right.append(&title);
        let talk = gtk::Box::new(Orientation::Vertical, 6);
        talk.add_css_class("talk");
        talk.set_valign(Align::End);
        let scroll = gtk::ScrolledWindow::builder().child(&talk).vexpand(true).build();
        right.append(&scroll);
        let compose = gtk::Box::new(Orientation::Horizontal, 6);
        compose.add_css_class("compose");
        let entry = gtk::Entry::builder().placeholder_text("Text through the phone").hexpand(true).build();
        compose.append(&entry);
        let send = gtk::Button::from_icon_name("go-next-symbolic");
        send.add_css_class("send");
        send.update_property(&[gtk::accessible::Property::Label("Send")]);
        compose.append(&send);
        right.append(&compose);
        page.append(&right);
        let gate = Gate::new(
            ctx,
            "chat-message-new-symbolic",
            "messages",
            "Messages are off",
            "Turn it on to read the phone's texts here and answer them.",
            &page,
        );
        let messages = Self(Rc::new(Inner {
            gate,
            ctx: ctx.clone(),
            threads,
            title,
            talk,
            scroll,
            entry: entry.clone(),
            thread: Cell::new(None),
            texts: RefCell::default(),
            contacts: RefCell::default(),
        }));
        let go = {
            let messages = messages.clone();
            move || messages.send()
        };
        entry.connect_activate({
            let go = go.clone();
            move |_| go()
        });
        send.connect_clicked(move |_| go());
        messages.0.threads.connect_row_activated({
            let messages = messages.clone();
            move |_, row| {
                let thread = row.widget_name().parse().ok();
                messages.0.thread.set(thread);
                messages.show_thread();
            }
        });
        messages
    }

    pub fn gate(&self) -> &Gate {
        &self.0.gate
    }

    /// The phone's texts (newest first) and contacts changed.
    pub fn set(&self, texts: Vec<Text>, contacts: Vec<(String, Vec<String>)>) {
        let this = &self.0;
        this.texts.replace(texts);
        this.contacts.replace(contacts);
        while let Some(row) = this.threads.first_child() {
            this.threads.remove(&row);
        }
        let texts = this.texts.borrow();
        let mut seen = Vec::new();
        for text in texts.iter() {
            if seen.contains(&text.thread) {
                continue;
            }
            seen.push(text.thread);
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&text.thread.to_string());
            let body = gtk::Box::new(Orientation::Vertical, 2);
            body.add_css_class("thread");
            let top = gtk::Box::new(Orientation::Horizontal, 8);
            let name = gtk::Label::builder().label(self.name_of(&text.address)).xalign(0.0).hexpand(true).build();
            name.add_css_class("from-name");
            top.append(&name);
            let when = gtk::Label::new(Some(&when(text.date)));
            when.add_css_class("faint");
            top.append(&when);
            body.append(&top);
            let last = gtk::Label::builder()
                .label(if text.sent { format!("You: {}", text.body) } else { text.body.clone() })
                .xalign(0.0)
                .ellipsize(pango::EllipsizeMode::End)
                .build();
            last.add_css_class("slate");
            body.append(&last);
            row.set_child(Some(&body));
            this.threads.append(&row);
        }
        if this.thread.get().is_none_or(|thread| !seen.contains(&thread)) {
            this.thread.set(seen.first().copied());
        }
        drop(texts);
        self.show_thread();
    }

    fn show_thread(&self) {
        let this = &self.0;
        while let Some(child) = this.talk.first_child() {
            this.talk.remove(&child);
        }
        let texts = this.texts.borrow();
        let thread: Vec<&Text> = texts.iter().filter(|text| Some(text.thread) == this.thread.get()).collect();
        this.title.set_text(&thread.first().map(|text| self.name_of(&text.address)).unwrap_or_default());
        this.entry.set_sensitive(!thread.is_empty());
        if thread.is_empty() {
            let none = gtk::Label::new(Some("No texts yet."));
            none.add_css_class("faint");
            this.talk.append(&none);
        }
        for text in thread.iter().rev() {
            let bubble = gtk::Label::builder().label(&text.body).wrap(true).xalign(0.0).max_width_chars(48).build();
            bubble.add_css_class("bubble");
            bubble.add_css_class(if text.sent { "sent" } else { "got" });
            bubble.set_halign(if text.sent { Align::End } else { Align::Start });
            bubble.set_tooltip_text(Some(&when(text.date)));
            this.talk.append(&bubble);
        }
        let row = this
            .threads
            .observe_children()
            .into_iter()
            .filter_map(|child| child.ok().and_downcast::<gtk::ListBoxRow>())
            .find(|row| this.thread.get().is_some_and(|thread| row.widget_name() == thread.to_string()));
        this.threads.select_row(row.as_ref());
        // Newest at the bottom, in view.
        let adjustment = this.scroll.vadjustment();
        glib::idle_add_local_once(move || adjustment.set_value(adjustment.upper()));
    }

    fn send(&self) {
        let this = &self.0;
        let body = this.entry.text().trim().to_owned();
        let to = this
            .texts
            .borrow()
            .iter()
            .find(|text| Some(text.thread) == this.thread.get())
            .map(|text| text.address.clone());
        let (Some(to), false) = (to, body.is_empty()) else { return };
        this.ctx.send(|device| Command::SendText { device, to, body });
        this.entry.set_text("");
    }

    /// The contact's name for a number, matched on its last nine digits; else the number.
    fn name_of(&self, address: &str) -> String {
        let tail = |number: &str| {
            let digits: String = number.chars().filter(char::is_ascii_digit).collect();
            digits[digits.len().saturating_sub(9)..].to_owned()
        };
        let wanted = tail(address);
        self.0
            .contacts
            .borrow()
            .iter()
            .find(|(_, numbers)| !wanted.is_empty() && numbers.iter().any(|number| tail(number) == wanted))
            .map_or_else(|| address.to_owned(), |(name, _)| name.clone())
    }
}

fn when(ms: u64) -> String {
    let Ok(seconds) = i64::try_from(ms / 1000) else { return String::new() };
    let (Ok(when), Ok(now)) = (glib::DateTime::from_unix_local(seconds), glib::DateTime::now_local()) else {
        return String::new();
    };
    let format = if when.ymd() == now.ymd() { "%H:%M" } else { "%-d %b" };
    when.format(format).map(|text| text.to_string()).unwrap_or_default()
}

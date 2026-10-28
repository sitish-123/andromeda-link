//! What every view shares: the phone on show, the command line to the service, and the gate that says why a view
//! is empty (no service, no phone, the phone's switch off, the phone away).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::{self as gtk, Align, prelude::*};
use libadwaita as adw;
use tokio::sync::mpsc::UnboundedSender;

use crate::link::{Command, Device};
use crate::seam;

#[derive(Clone)]
pub struct Ctx {
    pub commands: UnboundedSender<Command>,
    /// The phone on show.
    pub device: Rc<RefCell<Option<Device>>>,
}

impl Ctx {
    /// Sends a command for the phone on show, when there is one.
    pub fn send(&self, command: impl FnOnce(String) -> Command) {
        if let Some(device) = self.device.borrow().as_ref() {
            let _ = self.commands.send(command(device.id.clone()));
        }
    }
}

pub struct Gate {
    pub stack: gtk::Stack,
    away: adw::StatusPage,
    grant: &'static str,
}

impl Gate {
    /// `off` is the title while the phone's `grant` switch is off; `why` says what turning it on gives.
    pub fn new(ctx: &Ctx, icon: &str, grant: &'static str, off: &str, why: &str, page: &impl IsA<gtk::Widget>) -> Self {
        let stack = gtk::Stack::new();
        stack.add_named(page, Some("page"));
        let status = |title: &str, description: &str| {
            adw::StatusPage::builder().icon_name(icon).title(title).description(description).build()
        };
        stack.add_named(
            &status("Link is not running", "Start andromeda-linkd; this view fills in when it runs."),
            Some("down"),
        );
        stack.add_named(&status("No phone yet", "Pair a phone in Settings."), Some("none"));
        let allow = seam::button("Turn on", true);
        allow.set_halign(Align::Center);
        allow.connect_clicked({
            let ctx = ctx.clone();
            move |_| ctx.send(|device| Command::SetGrant { device, feature: grant.to_owned(), granted: true })
        });
        let off = status(off, why);
        off.set_child(Some(&allow));
        stack.add_named(&off, Some("off"));
        let away = status("", "This view fills in when it connects.");
        stack.add_named(&away, Some("away"));
        Self { stack, away, grant }
    }

    /// Shows the page or why not; true when the page shows.
    pub fn show(&self, service: bool, device: Option<&Device>, grants: &HashMap<String, Vec<String>>) -> bool {
        let page = match device {
            _ if !service => "down",
            None => "none",
            Some(device) if !grants.get(&device.id).is_some_and(|held| held.iter().any(|held| held == self.grant)) => {
                "off"
            }
            Some(device) if !device.connected => {
                self.away.set_title(&format!("{} is not connected", device.name));
                "away"
            }
            Some(_) => "page",
        };
        self.stack.set_visible_child_name(page);
        page == "page"
    }
}

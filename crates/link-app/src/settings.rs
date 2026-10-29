//! Settings: the service, discovery, and what the phone on show may do.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::{Align, prelude::*};
use libadwaita::{self as adw, prelude::*};

use crate::link::{Command, Device};
use crate::seam;
use crate::view::Ctx;

/// Each grant the service knows, with what it lets the phone do, in plain words.
const GRANTS: [(&str, &str, &str); 13] = [
    ("clipboard", "Clipboard", "Copy on one, paste on the other"),
    ("files", "Files", "Send files both ways"),
    ("notifications", "Notifications", "Show the phone's notifications here"),
    ("media", "Media", "Show and control what the phone plays"),
    ("ring", "Ring", "Ring the phone from here, and this desk from the phone"),
    ("calls", "Calls", "Show the phone's calls here"),
    ("browse", "Phone storage", "Open the phone's files from this desk"),
    ("screen", "Screen", "Show the phone's screen here"),
    ("messages", "Messages", "Read and send the phone's texts"),
    ("input", "Remote input", "Use the phone as a pointer and keyboard"),
    ("commands", "Commands", "Run this desk's commands from the phone"),
    (
        "system",
        "Control system",
        "Change this desk's volume, brightness, and lock from the phone, and the phone's from here",
    ),
    ("dnd-sync", "Sync Do Not Disturb", "Keep Do Not Disturb the same on this desk and the phone"),
];

pub struct Settings {
    pub page: adw::PreferencesPage,
    service: adw::ActionRow,
    localsend: adw::SwitchRow,
    notify_locked: adw::SwitchRow,
    phone: adw::PreferencesGroup,
    grants: Vec<adw::SwitchRow>,
    auto_accept: adw::SwitchRow,
    /// Set while `show` writes the rows, so their handlers send nothing.
    quiet: Rc<Cell<bool>>,
    /// The values the switches were last set from.
    written: RefCell<String>,
}

impl Settings {
    pub fn new(ctx: &Ctx) -> Self {
        let commands = &ctx.commands;
        let page = adw::PreferencesPage::new();
        page.add_css_class("settings");
        let device = &ctx.device;
        let quiet = Rc::new(Cell::new(false));
        let switch = |row: &adw::SwitchRow, command: Box<dyn Fn(String, bool) -> Command>| {
            let (commands, device, quiet) = (commands.clone(), device.clone(), quiet.clone());
            row.connect_active_notify(move |row| {
                if quiet.get() {
                    return;
                }
                if let Some(device) = device.borrow().as_ref() {
                    let _ = commands.send(command(device.id.clone(), row.is_active()));
                }
            });
        };

        let general = adw::PreferencesGroup::builder().title("This desk").build();
        let service = adw::ActionRow::builder().title("Link service").build();
        general.add(&service);
        let desk_switch = |title: &str, subtitle: &str, command: fn(bool) -> Command| {
            let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).build();
            let (commands, quiet) = (commands.clone(), quiet.clone());
            row.connect_active_notify(move |row| {
                if !quiet.get() {
                    let _ = commands.send(command(row.is_active()));
                }
            });
            general.add(&row);
            row
        };
        let localsend = desk_switch(
            "Visible to LocalSend",
            "Receive files from LocalSend apps on this network, with the same consent as Link",
            Command::SetLocalSend,
        );
        let notify_locked = desk_switch(
            "Show phone notifications while locked",
            "Off, a locked desk shows only how many arrived",
            Command::SetNotifyLocked,
        );
        let pair = seam::button("Pair a phone", false);
        pair.set_valign(Align::Center);
        pair.set_action_name(Some("win.pair"));
        let pair_row = adw::ActionRow::builder().title("Another phone").subtitle("Scan a code with Link on it").build();
        pair_row.add_suffix(&pair);
        general.add(&pair_row);
        page.add(&general);

        let phone = adw::PreferencesGroup::new();
        let auto_accept = adw::SwitchRow::builder()
            .title("Accept files without asking")
            .subtitle("Files from this phone go straight to Downloads")
            .build();
        switch(&auto_accept, Box::new(|device, enabled| Command::SetAutoAccept { device, enabled }));
        phone.add(&auto_accept);
        let grants = GRANTS
            .iter()
            .map(|&(feature, title, subtitle)| {
                let row = adw::SwitchRow::builder().title(title).subtitle(subtitle).build();
                switch(
                    &row,
                    Box::new(move |device, granted| Command::SetGrant { device, feature: feature.to_owned(), granted }),
                );
                phone.add(&row);
                row
            })
            .collect();
        page.add(&phone);

        let forget = adw::PreferencesGroup::new();
        let unpair = seam::button("Unpair", false);
        unpair.set_valign(Align::Center);
        let unpair_row = adw::ActionRow::builder()
            .title("Unpair this phone")
            .subtitle("It forgets this desk at its next contact")
            .build();
        unpair_row.add_suffix(&unpair);
        forget.add(&unpair_row);
        page.add(&forget);
        unpair.connect_clicked({
            let (commands, device) = (commands.clone(), device.clone());
            move |button| {
                let Some(phone) = device.borrow().clone() else { return };
                let dialog = adw::AlertDialog::builder()
                    .heading(format!("Unpair {}?", phone.name))
                    .body("Link on the phone stops working with this desk until you pair them again.")
                    .default_response("cancel")
                    .close_response("cancel")
                    .build();
                dialog.add_responses(&[("cancel", "Cancel"), ("unpair", "Unpair")]);
                dialog.set_response_appearance("unpair", adw::ResponseAppearance::Destructive);
                let commands = commands.clone();
                dialog.connect_response(Some("unpair"), move |_, _| {
                    let _ = commands.send(Command::Unpair(phone.id.clone()));
                });
                dialog.present(Some(button));
            }
        });
        // Rows named by the phone are hidden until there is one.
        let phone_rows = forget.clone();
        phone.bind_property("visible", &phone_rows, "visible").sync_create().build();

        Self { page, service, localsend, notify_locked, phone, grants, auto_accept, quiet, written: RefCell::default() }
    }

    pub fn show(
        &self,
        service: bool,
        device: Option<&Device>,
        grants: &HashMap<String, Vec<String>>,
        auto_accept: &[String],
        localsend: bool,
        notify_locked: bool,
    ) {
        self.quiet.set(true);
        self.service.set_subtitle(if service {
            "Running"
        } else {
            "Not running. Start andromeda-linkd; this window connects as soon as it runs."
        });
        self.localsend.set_sensitive(service);
        self.notify_locked.set_sensitive(service);
        self.phone.set_visible(service && device.is_some());
        // The switches change only when the service's values do, so a flip waiting for its answer stays put.
        let held = device.and_then(|device| grants.get(&device.id)).map_or(&[][..], Vec::as_slice);
        let auto = device.is_some_and(|device| auto_accept.contains(&device.id));
        let values = format!("{:?}{held:?}{auto}{localsend}{notify_locked}", device.map(|device| &device.id));
        if self.written.replace(values.clone()) == values {
            self.quiet.set(false);
            if let Some(device) = device {
                self.describe(device);
            }
            return;
        }
        self.localsend.set_active(localsend);
        self.notify_locked.set_active(notify_locked);
        if let Some(device) = device {
            self.describe(device);
            for (row, (feature, ..)) in self.grants.iter().zip(GRANTS) {
                row.set_active(held.iter().any(|held| held == feature));
            }
            self.auto_accept.set_active(auto);
        }
        self.quiet.set(false);
    }

    fn describe(&self, device: &Device) {
        self.phone.set_title(&device.name);
        self.phone.set_description(Some(if device.connected {
            "What this phone may do with this desk"
        } else {
            "Not connected. Changes reach the phone when it connects."
        }));
    }
}

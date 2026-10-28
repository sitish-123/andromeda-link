//! Screen: opens the phone's screen in `andromeda-link-mirror`'s window.

use gtk4::{self as gtk, Align, prelude::*};
use libadwaita as adw;

use crate::seam;
use crate::view::{Ctx, Gate};

pub struct Screen {
    pub gate: Gate,
}

impl Screen {
    pub fn new(ctx: &Ctx) -> Self {
        let show = seam::button("Show the phone's screen", true);
        show.set_halign(Align::Center);
        let page = adw::StatusPage::builder()
            .icon_name("phone-symbolic")
            .title("The phone's screen")
            .description("Opens in its own window. The phone asks once before it shares; tap and type on it from here.")
            .child(&show)
            .build();
        let device = ctx.device.clone();
        show.connect_clicked(move |button| {
            let Some(device) = device.borrow().clone() else { return };
            // The mirror ships beside this binary; a build tree has it there too.
            let mirror = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("andromeda-link-mirror")))
                .filter(|path| path.exists())
                .unwrap_or_else(|| "andromeda-link-mirror".into());
            if let Err(error) = std::process::Command::new(&mirror).args([&device.id, "--name", &device.name]).spawn() {
                log::warn!("{}: {error}", mirror.display());
                button.set_label("The mirror did not start");
            }
        });
        let gate = Gate::new(
            ctx,
            "phone-symbolic",
            "screen",
            "Screen is off",
            "Turn it on to see and use the phone's screen from this desk.",
            &page,
        );
        Self { gate }
    }
}

pub struct Calls {
    pub gate: Gate,
    list: gtk::ListBox,
    stack: gtk::Stack,
}

impl Calls {
    pub fn new(ctx: &Ctx) -> Self {
        let list = gtk::ListBox::new();
        list.add_css_class("entries");
        list.set_selection_mode(gtk::SelectionMode::None);
        let stack = gtk::Stack::new();
        let empty = adw::StatusPage::builder()
            .icon_name("call-start-symbolic")
            .title("No calls yet")
            .description("Calls to the phone show here while Link runs.")
            .build();
        stack.add_named(&empty, Some("empty"));
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.add_css_class("browse");
        page.append(&gtk::ScrolledWindow::builder().child(&list).vexpand(true).build());
        stack.add_named(&page, Some("list"));
        let gate = Gate::new(
            ctx,
            "call-start-symbolic",
            "calls",
            "Calls are off",
            "Turn it on to see the phone's calls here and silence them.",
            &stack,
        );
        Self { gate, list, stack }
    }

    /// A call that ended; `who` is markup, newest on top.
    pub fn ended(&self, who: &str, missed: bool, time: &str) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.add_css_class("entry");
        let image = gtk::Image::from_icon_name(if missed { "call-missed-symbolic" } else { "call-start-symbolic" });
        image.add_css_class("app");
        if missed {
            image.add_css_class("miss");
        }
        row.append(&image);
        let name = gtk::Label::builder().xalign(0.0).hexpand(true).build();
        name.set_markup(who);
        row.append(&name);
        let what = gtk::Label::new(Some(if missed { "Missed" } else { "Answered" }));
        what.add_css_class("slate");
        row.append(&what);
        let when = gtk::Label::new(Some(time));
        when.add_css_class("slate");
        row.append(&when);
        self.list.prepend(&row);
        self.stack.set_visible_child_name("list");
    }
}

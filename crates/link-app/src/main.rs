//! `andromeda-link`: the Link window. Home is the seam between the phone and this desk.

mod files;
mod link;
mod messages;
mod screen;
mod seam;
mod settings;
mod view;
mod window;

use gtk4::{gdk, glib, prelude::*};
use libadwaita as adw;

const APP_ID: &str = "org.andromeda.Link";
/// Where the package installs the font and the style sheet; `ANDROMEDA_LINK_DATA` overrides it for a build tree.
const DATA_DIR: &str = "/usr/share/andromeda-link";

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| load_style());
    app.connect_activate(window::present);
    app.run()
}

fn load_style() {
    let data = std::env::var_os("ANDROMEDA_LINK_DATA").map_or_else(|| DATA_DIR.into(), std::path::PathBuf::from);
    // The display's font map is shared by every widget, so a font added to it is available to all of them.
    if let Some(fonts) = gtk4::Label::new(None).pango_context().font_map()
        && let Err(error) = fonts.add_font_file(data.join("fonts/GoogleSansFlex.ttf"))
    {
        log::warn!("Google Sans Flex: {error}");
    }
    let css = gtk4::CssProvider::new();
    css.load_from_path(data.join("style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(&display, &css, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}

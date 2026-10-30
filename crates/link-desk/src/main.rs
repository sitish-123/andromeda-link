mod activity;
mod calls;
mod client;
mod desktop;
mod files;
mod format;
mod history;
mod hub;
mod link;
mod messages;
#[cfg_attr(not(target_os = "linux"), path = "mirror_none.rs")]
mod mirror;
mod pairing;
mod photos;
mod screen;
mod settings;
#[cfg_attr(not(target_os = "linux"), path = "tray_none.rs")]
mod tray;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/org/andromeda/Link/qml/Main.qml"));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }
}

//! `LinkPairing`: one pairing window at a time, its code and QR image, and how it ended.

use std::fmt::Write;
use std::pin::Pin;

use cxx_qt::Threading;
use cxx_qt_lib::{QString, QUrl};

use crate::client::{Command, Event, cache};
use crate::format::{file_url, now};
use crate::hub;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qurl.h");
        type QUrl = cxx_qt_lib::QUrl;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, state)]
        #[qproperty(QString, code)]
        #[qproperty(QUrl, qr_url)]
        #[qproperty(i64, expires_at)]
        #[qproperty(QString, paired_name)]
        #[qproperty(QString, failure)]
        type LinkPairing = super::PairingRust;
    }

    #[auto_cxx_name]
    extern "RustQt" {
        #[qinvokable]
        fn start(self: Pin<&mut LinkPairing>);
        #[qinvokable]
        fn cancel(self: Pin<&mut LinkPairing>);
    }

    impl cxx_qt::Threading for LinkPairing {}
    impl cxx_qt::Initialize for LinkPairing {}
}

/// How long the service keeps a pairing window open, in seconds.
const WINDOW: u64 = 120;

pub struct PairingRust {
    state: QString,
    code: QString,
    qr_url: QUrl,
    expires_at: i64,
    paired_name: QString,
    failure: QString,
}

impl Default for PairingRust {
    fn default() -> Self {
        Self {
            state: QString::from("idle"),
            code: QString::default(),
            qr_url: QUrl::default(),
            expires_at: 0,
            paired_name: QString::default(),
            failure: QString::default(),
        }
    }
}

impl cxx_qt::Initialize for qobject::LinkPairing {
    fn initialize(self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        hub::subscribe(move |event| {
            if let Event::Pairing { .. } | Event::Paired(_) | Event::PairingFailed(_) = event {
                let event = event.clone();
                let _ = thread.queue(move |pairing| pairing.apply(event));
            }
        });
    }
}

/// An SVG of the QR code, one unit per module with a four-module margin, dark modules on transparent.
fn qr_svg(uri: &str) -> Option<String> {
    let code = qrcode::QrCode::new(uri.as_bytes()).ok()?;
    let width = code.width();
    let side = width + 8;
    let mut path = String::new();
    for (index, color) in code.to_colors().iter().enumerate() {
        if *color == qrcode::Color::Dark {
            let _ = write!(path, "M{} {}h1v1h-1z", index % width + 4, index / width + 4);
        }
    }
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {side} {side}\" shape-rendering=\"crispEdges\">\
         <path fill=\"#000\" d=\"{path}\"/></svg>"
    ))
}

/// Writes the QR image of the URI to the cache and returns its URL; empty when it could not.
fn qr_url(uri: &str, code: &str) -> QUrl {
    let written = cache("pairing").and_then(|folder| {
        let svg = qr_svg(uri).ok_or_else(|| std::io::Error::other("the URI does not fit a QR code"))?;
        let file = folder.join(format!("qr-{code}.svg"));
        std::fs::write(&file, svg)?;
        Ok(file)
    });
    match written {
        Ok(file) => QUrl::from(&*file_url(&file.to_string_lossy())),
        Err(error) => {
            log::warn!("pairing QR: {error}");
            QUrl::default()
        }
    }
}

impl qobject::LinkPairing {
    fn apply(mut self: Pin<&mut Self>, event: Event) {
        match event {
            Event::Pairing { code, uri } => {
                self.as_mut().set_code(QString::from(&code));
                self.as_mut().set_qr_url(qr_url(&uri, &code));
                self.as_mut().set_expires_at(i64::try_from(now() + WINDOW).unwrap_or(i64::MAX));
                self.as_mut().set_failure(QString::default());
                self.set_state(QString::from("waiting"));
            }
            Event::Paired(name) => {
                self.as_mut().set_paired_name(QString::from(&name));
                self.set_state(QString::from("paired"));
            }
            Event::PairingFailed(reason) => {
                self.as_mut().set_failure(QString::from(&reason));
                self.set_state(QString::from("failed"));
            }
            _ => {}
        }
    }

    fn start(self: Pin<&mut Self>) {
        hub::send(Command::Pair);
    }

    fn cancel(mut self: Pin<&mut Self>) {
        hub::send(Command::CancelPairing);
        self.as_mut().set_state(QString::from("idle"));
    }
}

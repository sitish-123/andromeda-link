//! The desktop's clipboard, default apps, and folders, through Qt.

#[cxx_qt::bridge]
pub mod ffi {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("link-desk/src/desktop.h");
        #[cxx_name = "setClipboardText"]
        fn set_clipboard_text(text: &QString);
        /// Puts the local files on the clipboard as URLs, for pasting in a file manager.
        #[cxx_name = "setClipboardFiles"]
        fn set_clipboard_files(paths: &QStringList);
        /// Opens the file in its default app; false when none took it.
        #[cxx_name = "openPath"]
        fn open_path(path: &QString) -> bool;
        #[cxx_name = "downloadsFolder"]
        fn downloads_folder() -> QString;
        #[cxx_name = "quitApp"]
        fn quit_app();
        /// Keeps the app running after its last window closes, for the tray icon.
        #[cxx_name = "keepRunningWithoutWindows"]
        fn keep_running_without_windows();
    }
}

use std::path::PathBuf;

use cxx_qt_lib::{QString, QStringList};

use crate::client::{Command, Event, Fetch, cache};
use crate::hub;

/// Copies the current phone's files here: Downloads to save, the cache to copy or open.
pub fn fetch(page: &'static str, paths: &QStringList, purpose: Fetch) {
    let folder = match purpose {
        Fetch::Save => Ok(PathBuf::from(ffi::downloads_folder().to_string())),
        Fetch::Copy | Fetch::Open => cache("files"),
    };
    let folder = match folder {
        Ok(folder) => folder,
        Err(error) => return hub::publish(Event::Failed(format!("No folder for the files: {error}"))),
    };
    let paths: Vec<String> = paths.iter().map(ToString::to_string).collect();
    if !paths.is_empty() {
        hub::to_phone(|device| Command::Fetch { device, paths, folder, page, purpose });
    }
}

/// Finishes a fetch: puts copies on the clipboard or opens them. Returns the folder they are in.
pub fn fetched(purpose: Fetch, paths: &[PathBuf]) -> QString {
    let list: QStringList = paths.iter().map(|path| QString::from(&*path.to_string_lossy())).collect();
    match purpose {
        Fetch::Copy => ffi::set_clipboard_files(&list),
        Fetch::Open => {
            for path in list.iter() {
                if !ffi::open_path(path) {
                    hub::publish(Event::Failed(format!("No app opens {path}")));
                }
            }
        }
        Fetch::Save => {}
    }
    let folder = paths.first().and_then(|path| path.parent()).map(|folder| folder.to_string_lossy().into_owned());
    QString::from(&folder.unwrap_or_default())
}

use cxx_qt_build::{CxxQtBuilder, QResource, QResources, QmlFile, QmlModule};
use qt_build_utils::QResourceFile;

/// Preview tools for the QML, kept out of the shipped module.
const PREVIEW_ONLY: [&str; 2] = ["Preview.qml", "Shot.qml"];
/// Bridges of the page singletons and the desktop helpers; cxx-qt takes bridges from one folder only.
const PAGES: [&str; 8] = ["activity", "desktop", "files", "messages", "pairing", "photos", "screen", "settings"];
const FONTS: &str = "../../apps/android/app/src/main/res/font";

fn main() {
    println!("cargo::rerun-if-changed=qml");
    let mut qml = Vec::new();
    let mut resources = QResource::new();
    for dir in ["qml", "qml/icons"] {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut paths: Vec<_> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
        paths.sort();
        for path in paths {
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
            match path.extension().and_then(|extension| extension.to_str()) {
                Some("qml") if !PREVIEW_ONLY.contains(&name) && !name.starts_with("Mock") => {
                    let singleton = std::fs::read_to_string(&path).is_ok_and(|text| text.contains("pragma Singleton"));
                    qml.push(QmlFile::from(path).singleton(singleton));
                }
                Some("svg" | "png") => resources = resources.file(path),
                _ => {}
            }
        }
    }
    for weight in ["regular", "medium", "semibold", "bold"] {
        let name = format!("source_sans_3_{weight}.ttf");
        println!("cargo::rerun-if-changed={FONTS}/{name}");
        resources = resources.file(QResourceFile::new(format!("{FONTS}/{name}")).alias(format!("qml/fonts/{name}")));
    }
    CxxQtBuilder::new_qml_module(QmlModule::new("org.andromeda.Link").qml_files(qml))
        .qt_module("Quick")
        .qrc_resources(QResources::new().resource(resources))
        .file("src/link.rs")
        .files(PAGES.map(|page| format!("src/{page}.rs")))
        .build();
}

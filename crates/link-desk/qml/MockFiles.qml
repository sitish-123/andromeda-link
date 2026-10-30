import QtQuick
import org.andromeda.Link

QtObject {
    property string path: "/storage/emulated/0/Download"
    property var crumbs: [{ name: "Internal storage", path: "/storage/emulated/0" }, { name: "Download", path: path }]
    property bool loading: false
    property string error: ""
    property ListModel entries: ListModel {
        ListElement { name: "Boarding passes"; path: "/d/Boarding passes"; folder: true; size: 0; sizeText: ""; modifiedText: "Yesterday"; icon: "folder" }
        ListElement { name: "Invoices"; path: "/d/Invoices"; folder: true; size: 0; sizeText: ""; modifiedText: "26 Oct"; icon: "folder" }
        ListElement { name: "Telegram"; path: "/d/Telegram"; folder: true; size: 0; sizeText: ""; modifiedText: "21 Oct"; icon: "folder" }
        ListElement { name: "quarterly-report-final.pdf"; path: "/d/q.pdf"; folder: false; size: 860000; sizeText: "840 KB"; modifiedText: "Today 14:02"; icon: "document" }
        ListElement { name: "IMG_2041.jpg"; path: "/d/IMG_2041.jpg"; folder: false; size: 3100000; sizeText: "3.1 MB"; modifiedText: "Today 12:40"; icon: "image" }
        ListElement { name: "VID_20261029_184411.mp4"; path: "/d/VID.mp4"; folder: false; size: 48200000; sizeText: "48.2 MB"; modifiedText: "Yesterday 18:44"; icon: "video" }
        ListElement { name: "meeting-notes.txt"; path: "/d/notes.txt"; folder: false; size: 2000; sizeText: "2 KB"; modifiedText: "Yesterday 09:15"; icon: "document" }
        ListElement { name: "podcast-episode-212.mp3"; path: "/d/pod.mp3"; folder: false; size: 36700000; sizeText: "36.7 MB"; modifiedText: "19 Oct"; icon: "audio" }
        ListElement { name: "photos-2025.zip"; path: "/d/photos.zip"; folder: false; size: 1400000; sizeText: "1.4 GB"; modifiedText: "12 Oct 2025"; icon: "archive" }
    }

    signal saved(int count, string folder)

    function cd(p) { console.log("cd", p) }
    function up() {}
    function reload() {}
    function save(paths) { saved(paths.length, "Downloads") }
    function openFile(p) { console.log("openFile", p) }
}

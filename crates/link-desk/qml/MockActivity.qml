import QtQuick
import org.andromeda.Link

QtObject {
    property string filter: "all"
    property int fileCount: 6
    property int clipCount: 2

    property ListModel items: ListModel {}
    Component.onCompleted: [
        { id: "1", kind: "file", name: "holiday-videos.zip", detail: "Receiving from OnePlus 12", timeText: "Now", day: "Today", dayDetail: "Fri 30 Oct", state: "active", progress: 0.32, sizeText: "38 MB of 120 MB", incoming: true, path: "", thumbUrl: "" },
        { id: "2", kind: "photo", name: "IMG_2041.jpg", detail: "Received from OnePlus 12", timeText: "2 min", day: "Today", dayDetail: "Fri 30 Oct", state: "done", progress: 1, sizeText: "3.1 MB", incoming: true, path: "/home/u/Downloads/IMG_2041.jpg", thumbUrl: "" },
        { id: "3", kind: "clip", name: "meeting moved to 4 pm, room 3B on the second floor", detail: "Copied from OnePlus 12", timeText: "5 min", day: "Today", dayDetail: "Fri 30 Oct", state: "done", progress: 1, sizeText: "", incoming: true, path: "", thumbUrl: "" },
        { id: "4", kind: "file", name: "scan-0042.pdf", detail: "OnePlus 12 wants to send this", timeText: "8 min", day: "Today", dayDetail: "Fri 30 Oct", state: "offered", progress: 0, sizeText: "1.2 MB", incoming: true, path: "", thumbUrl: "" },
        { id: "5", kind: "file", name: "quarterly-report-final.pdf", detail: "Sent to OnePlus 12", timeText: "1 h", day: "Today", dayDetail: "Fri 30 Oct", state: "done", progress: 1, sizeText: "840 KB", incoming: false, path: "/home/u/quarterly-report-final.pdf", thumbUrl: "" },
        { id: "6", kind: "file", name: "backup-2026-10.tar", detail: "Failed, not enough space on OnePlus 12", timeText: "18:02", day: "Yesterday", dayDetail: "Thu 29 Oct", state: "failed", progress: 0, sizeText: "1.2 GB", incoming: false, path: "", thumbUrl: "" },
        { id: "7", kind: "clip", name: "https://hetzner.com/invoices/2041", detail: "Copied to OnePlus 12", timeText: "09:15", day: "Yesterday", dayDetail: "Thu 29 Oct", state: "done", progress: 1, sizeText: "", incoming: false, path: "", thumbUrl: "" },
        { id: "8", kind: "file", name: "boarding-pass.pdf", detail: "Received from OnePlus 12", timeText: "21:44", day: "Wednesday", dayDetail: "Wed 28 Oct", state: "done", progress: 1, sizeText: "210 KB", incoming: true, path: "/home/u/Downloads/boarding-pass.pdf", thumbUrl: "" }
    ].forEach(r => items.append(r))

    function accept(id) { console.log("accept", id) }
    function decline(id) { console.log("decline", id) }
    function cancel(id) { console.log("cancel", id) }
    function open(id) { console.log("open", id) }
    function showInFolder(id) { console.log("showInFolder", id) }
    function copyAgain(id) { console.log("copyAgain", id) }
}

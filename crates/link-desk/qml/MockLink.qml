import QtQuick
import org.andromeda.Link

QtObject {
    id: mock

    property bool serviceRunning: true
    readonly property real now: Date.now() / 1000
    property var phones: [
        { id: "op12", name: "OnePlus 12", connected: connected, lastSeen: lastSeen },
        { id: "px8", name: "Pixel 8", connected: false, lastSeen: now - 30 * 3600 }
    ]
    property string currentPhone: "op12"
    property bool connected: true
    property real lastSeen: now - 12 * 60
    property int battery: 82
    property bool charging: true
    property string transport: connected ? "Wi-Fi" : ""
    property string network: "Home"
    property int signal: 4
    property string ringer: "ring"
    property bool dnd: false
    property int mediaVolume: 62
    property string playerTitle: "Midnight City"
    property string playerArtist: "M83"
    property string playerApp: "Spotify"
    property url playerArt
    property bool playing: true
    property real positionMs: 134000
    property real lengthMs: 243000
    property int playerVolume: 70
    property bool hasPlayer: true

    property ListModel notifications: ListModel {
        ListElement { key: "maya"; app: "WhatsApp"; title: "Maya"; body: "Are you still coming tonight? I can save you a seat near the front if you text me before eight."; ageText: "2 min"; iconUrl: "icons/msg.svg"; canReply: true }
        ListElement { key: "jonas"; app: "WhatsApp"; title: "Jonas"; body: "Do you have the keys for the van?"; ageText: "6 min"; iconUrl: "icons/msg.svg"; canReply: true }
        ListElement { key: "gmail"; app: "Gmail"; title: "Invoice 2041 from Hetzner"; body: "Your invoice for October is ready. The amount is charged to your card on 3 November."; ageText: "40 min"; iconUrl: "icons/log.svg"; canReply: false }
        ListElement { key: "cal"; app: "Calendar"; title: "Standup in 15 min"; body: "Room 3B, second floor"; ageText: "1 h"; iconUrl: "icons/bell.svg"; canReply: false }
        ListElement { key: "rev"; app: "Revolut"; title: "You paid 12.40 EUR"; body: "Rewe, Hauptstrasse 14"; ageText: "3 h"; iconUrl: "icons/shield.svg"; canReply: false }
    }
    property ListModel recent: ListModel {
        ListElement { name: "IMG_2041.jpg"; detail: "Received, 3.1 MB"; ageText: "2 min"; thumbUrl: ""; kind: "photo" }
        ListElement { name: "meeting moved to 4 pm, room 3B on the second floor"; detail: "Copied from OnePlus 12"; ageText: "5 min"; thumbUrl: ""; kind: "clip" }
        ListElement { name: "quarterly-report-final.pdf"; detail: "Sent, 840 KB"; ageText: "1 h"; thumbUrl: ""; kind: "file" }
    }

    signal failed(string message)
    signal raiseRequested()

    function ring() { console.log("ring") }
    property string hotspotAsk: ""
    function answerHotspot(allow) { console.log("answerHotspot " + allow); hotspotAsk = "" }
    function sendClipboard() { console.log("sendClipboard") }
    function sendFiles(urls) { console.log("sendFiles", urls) }
    function setRinger(mode) { ringer = mode }
    function setDnd(on) { dnd = on }
    function setMediaVolume(v) { mediaVolume = v }
    function playPause() { playing = !playing }
    function next() { positionMs = 0 }
    function previous() { positionMs = 0 }
    function seek(ms) { positionMs = ms }
    function setPlayerVolume(v) { playerVolume = v }
    function reply(key, text) { console.log("reply", key, text) }
    function dismiss(key) {
        for (let i = 0; i < notifications.count; i++)
            if (notifications.get(i).key === key)
                return notifications.remove(i)
    }
    function clearNotifications() { notifications.clear() }
    function connectPhone() { connected = true }
    function startService() { serviceRunning = true }
}

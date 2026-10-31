import QtQuick
import Qt.labs.platform as Platform
import org.andromeda.Link

// The macOS menu bar item: the same menu as the Linux tray icon (src/tray.rs).
Platform.SystemTrayIcon {
    id: tray
    required property var link
    required property string phoneName
    readonly property bool paired: (link.phones || []).length > 0
    readonly property bool live: link.serviceRunning && link.connected
    readonly property string headline: {
        if (!paired)
            return "No phone paired"
        if (live)
            return link.battery >= 0 ? phoneName + ", " + link.battery + "%" : phoneName
        const seen = Theme.since(link.lastSeen)
        if (seen === "just now")
            return phoneName + ", last seen just now"
        return seen ? phoneName + ", last seen " + seen : phoneName + ", not connected"
    }
    signal openRequested()

    visible: true
    icon.source: Qt.resolvedUrl("icons/menubar.svg")
    icon.mask: true
    tooltip: headline

    menu: Platform.Menu {
        Platform.MenuItem { text: tray.headline; enabled: false }
        Platform.MenuSeparator {}
        Platform.MenuItem { text: "Ring"; enabled: tray.live; onTriggered: tray.link.ring() }
        Platform.MenuItem { text: "Send clipboard"; enabled: tray.live; onTriggered: tray.link.sendClipboard() }
        Platform.MenuItem {
            text: "Do not disturb"
            enabled: tray.live
            checkable: true
            checked: tray.link.dnd
            onTriggered: {
                tray.link.setDnd(!tray.link.dnd)
                // A click toggles the item itself; the phone's answer sets it.
                checked = Qt.binding(() => tray.link.dnd)
            }
        }
        Platform.Menu {
            title: "Ringer"
            enabled: tray.live
            Platform.MenuItemGroup { id: ringer }
            Platform.MenuItem { text: "Ring"; group: ringer; checkable: true; checked: tray.link.ringer === "ring"; onTriggered: tray.setRinger(this, "ring") }
            Platform.MenuItem { text: "Vibrate"; group: ringer; checkable: true; checked: tray.link.ringer === "vibrate"; onTriggered: tray.setRinger(this, "vibrate") }
            Platform.MenuItem { text: "Silent"; group: ringer; checkable: true; checked: tray.link.ringer === "silent"; onTriggered: tray.setRinger(this, "silent") }
        }
        Platform.MenuSeparator {}
        Platform.MenuItem { text: "Open Link"; onTriggered: tray.openRequested() }
        Platform.MenuItem { text: "Quit"; onTriggered: Qt.quit() }
    }


    function setRinger(item, mode) {
        link.setRinger(mode)
        item.checked = Qt.binding(() => tray.link.ringer === mode)
    }
}

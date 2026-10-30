import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

ColumnLayout {
    id: tab
    required property var link
    required property var api
    required property var phone
    readonly property var granted: api.grants[phone.id] || []
    readonly property var here: [
        { f: "clipboard", name: "Clipboard", icon: "clip" },
        { f: "files", name: "Files", icon: "file" },
        { f: "notifications", name: "Notifications", icon: "bell" },
        { f: "media", name: "Media", icon: "vol" },
        { f: "input", name: "Input", icon: "pad" },
        { f: "commands", name: "Commands", icon: "term" },
        { f: "system", name: "Control system", icon: "sysctl" }
    ]
    readonly property var there: [
        { f: "ring", name: "Ring", icon: "ping" },
        { f: "calls", name: "Calls", icon: "call" },
        { f: "browse", name: "Browse", icon: "folder" },
        { f: "messages", name: "Messages", icon: "msg" },
        { f: "screen", name: "Screen", icon: "screen" }
    ]

    spacing: 0

    RowLayout {
        Layout.fillWidth: true
        Layout.topMargin: 24
        spacing: 16
        Rectangle {
            width: 56
            height: 56
            radius: 16
            color: tab.phone.connected ? Theme.acc : Theme.tert
            Icon {
                anchors.centerIn: parent
                glyph: "phone"
                size: 26
                color: tab.phone.connected ? Theme.accText : Theme.t2
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            Label {
                Layout.fillWidth: true
                text: tab.phone.name
                font.pixelSize: 24
                font.weight: Font.Bold
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                text: tab.phone.connected
                      ? (tab.phone.id === tab.link.currentPhone && tab.link.transport ? "Connected over " + tab.link.transport + "." : "Connected.")
                      : tab.phone.lastSeen > 0 ? "Not connected. Last seen " + Theme.since(tab.phone.lastSeen) + "." : "Not connected."
                font.pixelSize: Theme.body
                color: Theme.t2
            }
        }
        TonalButton {
            // Unpairing cannot be undone from here, so it takes a second click.
            property string armed
            kind: "warn"
            radius: 20
            text: armed === tab.phone.id ? "Click again to unpair" : "Unpair " + tab.phone.name
            onClicked: armed === tab.phone.id ? tab.api.unpair(tab.phone.id) : armed = tab.phone.id
        }
    }

    Label {
        Layout.fillWidth: true
        Layout.topMargin: 16
        visible: (tab.api.notificationsOff || []).indexOf(tab.phone.id) >= 0
        text: "Notification access is off on " + tab.phone.name + ". Turn it on in the phone's settings to see its notifications here."
        leftPadding: 16
        rightPadding: 16
        topPadding: 10
        bottomPadding: 10
        wrapMode: Text.WordWrap
        font.pixelSize: Theme.small
        color: Theme.t2
        background: Rectangle { radius: 12; color: Qt.alpha(Theme.warn, 0.12) }
    }

    Heading { text: "What " + tab.phone.name + " can do here" }
    Tiles { features: tab.here }
    Heading { text: "What this computer does on " + tab.phone.name }
    Tiles { features: tab.there }

    Heading { text: "Files" }
    SettingRow {
        Layout.maximumWidth: 520
        divider: false
        title: "Accept files automatically"
        text: "Files from " + tab.phone.name + " go to the Downloads folder without a prompt"
        LinkSwitch {
            checkable: false
            checked: (tab.api.autoAccept || []).indexOf(tab.phone.id) >= 0
            text: "Accept files automatically"
            onClicked: tab.api.setAutoAccept(tab.phone.id, !checked)
        }
    }

    Heading { text: "Clipboard" }
    SettingRow {
        Layout.maximumWidth: 520
        divider: false
        title: "Clipboard helper"
        text: tab.api.helperStatus || "Sends what you copy on " + tab.phone.name + " here without root or Shizuku. Needs wireless debugging on the phone; runs until the phone restarts."
        TonalButton {
            radius: 20
            enabled: tab.api.helperStatus !== "Starting…"
            text: "Start helper"
            onClicked: tab.api.startHelper(tab.phone.name)
        }
    }

    component Heading: Label {
        Layout.topMargin: 24
        Layout.bottomMargin: 8
        font.pixelSize: Theme.label
        font.weight: Font.DemiBold
    }
    component Tiles: GridLayout {
        id: tiles
        property var features
        Layout.fillWidth: true
        columns: width > 760 ? 4 : 3
        columnSpacing: 8
        rowSpacing: 8
        Repeater {
            model: tiles.features
            GrantTile {
                required property var modelData
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                text: modelData.name
                iconName: modelData.icon
                on: tab.granted.indexOf(modelData.f) >= 0
                onClicked: tab.api.setGrant(tab.phone.id, modelData.f, !on)
            }
        }
    }
}

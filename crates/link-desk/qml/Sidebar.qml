import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Rectangle {
    id: side
    required property var link
    property int page: 0
    readonly property var pages: [
        { name: "Home", icon: "home" },
        { name: "Messages", icon: "msg" },
        { name: "Calls", icon: "call" },
        { name: "Photos", icon: "photo" },
        { name: "Files", icon: "folder" },
        { name: "Screen", icon: "screen" },
        { name: "Activity", icon: "swap" }
    ]
    readonly property int settingsPage: pages.length
    readonly property var phone: (link.phones || []).find(p => p.id === link.currentPhone) ?? null
    readonly property var others: (link.phones || []).filter(p => p.id !== link.currentPhone)

    implicitWidth: Theme.sidebarWidth
    color: Theme.bg

    Rectangle {
        anchors.right: parent.right
        width: 1
        height: parent.height
        color: Theme.b2
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: 24
        anchors.leftMargin: 16
        anchors.rightMargin: 17
        anchors.bottomMargin: 16
        spacing: 0

        Item {
            Layout.fillWidth: true
            implicitHeight: 48

            Rectangle {
                x: 4
                width: 24
                height: 24
                radius: 8
                color: side.link.connected ? Theme.acc : "transparent"
                border.width: side.link.connected ? 0 : 1.5
                border.color: Theme.b1
                Icon {
                    anchors.centerIn: parent
                    glyph: "phone"
                    size: 14
                    color: side.link.connected ? Theme.accText : Theme.t3
                }
            }
            Column {
                x: 40
                width: parent.width - 40
                Label {
                    width: parent.width
                    text: side.phone ? side.phone.name : "No phone"
                    font.pixelSize: Theme.name
                    font.weight: Font.Bold
                    lineHeight: 24
                    lineHeightMode: Text.FixedHeight
                    elide: Text.ElideRight
                }
                Row {
                    spacing: 8
                    height: 20
                    BatteryGlyph {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: side.link.battery >= 0
                        level: side.link.battery
                        live: side.link.connected
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: side.link.battery >= 0
                        text: side.link.battery + "%"
                        font.pixelSize: Theme.small
                        font.features: Theme.tabular
                        color: Theme.t2
                    }
                    Label {
                        anchors.verticalCenter: parent.verticalCenter
                        text: side.link.connected ? side.link.transport : "Not connected"
                        font.pixelSize: Theme.small
                        font.weight: side.link.connected ? Font.DemiBold : Font.Normal
                        color: side.link.connected ? Theme.acc : Theme.t3
                    }
                }
            }
        }

        Column {
            Layout.fillWidth: true
            Layout.topMargin: 32
            Layout.leftMargin: -4
            Repeater {
                model: side.pages
                NavRow {
                    required property var modelData
                    required property int index
                    width: parent.width
                    text: modelData.name
                    iconName: modelData.icon
                    current: side.page === index
                    onClicked: side.page = index
                }
            }
        }

        Item { Layout.fillHeight: true }

        Repeater {
            model: side.others
            AbstractButton {
                id: other
                required property var modelData
                Layout.fillWidth: true
                implicitHeight: 56
                text: modelData.name
                Accessible.description: modelData.connected ? "Connected" : "Last seen " + Theme.since(modelData.lastSeen)
                onClicked: side.link.currentPhone = modelData.id
                background: Rectangle { radius: 10; color: other.hovered ? Theme.hover : "transparent" }
                contentItem: RowLayout {
                    spacing: 12
                    Icon { Layout.leftMargin: 4; glyph: "phone"; color: Theme.t3 }
                    Column {
                        Layout.fillWidth: true
                        Label { text: other.text; font.pixelSize: Theme.body; font.weight: Font.DemiBold }
                        Label { text: other.Accessible.description; font.pixelSize: Theme.meta; color: Theme.t3 }
                    }
                    Icon { glyph: "chev"; color: Theme.t3 }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.topMargin: 4
            implicitHeight: 1
            color: Theme.b2
        }
        NavRow {
            Layout.fillWidth: true
            Layout.topMargin: 8
            leftPadding: 4
            text: "Settings"
            iconName: "gear"
            current: side.page === side.settingsPage
            onClicked: side.page = side.settingsPage
        }
    }

    component NavRow: AbstractButton {
        id: nav
        property string iconName
        property bool current

        implicitHeight: 40
        leftPadding: 40
        Accessible.role: Accessible.PageTab
        Accessible.selected: current

        background: Rectangle {
            radius: 10
            color: nav.current ? Theme.wash : nav.hovered ? Theme.hover : "transparent"
        }
        contentItem: Row {
            spacing: 12
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                glyph: nav.iconName
                color: nav.current ? Theme.t1 : Theme.t3
            }
            Label {
                anchors.verticalCenter: parent.verticalCenter
                text: nav.text
                font.pixelSize: Theme.label
                font.weight: nav.current ? Font.DemiBold : Font.Normal
                color: nav.current ? Theme.t1 : Theme.t2
            }
        }
    }
}

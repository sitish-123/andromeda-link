import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Dialogs
import QtQuick.Layouts
import org.andromeda.Link

Flickable {
    id: home
    required property var link
    property string phoneName
    readonly property bool live: link.serviceRunning && link.connected

    contentHeight: col.implicitHeight + 24
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: ScrollBar {}

    ColumnLayout {
        id: col
        x: 32
        y: 24
        width: home.width - 64
        spacing: 0

        Row {
            Layout.preferredHeight: 20
            spacing: 24
            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                visible: home.link.battery >= 0
                BatteryGlyph {
                    anchors.verticalCenter: parent.verticalCenter
                    level: home.link.battery
                    live: home.live
                }
                StateText {
                    text: home.link.battery + "%"
                    font.weight: home.live ? Font.DemiBold : Font.Normal
                    font.features: Theme.tabular
                    color: home.live ? Theme.t1 : Theme.t3
                }
                StateText { visible: home.live && home.link.charging; text: "charging" }
            }
            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                visible: home.link.ringer !== ""
                Icon { anchors.verticalCenter: parent.verticalCenter; glyph: "bell"; color: home.live ? Theme.t2 : Theme.t3 }
                StateText {
                    text: ({ ring: "Ringer on", vibrate: "Vibrate only", silent: "Silent" })[home.link.ringer] ?? ""
                }
            }
            Row {
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                Icon { anchors.verticalCenter: parent.verticalCenter; glyph: "dnd"; color: home.live ? Theme.t2 : Theme.t3 }
                StateText { text: home.link.dnd ? "Do not disturb on" : "Do not disturb off" }
            }
        }

        PlayerCard {
            Layout.fillWidth: true
            Layout.topMargin: 16
            visible: home.live && home.link.hasPlayer
            link: home.link
        }
        DisconnectedCard {
            Layout.fillWidth: true
            Layout.topMargin: 16
            visible: !home.live
            link: home.link
            phoneName: home.phoneName
        }

        GridLayout {
            Layout.fillWidth: true
            Layout.topMargin: 16
            columns: 3
            columnSpacing: 8
            enabled: home.live
            uniformCellWidths: true
            ActionButton { Layout.fillWidth: true; Layout.minimumWidth: 0; Layout.preferredWidth: 1; text: "Ring phone"; iconName: "ping"; onClicked: home.link.ring() }
            ActionButton { Layout.fillWidth: true; Layout.minimumWidth: 0; Layout.preferredWidth: 1; text: "Send clipboard"; iconName: "clip"; onClicked: home.link.sendClipboard() }
            ActionButton { Layout.fillWidth: true; Layout.minimumWidth: 0; Layout.preferredWidth: 1; text: "Send files"; iconName: "file"; onClicked: files.open() }
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: 16
            spacing: 0
            enabled: home.live

            RowItem {
                iconName: "bell"
                text: "Ringer"
                divider: false
                Segmented {
                    options: [{ label: "Ring", value: "ring" }, { label: "Vibrate", value: "vibrate" }, { label: "Silent", value: "silent" }]
                    current: home.link.ringer
                    onPicked: v => home.link.setRinger(v)
                }
            }
            RowItem {
                iconName: "dnd"
                text: "Do not disturb"
                LinkSwitch {
                    text: "Do not disturb"
                    checked: home.link.dnd
                    onToggled: home.link.setDnd(checked)
                }
            }
            Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: Theme.b2 }
            RowLayout {
                Layout.topMargin: 12
                Label {
                    Layout.fillWidth: true
                    text: "Media volume"
                    font.pixelSize: Theme.label
                    color: enabled ? Theme.t1 : Theme.dis
                }
                Label {
                    visible: home.link.mediaVolume >= 0
                    text: volume.value
                    font.pixelSize: Theme.label
                    font.weight: Font.DemiBold
                    font.features: Theme.tabular
                    color: enabled ? Theme.t1 : Theme.dis
                }
            }
            LinkSlider {
                id: volume
                Layout.fillWidth: true
                Layout.topMargin: 8
                enabled: home.live && home.link.mediaVolume >= 0
                value: Math.max(0, home.link.mediaVolume)
                Accessible.name: "Media volume"
                onMoved: home.link.setMediaVolume(value)
            }
        }

        RowLayout {
            Layout.topMargin: 24
            Layout.bottomMargin: 4
            visible: recentList.count > 0
            Label {
                Layout.fillWidth: true
                text: "Recent"
                font.pixelSize: Theme.label
                font.weight: Font.DemiBold
            }
            TextButton {
                visible: home.live
                text: "See all"
                onClicked: home.seeAll()
            }
        }
        Repeater {
            id: recentList
            model: home.link.recent
            RecentRow {
                Layout.fillWidth: true
                visible: index < 3
            }
        }
    }

    signal seeAll()

    FileDialog {
        id: files
        fileMode: FileDialog.OpenFiles
        title: "Send files to " + home.phoneName
        onAccepted: home.link.sendFiles(selectedFiles)
    }

    component StateText: Label {
        anchors.verticalCenter: parent.verticalCenter
        font.pixelSize: Theme.body
        color: home.live ? Theme.t2 : Theme.t3
    }

    component RecentRow: Item {
        id: rr
        required property int index
        required property string name
        required property string detail
        required property string ageText
        required property url thumbUrl
        required property string kind

        implicitHeight: 56

        Rectangle {
            visible: rr.index > 0
            x: 52
            width: parent.width - 52
            height: 1
            color: Theme.b2
        }
        Art {
            visible: rr.kind === "photo"
            anchors.verticalCenter: parent.verticalCenter
            width: 40
            height: 40
            radius: 10
            source: rr.thumbUrl
        }
        Rectangle {
            visible: rr.kind !== "photo"
            anchors.verticalCenter: parent.verticalCenter
            width: 40
            height: 40
            radius: 10
            color: Theme.tert
            Icon {
                anchors.centerIn: parent
                glyph: rr.kind === "clip" ? "clip" : "file"
            }
        }
        Column {
            x: 52
            width: parent.width - x - age.width - 12
            anchors.verticalCenter: parent.verticalCenter
            Label {
                width: parent.width
                text: rr.name
                font.pixelSize: Theme.label
                font.weight: Font.Medium
                elide: Text.ElideRight
            }
            Label {
                width: parent.width
                text: rr.detail
                font.pixelSize: Theme.small
                color: Theme.t2
                elide: Text.ElideRight
            }
        }
        Label {
            id: age
            anchors.right: parent.right
            y: 10
            text: rr.ageText
            font.pixelSize: Theme.meta
            font.features: Theme.tabular
            color: Theme.t3
        }
    }
}

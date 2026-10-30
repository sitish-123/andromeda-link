import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

// The mirror shows in andromeda-link-mirror's own window; this page holds its controls.
Item {
    id: page
    required property var link
    required property var api
    required property var settings
    property string phoneName
    readonly property bool live: link.serviceRunning && link.connected
    readonly property bool granted: (settings.grants[link.currentPhone] || []).indexOf("screen") >= 0
    readonly property bool mirroring: live && api.state === "live"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 32
        anchors.topMargin: 24
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            Label {
                Layout.fillWidth: true
                text: "Screen"
                font.pixelSize: Theme.title
                font.weight: Font.DemiBold
                Accessible.role: Accessible.Heading
            }
            Row {
                visible: page.mirroring
                spacing: 8
                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: Theme.ok }
                Label { text: "Live"; font.pixelSize: Theme.small; color: Theme.t2 }
            }
        }
        DisconnectedCard {
            Layout.fillWidth: true
            Layout.topMargin: 16
            visible: !page.live
            link: page.link
            phoneName: page.phoneName
        }
        OffCard {
            Layout.fillWidth: true
            Layout.topMargin: 16
            visible: page.live && !page.granted
            title: "Screen mirroring is off on " + page.phoneName
            text: "Turn it on to see and use the phone's screen from this computer."
            onTurnOn: page.settings.setGrant(page.link.currentPhone, "screen", true)
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            Rectangle {
                id: phone
                anchors.centerIn: parent
                anchors.verticalCenterOffset: page.live && page.granted ? -40 : 0
                height: Math.min(parent.height - (page.live && page.granted ? 140 : 40), 440)
                width: height * 0.47
                radius: 32
                color: page.mirroring ? Qt.alpha(Theme.acc, 0.08) : Theme.elev
                border.width: page.mirroring ? 3 : 1.5
                border.color: page.mirroring ? Theme.acc : Theme.b1
                Column {
                    anchors.centerIn: parent
                    spacing: 12
                    Icon {
                        anchors.horizontalCenter: parent.horizontalCenter
                        glyph: "screen"
                        size: 24
                        color: page.mirroring ? Theme.acc : Theme.t3
                    }
                    Label {
                        width: phone.width - 40
                        visible: page.mirroring
                        text: page.phoneName + " shows in its own window"
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.WordWrap
                        font.pixelSize: Theme.small
                        color: Theme.t2
                    }
                }
            }

            Hint {
                visible: page.mirroring && x > 0
                anchors.right: phone.left
                anchors.rightMargin: 32
                anchors.verticalCenter: phone.verticalCenter
                iconName: "mouse"
                title: "Mouse"
                text: "Click to tap. Drag to swipe. Scroll to scroll."
            }
            Hint {
                visible: page.mirroring && x + width < parent.width
                anchors.left: phone.right
                anchors.leftMargin: 32
                anchors.verticalCenter: phone.verticalCenter
                iconName: "kbd"
                title: "Keyboard"
                text: "Type to enter text on the phone. Enter adds a new line."
            }

            ColumnLayout {
                visible: page.live && page.granted && !page.mirroring
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.top: phone.bottom
                anchors.topMargin: 16
                spacing: 8
                TonalButton {
                    Layout.alignment: Qt.AlignHCenter
                    implicitHeight: 44
                    radius: 22
                    kind: "primary"
                    iconName: "screen"
                    enabled: page.api.state === "idle"
                    text: page.api.state === "starting" ? "Starting…" : "Mirror the screen"
                    onClicked: page.api.start()
                }
                Label {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.maximumWidth: 360
                    text: page.api.error !== "" && page.api.state === "idle" ? page.api.error
                        : page.phoneName + " asks for permission on its screen. Tap Start now there."
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    font.pixelSize: Theme.small
                    font.weight: page.api.error !== "" && page.api.state === "idle" ? Font.DemiBold : Font.Normal
                    color: page.api.error !== "" && page.api.state === "idle" ? Theme.warn : Theme.t2
                }
            }

            ScreenToolbar {
                visible: page.mirroring
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.bottom: parent.bottom
                api: page.api
            }
        }
    }

    component Hint: Rectangle {
        property string iconName
        property string title
        property string text
        width: 172
        height: hint.implicitHeight + 32
        radius: 16
        color: Theme.elev
        border.width: 1
        border.color: Theme.b2
        Column {
            id: hint
            x: 16
            y: 16
            width: parent.width - 32
            spacing: 4
            Icon { glyph: parent.parent.iconName; color: Theme.acc }
            Label {
                topPadding: 8
                text: parent.parent.title
                font.pixelSize: Theme.body
                font.weight: Font.DemiBold
            }
            Label {
                width: parent.width
                text: parent.parent.text
                wrapMode: Text.WordWrap
                font.pixelSize: Theme.small
                color: Theme.t2
            }
        }
    }
}

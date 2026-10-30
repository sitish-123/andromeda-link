import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

// Floating ink pill: phone keys, then Stop.
Rectangle {
    id: bar
    required property var api

    width: row.implicitWidth + 16
    height: 56
    radius: 28
    color: Theme.t1

    Row {
        id: row
        anchors.centerIn: parent
        spacing: 4
        Key { iconName: "back"; text: "Back"; onClicked: bar.api.back() }
        Key { iconName: "circle"; text: "Home"; onClicked: bar.api.home() }
        Key { iconName: "recents"; text: "Recents"; onClicked: bar.api.recents() }
        Rectangle {
            anchors.verticalCenter: parent.verticalCenter
            width: 1
            height: 24
            color: Qt.alpha(Theme.bg, 0.25)
        }
        Key { iconName: "stop"; text: "Stop"; stop: true; onClicked: bar.api.stop() }
    }

    component Key: AbstractButton {
        id: key
        property string iconName
        property bool stop
        height: 44
        leftPadding: 16
        rightPadding: 16
        font.pixelSize: Theme.body
        font.weight: Font.DemiBold
        Accessible.name: text
        background: Rectangle {
            radius: 22
            color: key.stop ? Theme.warn : key.hovered ? Qt.alpha(Theme.bg, 0.12) : "transparent"
            opacity: key.down ? 0.8 : 1
        }
        contentItem: Row {
            spacing: 8
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                glyph: key.iconName
                size: 16
                color: key.stop ? (Theme.dark ? Theme.bg : Theme.t1) : Theme.bg
            }
            Label {
                anchors.verticalCenter: parent.verticalCenter
                text: key.text
                font: key.font
                color: key.stop ? (Theme.dark ? Theme.bg : Theme.t1) : Theme.bg
            }
        }
    }
}

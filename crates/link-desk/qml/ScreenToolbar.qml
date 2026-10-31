import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

// Floating ink pill: phone keys, the phone's own buttons, then Stop.
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
        Divider {}
        Key { iconName: "vol-down"; text: "Volume down"; iconOnly: true; onClicked: bar.api.press(25) }
        Key { iconName: "vol-up"; text: "Volume up"; iconOnly: true; onClicked: bar.api.press(24) }
        Key { iconName: "power"; text: "Power"; iconOnly: true; onClicked: bar.api.press(26) }
        Key {
            iconName: "screen"
            text: bar.api.screenOff ? "Turn phone screen on" : "Turn phone screen off"
            iconOnly: true
            checked: bar.api.screenOff
            onClicked: bar.api.toggleScreen()
        }
        Divider {}
        Key { iconName: "stop"; text: "Stop"; stop: true; onClicked: bar.api.stop() }
    }

    component Divider: Rectangle {
        anchors.verticalCenter: parent.verticalCenter
        width: 1
        height: 24
        color: Qt.alpha(Theme.bg, 0.25)
    }

    component Key: AbstractButton {
        id: key
        property string iconName
        property bool stop
        property bool iconOnly
        height: 44
        leftPadding: iconOnly ? 14 : 16
        rightPadding: iconOnly ? 14 : 16
        ToolTip.visible: iconOnly && hovered
        ToolTip.text: text
        ToolTip.delay: 500
        font.pixelSize: Theme.body
        font.weight: Font.DemiBold
        Accessible.name: text
        background: Rectangle {
            radius: 22
            color: key.stop ? Theme.warn : key.checked ? Qt.alpha(Theme.bg, 0.24)
                : key.hovered ? Qt.alpha(Theme.bg, 0.12) : "transparent"
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
                visible: !key.iconOnly
                anchors.verticalCenter: parent.verticalCenter
                text: key.text
                font: key.font
                color: key.stop ? (Theme.dark ? Theme.bg : Theme.t1) : Theme.bg
            }
        }
    }
}

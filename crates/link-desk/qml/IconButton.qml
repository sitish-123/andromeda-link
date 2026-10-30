import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

AbstractButton {
    id: control
    property string iconName
    property color fill: "transparent"
    property color iconColor: Theme.t1
    property int iconSize: Theme.icon

    implicitWidth: 40
    implicitHeight: 40
    Accessible.name: text
    display: AbstractButton.IconOnly

    background: Rectangle {
        radius: width / 2
        color: control.fill.a > 0 ? control.fill : control.hovered ? Theme.hover : "transparent"
        opacity: control.down ? 0.85 : 1
    }
    contentItem: Item {
        Icon {
            anchors.centerIn: parent
            glyph: control.iconName
            size: control.iconSize
            color: control.enabled ? control.iconColor : Theme.dis
        }
    }
}

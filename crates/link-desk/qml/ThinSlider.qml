import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

Slider {
    id: control
    property int barHeight: 4
    property bool showHandle: true

    implicitHeight: 16
    padding: 0

    background: Rectangle {
        y: (control.height - height) / 2
        width: control.availableWidth
        height: control.barHeight
        radius: height / 2
        color: Theme.input
        Rectangle {
            width: parent.width * control.visualPosition
            height: parent.height
            radius: parent.radius
            color: control.enabled ? Theme.acc : Theme.dis
        }
    }
    handle: Rectangle {
        visible: control.showHandle || control.hovered || control.pressed
        x: control.visualPosition * control.availableWidth - width / 2
        y: (control.height - height) / 2
        width: 14
        height: 14
        radius: 7
        color: control.enabled ? Theme.acc : Theme.dis
    }
}

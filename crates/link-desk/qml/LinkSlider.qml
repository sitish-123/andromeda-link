import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

Slider {
    id: control
    property string iconName: "vol"

    from: 0
    to: 100
    stepSize: 1
    implicitHeight: 44
    padding: 0

    background: Rectangle {
        radius: 14
        color: control.enabled ? Theme.input : "transparent"
        border.width: control.enabled ? 0 : 1.5
        border.color: Theme.b1
        Rectangle {
            visible: control.enabled && control.visualPosition > 0
            width: Math.max(radius * 2, parent.width * control.visualPosition)
            height: parent.height
            radius: 14
            color: Theme.acc
        }
        Icon {
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            glyph: control.iconName
            color: !control.enabled ? Theme.dis : control.visualPosition * parent.width > 32 ? Theme.accText : Theme.t2
        }
    }
    handle: null
}

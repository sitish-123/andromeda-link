import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

AbstractButton {
    id: control

    checkable: true
    implicitWidth: 44
    implicitHeight: 26
    padding: 0
    Accessible.role: Accessible.CheckBox
    Accessible.name: text

    background: Rectangle {
        radius: 13
        color: !control.enabled ? "transparent" : control.checked ? Theme.acc : Theme.input
        border.width: 1.5
        border.color: control.enabled && control.checked ? Theme.acc : Theme.b1
    }
    contentItem: Item {
        Rectangle {
            width: 17
            height: 17
            radius: 8.5
            y: (parent.height - height) / 2
            x: control.checked ? parent.width - width - 4.5 : 4.5
            color: !control.enabled ? Theme.b1 : control.checked ? Theme.accText : Theme.t3
            Behavior on x { NumberAnimation { duration: 120 } }
        }
    }
}

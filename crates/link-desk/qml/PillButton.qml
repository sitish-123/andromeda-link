import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

AbstractButton {
    id: control

    implicitHeight: 40
    leftPadding: 24
    rightPadding: 24
    font.pixelSize: Theme.body
    font.weight: Font.DemiBold

    background: Rectangle {
        radius: height / 2
        color: control.down ? Qt.darker(Theme.acc, 1.15) : control.hovered ? Qt.lighter(Theme.acc, 1.1) : Theme.acc
    }
    contentItem: Label {
        text: control.text
        font: control.font
        color: Theme.accText
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
}

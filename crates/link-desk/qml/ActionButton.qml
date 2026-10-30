import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

AbstractButton {
    id: control
    property string iconName
    readonly property bool compact: width < 120
    Accessible.name: text

    implicitHeight: 48
    font.pixelSize: Theme.body
    font.weight: Font.DemiBold

    background: Rectangle {
        radius: 14
        color: !control.enabled ? "transparent"
             : control.down ? Qt.alpha(Theme.t1, 0.12)
             : control.hovered ? Qt.alpha(Theme.t1, 0.10) : Theme.wash
        border.width: control.enabled ? 0 : 1.5
        border.color: Theme.b1
    }
    contentItem: Item {
        Row {
            anchors.centerIn: parent
            spacing: 8
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                glyph: control.iconName
                color: control.enabled ? Theme.t1 : Theme.dis
            }
            Label {
                anchors.verticalCenter: parent.verticalCenter
                visible: !control.compact
                width: Math.min(implicitWidth, control.width - 64)
                elide: Text.ElideRight
                text: control.text
                font: control.font
                color: control.enabled ? Theme.t1 : Theme.dis
            }
        }
    }
}

import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

Rectangle {
    id: seg
    property var options: []
    property string current
    signal picked(string value)

    implicitWidth: row.implicitWidth + 6
    implicitHeight: 36
    radius: 12
    color: enabled ? Theme.input : "transparent"
    border.width: enabled ? 0 : 1.5
    border.color: Theme.b1
    Accessible.role: Accessible.PageTabList

    Row {
        id: row
        anchors.fill: parent
        anchors.margins: 3
        Repeater {
            model: seg.options
            AbstractButton {
                id: opt
                required property var modelData
                readonly property bool on: seg.current === modelData.value

                height: row.height
                leftPadding: 16
                rightPadding: 16
                text: modelData.label
                checkable: true
                checked: on
                font.pixelSize: Theme.body
                font.weight: on && seg.enabled ? Font.DemiBold : Font.Medium
                Accessible.role: Accessible.PageTab
                onClicked: seg.picked(modelData.value)

                background: Rectangle {
                    radius: 9
                    color: opt.on && seg.enabled ? Theme.acc : opt.hovered ? Theme.hover : "transparent"
                }
                contentItem: Label {
                    text: opt.text
                    font: opt.font
                    verticalAlignment: Text.AlignVCenter
                    color: !seg.enabled ? Theme.dis : opt.on ? Theme.accText : Theme.t2
                }
            }
        }
    }
}

import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

Rectangle {
    id: seg
    property var options: []
    property string current
    // Quiet tabs: the current one is a raised paper chip, not an accent fill.
    property bool quiet: false
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
                    color: opt.on && seg.enabled ? (seg.quiet ? Theme.bg : Theme.acc) : opt.hovered ? Theme.hover : "transparent"
                    border.width: opt.on && seg.quiet ? 1 : 0
                    border.color: Theme.b2
                }
                contentItem: Row {
                    spacing: 8
                    Label {
                        height: parent.height
                        text: opt.text
                        font: opt.font
                        verticalAlignment: Text.AlignVCenter
                        color: !seg.enabled ? Theme.dis : opt.on ? (seg.quiet ? Theme.t1 : Theme.accText) : Theme.t2
                    }
                    Label {
                        height: parent.height
                        visible: opt.modelData.count !== undefined
                        text: opt.modelData.count ?? ""
                        font.pixelSize: Theme.meta
                        font.features: Theme.tabular
                        verticalAlignment: Text.AlignVCenter
                        opacity: 0.7
                        color: !seg.enabled ? Theme.dis : opt.on ? (seg.quiet ? Theme.t1 : Theme.accText) : Theme.t2
                    }
                }
            }
        }
    }
}

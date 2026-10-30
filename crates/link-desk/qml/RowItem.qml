import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Item {
    id: row
    property string iconName
    property string text
    property bool divider: true
    default property alias trailing: slot.data

    implicitHeight: 48
    Layout.fillWidth: true

    Rectangle {
        visible: row.divider
        width: parent.width
        height: 1
        color: Theme.b2
    }
    Row {
        anchors.verticalCenter: parent.verticalCenter
        spacing: 12
        Icon {
            anchors.verticalCenter: parent.verticalCenter
            glyph: row.iconName
            color: row.enabled ? Theme.t2 : Theme.dis
        }
        Label {
            anchors.verticalCenter: parent.verticalCenter
            text: row.text
            font.pixelSize: Theme.label
            color: row.enabled ? Theme.t1 : Theme.dis
        }
    }
    Item {
        id: slot
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        width: childrenRect.width
        height: childrenRect.height
    }
}

import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Item {
    id: row
    property string title
    property string text
    property bool divider: true
    default property alias trailing: slot.data

    Layout.fillWidth: true
    implicitHeight: Math.max(56, col.implicitHeight + 16)

    Rectangle {
        visible: row.divider
        width: parent.width
        height: 1
        color: Theme.b2
    }
    ColumnLayout {
        id: col
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width - slot.width - 16
        spacing: 0
        Label {
            Layout.fillWidth: true
            text: row.title
            font.pixelSize: Theme.label
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }
        Label {
            Layout.fillWidth: true
            visible: text !== ""
            text: row.text
            font.pixelSize: Theme.small
            color: Theme.t2
            wrapMode: Text.WordWrap
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

import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

ColumnLayout {
    id: empty
    property string iconName
    property string title
    property string text
    property string action
    property bool primary: true
    property int tile: 64
    signal triggered()

    spacing: 0

    Rectangle {
        Layout.alignment: Qt.AlignHCenter
        Layout.bottomMargin: 16
        width: empty.tile
        height: empty.tile
        radius: empty.tile * 0.31
        color: Theme.tert
        Icon {
            anchors.centerIn: parent
            glyph: empty.iconName
            size: empty.tile * 0.44
        }
    }
    Label {
        Layout.alignment: Qt.AlignHCenter
        text: empty.title
        font.pixelSize: Theme.title
        font.weight: Font.DemiBold
        lineHeight: 28
        lineHeightMode: Text.FixedHeight
        Accessible.role: Accessible.Heading
    }
    Label {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 4
        Layout.maximumWidth: 380
        visible: text !== ""
        text: empty.text
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        font.pixelSize: Theme.body
        color: Theme.t2
    }
    PillButton {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 16
        visible: empty.action !== "" && empty.primary
        text: empty.action
        onClicked: empty.triggered()
    }
    TonalButton {
        Layout.alignment: Qt.AlignHCenter
        Layout.topMargin: 16
        visible: empty.action !== "" && !empty.primary
        text: empty.action
        onClicked: empty.triggered()
    }
}

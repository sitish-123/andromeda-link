import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

AbstractButton {
    id: tile
    property string iconName
    property bool on

    implicitHeight: 52
    leftPadding: 12
    rightPadding: 8
    font.pixelSize: Theme.body
    font.weight: Font.DemiBold
    Accessible.role: Accessible.CheckBox
    Accessible.name: text
    Accessible.checked: on

    background: Rectangle {
        radius: 14
        color: tile.on ? Qt.alpha(Theme.acc, tile.hovered ? 0.18 : 0.13) : Qt.alpha(Theme.t1, tile.hovered ? 0.09 : 0.06)
    }
    contentItem: RowLayout {
        spacing: 10
        Icon {
            glyph: tile.iconName
            color: tile.on ? Theme.acc : Theme.t2
        }
        Label {
            Layout.fillWidth: true
            text: tile.text
            font: tile.font
            elide: Text.ElideRight
        }
        LinkSwitch {
            scale: 0.85
            checkable: false
            checked: tile.on
            onClicked: tile.click()
        }
    }
}

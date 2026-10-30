import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

AbstractButton {
    id: cell
    property string path
    property string name
    property url thumbUrl
    property bool video
    property string durationText
    property bool selected

    checkable: true
    checked: selected
    text: name
    Accessible.role: Accessible.CheckBox
    Accessible.name: name

    background: Item {
        Rectangle {
            anchors.fill: parent
            anchors.margins: -6
            visible: cell.selected
            radius: 16
            color: "transparent"
            border.width: 3
            border.color: Theme.acc
        }
        Art {
            anchors.fill: parent
            radius: 10
            gradient: false
            source: cell.thumbUrl
            opacity: cell.hovered && !cell.selected ? 0.9 : 1
        }
        Rectangle {
            x: 8
            y: 8
            width: 24
            height: 24
            radius: 12
            visible: cell.selected || cell.hovered
            color: cell.selected ? Theme.acc : Qt.alpha("black", 0.35)
            border.width: cell.selected ? 0 : 1.5
            border.color: "white"
            Icon {
                anchors.centerIn: parent
                visible: cell.selected
                glyph: "check"
                size: 14
                color: Theme.accText
            }
        }
        Rectangle {
            visible: cell.video
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            anchors.margins: 6
            width: badge.implicitWidth + 12
            height: 20
            radius: 10
            color: Qt.alpha("black", 0.62)
            Row {
                id: badge
                anchors.centerIn: parent
                spacing: 3
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    glyph: "play"
                    size: 10
                    color: "white"
                }
                Label {
                    visible: cell.durationText !== ""
                    text: cell.durationText
                    font.pixelSize: Theme.meta
                    font.weight: Font.DemiBold
                    font.features: Theme.tabular
                    color: "white"
                }
            }
        }
    }
    contentItem: null
}

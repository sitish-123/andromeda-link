import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

AbstractButton {
    id: row
    required property string name
    required property string path
    required property bool folder
    required property string sizeText
    required property string modifiedText
    required property var model
    property bool selected
    property bool first
    property bool deletable
    signal opened()
    signal deleteAsked()

    implicitHeight: 48
    text: name
    Accessible.role: folder ? Accessible.Button : Accessible.CheckBox
    Accessible.checked: selected

    onDoubleClicked: opened()

    TapHandler {
        acceptedButtons: Qt.RightButton
        enabled: row.deletable
        onTapped: menu.popup()
    }
    Menu {
        id: menu
        padding: 6
        background: Rectangle {
            implicitWidth: 180
            radius: 14
            color: Theme.elev
            border.width: 1
            border.color: Theme.b2
        }
        MenuItem {
            text: "Delete"
            onTriggered: row.deleteAsked()
            contentItem: Label {
                leftPadding: 8
                text: parent.text
                font.pixelSize: Theme.body
                color: Theme.warn
            }
            background: Rectangle {
                implicitHeight: 36
                radius: 10
                color: parent.highlighted ? Theme.hover : "transparent"
            }
        }
    }

    background: Item {
        Rectangle {
            visible: !row.first && !row.selected
            x: 50
            width: parent.width - x
            height: 1
            color: Theme.b2
        }
        Rectangle {
            anchors.fill: parent
            radius: 10
            color: row.selected ? Qt.alpha(Theme.acc, 0.14) : row.hovered ? Theme.hover : "transparent"
        }
    }
    contentItem: RowLayout {
        spacing: 16
        Rectangle {
            Layout.leftMargin: 8
            width: 26
            height: 26
            radius: 7
            color: row.selected ? Theme.acc : row.folder ? Qt.alpha(Theme.acc, 0.12) : Theme.tert
            Icon {
                anchors.centerIn: parent
                size: 16
                glyph: row.selected ? "check" : row.folder ? "folder" : row.model.icon === "image" ? "photo"
                     : row.model.icon === "video" ? "video" : row.model.icon === "audio" ? "audio"
                     : row.model.icon === "archive" ? "archive" : "file"
                color: row.selected ? Theme.accText : row.folder ? Theme.acc : Theme.t2
            }
        }
        Label {
            Layout.fillWidth: true
            text: row.name
            font.pixelSize: Theme.label
            font.weight: row.folder ? Font.DemiBold : Font.Medium
            elide: Text.ElideMiddle
        }
        Label {
            Layout.preferredWidth: 96
            horizontalAlignment: Text.AlignRight
            text: row.folder ? "—" : row.sizeText
            font.pixelSize: Theme.small
            font.features: Theme.tabular
            color: Theme.t2
        }
        Label {
            Layout.preferredWidth: 140
            Layout.leftMargin: 16
            text: row.modifiedText
            font.pixelSize: Theme.small
            font.features: Theme.tabular
            color: Theme.t2
            elide: Text.ElideRight
        }
        IconButton {
            Layout.preferredWidth: 32
            Layout.preferredHeight: 32
            opacity: row.deletable ? 1 : 0
            enabled: row.deletable
            iconName: "more"
            text: "More for " + row.name
            iconColor: Theme.t2
            onClicked: menu.popup(this, 0, height)
        }
    }
}

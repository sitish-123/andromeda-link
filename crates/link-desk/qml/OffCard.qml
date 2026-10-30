import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

// A feature the phone has not granted, with "Turn on".
Rectangle {
    id: card
    property string title
    property string text
    signal turnOn()

    implicitHeight: row.implicitHeight + 48
    radius: 20
    color: Theme.elev
    border.width: 1
    border.color: Theme.b2

    RowLayout {
        id: row
        x: 24
        y: 24
        width: Math.min(parent.width - 48, 520)
        spacing: 20

        Rectangle {
            Layout.alignment: Qt.AlignTop
            width: 48
            height: 48
            radius: 14
            color: Qt.alpha(Theme.warn, 0.14)
            Icon {
                anchors.centerIn: parent
                glyph: "lock"
                color: Theme.warn
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            Label {
                Layout.fillWidth: true
                text: card.title
                font.pixelSize: Theme.title
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                text: card.text
                font.pixelSize: Theme.body
                color: Theme.t2
                wrapMode: Text.WordWrap
            }
            PillButton {
                Layout.topMargin: 12
                text: "Turn on"
                onClicked: card.turnOn()
            }
        }
    }
}

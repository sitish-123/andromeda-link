import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Rectangle {
    id: card
    required property var link
    property string phoneName
    readonly property bool serviceOff: !link.serviceRunning

    implicitHeight: 186
    radius: 20
    color: Theme.elev
    border.width: 1
    border.color: Theme.b2

    ColumnLayout {
        anchors.verticalCenter: parent.verticalCenter
        x: 24
        width: Math.min(parent.width - 48, 440)
        spacing: 0

        Label {
            Layout.fillWidth: true
            text: card.serviceOff ? "The Link service is off" : card.phoneName + " is not connected"
            font.pixelSize: Theme.title
            font.weight: Font.DemiBold
            lineHeight: 28
            lineHeightMode: Text.FixedHeight
            elide: Text.ElideRight
        }
        Label {
            visible: !card.serviceOff && card.link.lastSeen > 0
            text: "Last seen " + Theme.since(card.link.lastSeen)
            font.pixelSize: Theme.body
            color: Theme.t2
        }
        Label {
            Layout.fillWidth: true
            Layout.topMargin: 8
            text: card.serviceOff ? "Your phones cannot connect until it runs."
                                  : "Link connects again when both are on the same Wi-Fi network."
            font.pixelSize: Theme.body
            color: Theme.t2
            wrapMode: Text.WordWrap
        }
        PillButton {
            Layout.topMargin: 16
            text: card.serviceOff ? "Turn on" : "Connect"
            onClicked: card.serviceOff ? card.link.startService() : card.link.connectPhone()
        }
    }
}

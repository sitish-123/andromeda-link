import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Rectangle {
    id: card
    required property var link
    property string phoneName
    readonly property bool serviceOff: !link.serviceRunning
    readonly property bool paired: (link.phones || []).length > 0

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
            text: card.serviceOff ? "The Link service is off"
                : card.paired ? card.phoneName + " is not connected" : "Pair your phone"
            font.pixelSize: Theme.title
            font.weight: Font.DemiBold
            lineHeight: 28
            lineHeightMode: Text.FixedHeight
            elide: Text.ElideRight
        }
        Label {
            visible: !card.serviceOff && card.paired && card.link.lastSeen > 0
            text: "Last seen " + Theme.since(card.link.lastSeen)
            font.pixelSize: Theme.body
            color: Theme.t2
        }
        Label {
            Layout.fillWidth: true
            Layout.topMargin: 8
            text: card.serviceOff ? "Your phones cannot connect until it runs."
                  : card.paired ? "Link connects again when the phone is on the same network, or near this computer with Bluetooth on. If it does not, pair again."
                  : "Open Andromeda Link on your phone and tap Pair a desktop. The phone must be on the same network, or near with Bluetooth on."
            font.pixelSize: Theme.body
            color: Theme.t2
            wrapMode: Text.WordWrap
        }
        PillButton {
            Layout.topMargin: 16
            text: card.serviceOff ? "Turn on" : card.paired ? "Pair again" : "Pair a phone"
            onClicked: {
                if (card.serviceOff)
                    card.link.startService()
                else
                    ApplicationWindow.window.pairOpen = true
            }
        }
    }
}

import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

// Full card after pairing: paired, or failed with the service's reason.
Rectangle {
    id: card
    required property var api
    readonly property bool ok: api.state === "paired"
    signal closed()

    radius: 28
    color: ok ? Qt.alpha(Theme.acc, 0.09) : Qt.alpha(Theme.warn, 0.11)

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(parent.width - 64, 520)
        spacing: 12

        Rectangle {
            Layout.alignment: Qt.AlignHCenter
            width: 72
            height: 72
            radius: 22
            color: card.ok ? Theme.acc : Qt.alpha(Theme.warn, 0.18)
            Icon {
                anchors.centerIn: parent
                glyph: card.ok ? "check" : "phone"
                size: 34
                color: card.ok ? Theme.accText : Theme.warn
            }
        }
        Label {
            Layout.fillWidth: true
            text: card.ok ? card.api.pairedName + " is paired" : "Pairing did not finish"
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            font.pixelSize: 32
            font.weight: Font.Bold
            Accessible.role: Accessible.Heading
        }
        Label {
            Layout.fillWidth: true
            text: card.ok ? "It connects to this computer on the same network, or over Bluetooth when it is near."
                          : (card.api.failure ? card.api.failure + ". " : "")
                            + "Check that the phone is on the same network as this computer, or near it with Bluetooth on, then try again."
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            font.pixelSize: 16
            color: Theme.t2
        }
        Row {
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 20
            spacing: 12
            TonalButton {
                implicitHeight: 52
                radius: 26
                leftPadding: 36
                rightPadding: 36
                font.pixelSize: 16
                kind: "primary"
                text: card.ok ? "Done" : "Try again"
                onClicked: card.ok ? card.closed() : card.api.start()
            }
            TonalButton {
                visible: !card.ok
                implicitHeight: 52
                radius: 26
                leftPadding: 36
                rightPadding: 36
                font.pixelSize: 16
                text: "Cancel"
                onClicked: {
                    card.api.cancel()
                    card.closed()
                }
            }
        }
    }
}

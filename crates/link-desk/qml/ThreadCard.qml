import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

// The open thread, or a new conversation when `composing`.
Rectangle {
    id: card
    required property var api
    required property var link
    property bool composing: false
    readonly property bool live: link.serviceRunning && link.connected
    readonly property bool open: api.currentThread !== ""

    radius: 20
    color: Theme.elev
    border.width: 1
    border.color: Theme.b2
    clip: true

    function send() {
        const body = draft.text.trim()
        if (!body || !card.live)
            return
        if (card.composing) {
            const to = address.text.trim()
            if (!to)
                return
            card.api.sendTo(to, body)
            card.composing = false
        } else {
            card.api.send(body)
        }
        draft.clear()
    }

    EmptyState {
        anchors.centerIn: parent
        visible: !card.open && !card.composing
        iconName: "msg"
        title: "Pick a conversation"
        text: "Pick a conversation on the left, or start a new one."
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 1
        spacing: 0
        visible: card.open || card.composing

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 80
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            spacing: 12
            visible: !card.composing
            Avatar {
                initial: card.api.threadName ? card.api.threadName.charAt(0).toUpperCase() : "#"
                seed: card.api.threadName
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                Label {
                    Layout.fillWidth: true
                    text: card.api.threadName
                    font.pixelSize: Theme.title
                    font.weight: Font.Bold
                    elide: Text.ElideRight
                    Accessible.role: Accessible.Heading
                }
                Label {
                    Layout.fillWidth: true
                    text: "SMS, " + card.api.threadAddress
                    font.pixelSize: Theme.small
                    color: Theme.t2
                    elide: Text.ElideRight
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 80
            Layout.leftMargin: 24
            Layout.rightMargin: 16
            spacing: 12
            visible: card.composing
            Label {
                text: "To"
                font.pixelSize: Theme.title
                font.weight: Font.Bold
            }
            TextField {
                id: address
                Layout.fillWidth: true
                placeholderText: "Phone number"
                placeholderTextColor: Theme.t3
                color: Theme.t1
                font.pixelSize: Theme.label
                inputMethodHints: Qt.ImhDialableCharactersOnly
                background: null
            }
            IconButton {
                iconName: "x"
                text: "Cancel"
                onClicked: card.composing = false
            }
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.b2
        }
        Label {
            Layout.fillWidth: true
            visible: !card.live
            text: card.link.lastSeen > 0 ? "Showing texts from " + Theme.since(card.link.lastSeen) + ". Sending needs a connection."
                                         : "Sending needs a connection."
            leftPadding: 24
            topPadding: 8
            bottomPadding: 8
            font.pixelSize: Theme.small
            color: Theme.t2
            background: Rectangle { color: Qt.alpha(Theme.warn, 0.12) }
        }

        Conversation {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: !card.composing
            model: card.api.conversation
        }
        Item {
            Layout.fillHeight: true
            visible: card.composing
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.margins: 16
            Layout.topMargin: 12
            spacing: 8
            TextField {
                id: draft
                Layout.fillWidth: true
                implicitHeight: 52
                leftPadding: 20
                rightPadding: 20
                enabled: card.live
                placeholderText: card.live ? "Text message" : "Sending needs a connection"
                placeholderTextColor: card.live ? Theme.t3 : Theme.dis
                color: Theme.t1
                font.pixelSize: Theme.body
                onAccepted: card.send()
                background: Rectangle {
                    radius: 16
                    color: draft.enabled ? Theme.input : "transparent"
                    border.width: draft.enabled ? 0 : 1.5
                    border.color: Theme.b1
                }
            }
            IconButton {
                implicitWidth: 52
                implicitHeight: 52
                iconName: "send"
                text: "Send"
                enabled: card.live
                iconColor: Theme.accText
                onClicked: card.send()
                background: Rectangle {
                    radius: 16
                    color: parent.enabled ? Theme.acc : "transparent"
                    border.width: parent.enabled ? 0 : 1.5
                    border.color: Theme.b1
                }
            }
        }
    }
}

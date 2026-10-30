import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Rectangle {
    id: pane
    required property var link
    readonly property bool live: link.serviceRunning && link.connected
    property string replyKey
    property string draft

    color: Theme.bg

    Rectangle {
        width: 1
        height: parent.height
        color: Theme.b2
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 25
        anchors.rightMargin: 24
        anchors.topMargin: 24
        spacing: 0

        RowLayout {
            Layout.bottomMargin: 12
            Label {
                Layout.fillWidth: true
                text: "Notifications"
                font.pixelSize: Theme.label
                font.weight: Font.DemiBold
                Accessible.role: Accessible.Heading
            }
            TextButton {
                visible: pane.live && list.count > 0
                text: "Clear all"
                onClicked: pane.link.clearNotifications()
            }
        }
        Label {
            Layout.fillWidth: true
            Layout.topMargin: -4
            Layout.bottomMargin: 4
            visible: !pane.live && list.count > 0
            text: pane.link.lastSeen > 0 ? "Shown as of " + Theme.since(pane.link.lastSeen) + ". Reply needs a connection."
                                         : "Reply needs a connection."
            font.pixelSize: Theme.small
            color: Theme.t3
            wrapMode: Text.WordWrap
        }
        Label {
            visible: list.count === 0
            text: "No notifications"
            font.pixelSize: Theme.body
            color: Theme.t2
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: pane.link.notifications
            section.property: "app"
            ScrollBar.vertical: ScrollBar {}
            delegate: NoteItem {}
        }
    }

    component NoteItem: Item {
        id: item
        required property int index
        required property string key
        required property string app
        required property string title
        required property string body
        required property string ageText
        required property string iconUrl
        required property bool canReply
        readonly property bool first: ListView.section !== ListView.previousSection
        readonly property bool last: ListView.section !== ListView.nextSection
        readonly property bool replying: pane.live && pane.replyKey === key

        width: ListView.view.width
        implicitHeight: content.implicitHeight + (first ? 16 : 12) + (last ? 16 : 0)

        HoverHandler { id: hover }

        Rectangle {
            visible: item.first && item.index > 0
            width: parent.width
            height: 1
            color: Theme.b2
        }

        Rectangle {
            visible: item.first
            y: 16
            width: 36
            height: 36
            radius: 10
            color: Theme.tert
            opacity: pane.live ? 1 : 0.45
            Icon {
                anchors.centerIn: parent
                source: item.iconUrl || Qt.resolvedUrl("icons/bell.svg")
                // The phone sends full-colour app icons; a tint would flatten them to a disc.
                color: item.iconUrl ? "transparent" : Theme.t2
            }
        }

        ColumnLayout {
            id: content
            x: 48
            y: item.first ? 16 : 12
            width: parent.width - 48
            spacing: 0
            opacity: pane.live ? 1 : 0.45

            RowLayout {
                Layout.fillWidth: true
                Layout.preferredHeight: 16
                Label {
                    Layout.fillWidth: true
                    text: item.app
                    font.pixelSize: Theme.meta
                    color: Theme.t3
                    elide: Text.ElideRight
                }
                Label {
                    visible: !dismiss.visible
                    text: item.ageText
                    font.pixelSize: Theme.meta
                    font.features: Theme.tabular
                    color: Theme.t3
                }
                IconButton {
                    id: dismiss
                    visible: pane.live && (hover.hovered || activeFocus)
                    implicitWidth: 16
                    implicitHeight: 16
                    iconSize: 14
                    iconName: "x"
                    iconColor: Theme.t3
                    text: "Dismiss " + item.title
                    onClicked: pane.link.dismiss(item.key)
                }
            }
            Label {
                Layout.fillWidth: true
                Layout.topMargin: 2
                text: item.title
                font.pixelSize: Theme.label
                font.weight: Font.DemiBold
                lineHeight: 20
                lineHeightMode: Text.FixedHeight
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                text: item.body
                font.pixelSize: Theme.body
                color: Theme.t2
                lineHeight: 20
                lineHeightMode: Text.FixedHeight
                wrapMode: Text.Wrap
                maximumLineCount: 2
                elide: Text.ElideRight
            }
            TextButton {
                Layout.topMargin: 4
                visible: pane.live && item.canReply && !item.replying
                text: "Reply"
                Accessible.name: "Reply to " + item.title
                onClicked: {
                    pane.draft = ""
                    pane.replyKey = item.key
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.topMargin: 8
                visible: item.replying
                spacing: 8

                TextField {
                    id: field
                    Layout.fillWidth: true
                    implicitHeight: 40
                    leftPadding: 12
                    rightPadding: 12
                    text: pane.draft
                    placeholderText: "Reply to " + item.title
                    placeholderTextColor: Theme.t3
                    font.pixelSize: Theme.body
                    color: Theme.t1
                    selectionColor: Qt.alpha(Theme.acc, 0.3)
                    selectedTextColor: Theme.t1
                    Accessible.name: "Reply to " + item.title
                    focus: item.replying
                    background: Rectangle {
                        radius: 12
                        color: Theme.elev
                        border.width: 1
                        border.color: field.activeFocus ? Theme.acc : Theme.b1
                    }
                    onAccepted: send.clicked()
                    Keys.onEscapePressed: pane.replyKey = ""
                }
                IconButton {
                    id: send
                    text: "Send reply"
                    iconName: "up"
                    fill: Theme.acc
                    iconColor: Theme.accText
                    enabled: field.text.trim() !== ""
                    onClicked: {
                        pane.link.reply(item.key, field.text.trim())
                        pane.replyKey = ""
                    }
                }
            }
        }
    }
}

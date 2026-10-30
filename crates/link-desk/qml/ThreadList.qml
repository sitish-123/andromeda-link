import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

ColumnLayout {
    id: list
    required property var api
    property bool live: true
    property alias query: search.text
    signal compose()

    spacing: 0

    RowLayout {
        Layout.fillWidth: true
        Layout.leftMargin: 24
        Layout.rightMargin: 16
        Layout.topMargin: 24
        Layout.bottomMargin: 8
        Label {
            Layout.fillWidth: true
            text: "Messages"
            font.pixelSize: 24
            font.weight: Font.Bold
            Accessible.role: Accessible.Heading
        }
        IconButton {
            iconName: "pencil"
            text: "New conversation"
            enabled: list.live
            fill: enabled ? Theme.acc : "transparent"
            iconColor: Theme.accText
            background: Rectangle {
                radius: 14
                color: parent.enabled ? Theme.acc : "transparent"
                border.width: parent.enabled ? 0 : 1.5
                border.color: Theme.b1
            }
            onClicked: list.compose()
        }
    }

    TextField {
        id: search
        Layout.fillWidth: true
        Layout.leftMargin: 16
        Layout.rightMargin: 16
        Layout.bottomMargin: 8
        implicitHeight: 44
        leftPadding: 40
        placeholderText: "Search texts"
        placeholderTextColor: Theme.t3
        color: Theme.t1
        font.pixelSize: Theme.body
        background: Rectangle {
            radius: 14
            color: Theme.input
            border.width: search.activeFocus ? 1.5 : 0
            border.color: Theme.acc
            Icon {
                x: 12
                anchors.verticalCenter: parent.verticalCenter
                glyph: "search"
                color: Theme.t3
            }
        }
    }

    ListView {
        id: view
        Layout.fillWidth: true
        Layout.fillHeight: true
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        model: list.api.threads
        opacity: list.live ? 1 : 0.7
        ScrollBar.vertical: ScrollBar {}

        section.property: "section"
        section.delegate: RowLayout {
            required property string section
            width: ListView.view.width
            height: 44
            Label {
                Layout.leftMargin: 24
                Layout.topMargin: 12
                Layout.fillWidth: true
                text: parent.section
                font.pixelSize: Theme.label
                font.weight: Font.DemiBold
            }
            Label {
                Layout.rightMargin: 24
                Layout.topMargin: 12
                visible: parent.section === "Unread"
                text: list.api.unreadCount
                font.pixelSize: Theme.small
                color: Theme.t3
            }
        }

        delegate: AbstractButton {
            id: row
            required property string thread
            required property string name
            required property string address
            required property string initial
            required property string snippet
            required property string timeText
            required property bool unread
            readonly property bool match: {
                const q = list.query.trim().toLowerCase()
                return !q || (name + " " + address + " " + snippet).toLowerCase().indexOf(q) >= 0
            }
            readonly property bool current: list.api.currentThread === thread

            x: 8
            width: ListView.view.width - 16
            height: match ? 72 : 0
            visible: match
            leftPadding: 12
            rightPadding: 12
            text: name
            Accessible.description: snippet
            onClicked: list.api.currentThread = thread

            background: Rectangle {
                radius: 16
                color: row.current ? Theme.wash : row.hovered ? Theme.hover : "transparent"
            }
            contentItem: RowLayout {
                spacing: 12
                Avatar {
                    initial: row.initial
                    seed: row.name
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    Label {
                        Layout.fillWidth: true
                        text: row.name
                        font.pixelSize: Theme.label
                        font.weight: row.unread ? Font.Bold : Font.Medium
                        elide: Text.ElideRight
                    }
                    Label {
                        Layout.fillWidth: true
                        text: row.snippet
                        maximumLineCount: 1
                        font.pixelSize: Theme.small
                        font.weight: row.unread ? Font.Medium : Font.Normal
                        color: row.unread ? Theme.t1 : Theme.t2
                        elide: Text.ElideRight
                    }
                }
                ColumnLayout {
                    Layout.alignment: Qt.AlignTop
                    Layout.topMargin: 14
                    spacing: 8
                    Label {
                        Layout.alignment: Qt.AlignRight
                        text: row.timeText
                        font.pixelSize: Theme.meta
                        font.features: Theme.tabular
                        color: Theme.t3
                    }
                    Rectangle {
                        Layout.alignment: Qt.AlignRight
                        visible: row.unread
                        width: 10
                        height: 10
                        radius: 5
                        color: Theme.acc
                    }
                }
            }
        }
    }
}

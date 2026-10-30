import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

// Bubbles, oldest first, kept at the bottom like a chat log.
ListView {
    id: view

    clip: true
    spacing: 4
    leftMargin: 24
    rightMargin: 24
    bottomMargin: 8
    topMargin: Math.max(16, height - contentHeight - bottomMargin)
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: ScrollBar {}
    onCountChanged: Qt.callLater(positionViewAtEnd)
    onHeightChanged: Qt.callLater(positionViewAtEnd)

    section.property: "dayText"
    section.delegate: Item {
        required property string section
        width: view.width - 48
        height: 48
        Label {
            anchors.centerIn: parent
            text: parent.section
            leftPadding: 12
            rightPadding: 12
            topPadding: 4
            bottomPadding: 4
            font.pixelSize: Theme.meta
            color: Theme.t3
            background: Rectangle { radius: 12; color: Theme.dark ? Qt.alpha(Theme.t1, 0.08) : Theme.tert }
        }
    }

    delegate: Column {
        id: msg
        required property int index
        required property string body
        required property bool sent
        required property string timeText
        // The time shows once, under the last bubble of a run from one side in one minute.
        readonly property bool last: {
            view.count
            const next = view.itemAtIndex(index + 1)
            return !next || next.sent !== sent || next.timeText !== timeText
        }

        width: view.width - 48
        spacing: 2

        Rectangle {
            readonly property int big: 20
            readonly property int tail: 6
            x: msg.sent ? msg.width - width : 0
            width: Math.min(text.implicitWidth, 420) + 32
            height: text.implicitHeight + 24
            topLeftRadius: big
            topRightRadius: big
            bottomLeftRadius: msg.sent ? big : tail
            bottomRightRadius: msg.sent ? tail : big
            color: msg.sent ? Theme.t1 : Theme.dark ? Qt.alpha(Theme.t1, 0.08) : Theme.tert

            Text {
                id: text
                x: 16
                y: 12
                width: Math.min(implicitWidth, 420)
                text: msg.body
                wrapMode: Text.Wrap
                textFormat: Text.PlainText
                font.family: Theme.family
                font.pixelSize: Theme.label
                lineHeight: 20
                lineHeightMode: Text.FixedHeight
                color: msg.sent ? Theme.bg : Theme.t1
            }
        }
        Label {
            visible: msg.last
            x: msg.sent ? msg.width - width - 4 : 4
            topPadding: 2
            bottomPadding: 8
            text: (msg.sent ? "Sent " : "") + msg.timeText
            font.pixelSize: Theme.meta
            font.features: Theme.tabular
            color: Theme.t3
        }
    }
}

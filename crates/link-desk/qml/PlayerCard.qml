import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Rectangle {
    id: card
    required property var link

    implicitHeight: 186
    radius: 20
    color: Theme.elev
    border.width: 1
    border.color: Theme.b2

    Art {
        x: 16
        y: 16
        width: 112
        height: 112
        source: card.link.playerArt
    }

    ColumnLayout {
        x: 148
        y: 16
        width: parent.width - x - 16
        height: 112
        spacing: 0

        Label {
            Layout.fillWidth: true
            text: card.link.playerTitle
            font.pixelSize: Theme.title
            font.weight: Font.DemiBold
            font.letterSpacing: -0.2
            lineHeight: 28
            lineHeightMode: Text.FixedHeight
            elide: Text.ElideRight
        }
        Label {
            Layout.fillWidth: true
            text: [card.link.playerArtist, card.link.playerApp ? "on " + card.link.playerApp : ""].filter(s => s).join(", ")
            font.pixelSize: Theme.body
            color: Theme.t2
            elide: Text.ElideRight
        }
        Item { Layout.fillHeight: true }
        RowLayout {
            Layout.fillWidth: true
            spacing: 16
            IconButton { text: "Previous"; iconName: "prev"; onClicked: card.link.previous() }
            IconButton {
                implicitWidth: 48
                implicitHeight: 48
                text: card.link.playing ? "Pause" : "Play"
                iconName: card.link.playing ? "pause" : "play"
                fill: Theme.t1
                iconColor: Theme.bg
                onClicked: card.link.playPause()
            }
            IconButton { text: "Next"; iconName: "next"; onClicked: card.link.next() }
            Item { Layout.fillWidth: true }
            Icon { glyph: "vol"; color: Theme.t2 }
            ThinSlider {
                Layout.preferredWidth: 88
                from: 0
                to: 100
                stepSize: 1
                barHeight: 3
                showHandle: false
                value: card.link.playerVolume
                Accessible.name: "Player volume"
                onMoved: card.link.setPlayerVolume(value)
            }
        }
    }

    RowLayout {
        x: 16
        y: 146
        width: parent.width - 32
        spacing: 12
        Label {
            text: Theme.clock(seek.pressed ? seek.value : card.link.positionMs)
            font.pixelSize: Theme.meta
            font.features: Theme.tabular
            color: Theme.t3
        }
        ThinSlider {
            id: seek
            Layout.fillWidth: true
            from: 0
            to: Math.max(1, card.link.lengthMs)
            value: card.link.positionMs
            Accessible.name: "Seek"
            onPressedChanged: if (!pressed) card.link.seek(value)
        }
        Label {
            text: Theme.clock(card.link.lengthMs)
            font.pixelSize: Theme.meta
            font.features: Theme.tabular
            color: Theme.t3
        }
    }
}

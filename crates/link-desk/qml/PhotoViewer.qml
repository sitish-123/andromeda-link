import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtMultimedia
import org.andromeda.Link

// Full-size photos and videos over the Photos page, in the grid's order (`api.photos`).
Rectangle {
    id: viewer
    required property var api
    property int index: -1
    property bool info: false
    readonly property int count: pager.count
    readonly property color fg: "#EDEFEF"

    function open(path) {
        index = api.rowOf(path)
        if (index >= 0)
            forceActiveFocus()
    }
    function close() { index = -1 }
    function step(by) { index = Math.max(0, Math.min(count - 1, index + by)) }

    // A lightbox stays dark in both themes: Theme.bg of the dark theme.
    color: "#0D1012"
    visible: index >= 0
    focus: visible
    Keys.onLeftPressed: step(-1)
    Keys.onRightPressed: step(1)
    Keys.onEscapePressed: close()
    onCountChanged: if (index >= count) close()
    onIndexChanged: if (index >= 0) pager.currentIndex = index

    HoverHandler { id: hover }
    MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; onWheel: wheel => wheel.accepted = true }

    ListView {
        id: pager
        anchors.fill: parent
        anchors.topMargin: 64
        anchors.rightMargin: viewer.info ? 280 : 0
        orientation: ListView.Horizontal
        interactive: false
        clip: true
        highlightMoveDuration: 0
        highlightRangeMode: ListView.StrictlyEnforceRange
        // Set, not bound: a model reset moves currentIndex and would break a binding.
        onCountChanged: currentIndex = viewer.index
        model: viewer.visible ? viewer.api.photos : null
        delegate: Item {
            id: slide
            required property int index
            required property string path
            required property string name
            required property url thumbUrl
            required property bool video
            required property string album
            required property string taken
            required property string size
            readonly property bool current: ListView.isCurrentItem
            property string full
            property bool failed
            property real zoom: 1
            property real turn: 0
            property real panX: 0
            property real panY: 0
            readonly property size source: full !== "" && image.status === Image.Ready
                ? Qt.size(image.implicitWidth, image.implicitHeight) : Qt.size(thumb.implicitWidth, thumb.implicitHeight)
            readonly property bool sideways: turn % 180 !== 0
            readonly property real fit: source.width > 0
                ? Math.min(1, (width - 48) / (sideways ? source.height : source.width),
                           (height - 48) / (sideways ? source.width : source.height)) : 1

            width: ListView.view.width
            height: ListView.view.height

            function load() {
                failed = false
                viewer.api.view(path)
            }
            function reset() { zoom = 1; panX = 0; panY = 0 }
            onCurrentChanged: {
                if (current && full === "")
                    load()
                if (!current) {
                    reset()
                    turn = 0
                    player.stop()
                }
            }
            Component.onCompleted: if (current) load()
            onZoomChanged: if (zoom <= 1) { panX = 0; panY = 0 }

            Connections {
                target: viewer.api
                function onViewed(path, url) {
                    if (path !== slide.path)
                        return
                    slide.full = url
                    slide.failed = url === ""
                }
            }
            Connections {
                target: viewer
                enabled: slide.current
                function onTurned(by) { slide.turn = (slide.turn + by + 360) % 360; slide.reset() }
            }

            Item {
                id: frame
                visible: !slide.video
                x: (slide.width - width) / 2 + slide.panX
                y: (slide.height - height) / 2 + slide.panY
                width: slide.source.width * slide.fit * slide.zoom
                height: slide.source.height * slide.fit * slide.zoom
                rotation: slide.turn
                Image {
                    id: thumb
                    anchors.fill: parent
                    source: slide.thumbUrl
                    visible: image.status !== Image.Ready
                }
                Image {
                    id: image
                    anchors.fill: parent
                    asynchronous: true
                    autoTransform: true
                    source: slide.video ? "" : slide.full
                }
            }
            WheelHandler {
                enabled: !slide.video && slide.full !== ""
                onWheel: event => slide.zoom = Math.max(1, Math.min(8 / slide.fit, slide.zoom * Math.pow(1.0015, event.angleDelta.y)))
            }
            PinchHandler {
                enabled: !slide.video && slide.full !== ""
                target: null
                property real startZoom: 1
                onActiveChanged: if (active) startZoom = slide.zoom
                onActiveScaleChanged: slide.zoom = Math.max(1, Math.min(8 / slide.fit, startZoom * activeScale))
            }
            DragHandler {
                enabled: slide.zoom > 1
                target: null
                property real startX
                property real startY
                onActiveChanged: if (active) { startX = slide.panX; startY = slide.panY }
                onTranslationChanged: { slide.panX = startX + translation.x; slide.panY = startY + translation.y }
            }
            TapHandler {
                enabled: !slide.video && slide.full !== ""
                onDoubleTapped: slide.zoom = slide.zoom === 1 ? 1 / slide.fit : 1
            }

            Item {
                anchors.fill: parent
                anchors.margins: 24
                visible: slide.video && slide.full !== ""
                MediaPlayer {
                    id: player
                    source: slide.video && slide.current ? slide.full : ""
                    videoOutput: output
                    audioOutput: AudioOutput {}
                    onSourceChanged: if (source.toString() !== "") play()
                }
                VideoOutput {
                    id: output
                    anchors.fill: parent
                    anchors.bottomMargin: 56
                    rotation: slide.turn
                }
                TapHandler { onTapped: player.playing ? player.pause() : player.play() }
                RowLayout {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.bottom: parent.bottom
                    height: 40
                    spacing: 12
                    IconButton {
                        iconName: player.playing ? "pause" : "play"
                        text: player.playing ? "Pause" : "Play"
                        iconColor: viewer.fg
                        onClicked: player.playing ? player.pause() : player.play()
                    }
                    ThinSlider {
                        Layout.fillWidth: true
                        from: 0
                        to: Math.max(1, player.duration)
                        value: player.position
                        onMoved: player.position = value
                        Accessible.name: "Position"
                    }
                    Label {
                        text: viewer.clock(player.position) + " / " + viewer.clock(player.duration)
                        color: viewer.fg
                        font.pixelSize: Theme.small
                        font.features: Theme.tabular
                    }
                }
            }

            BusyIndicator {
                anchors.centerIn: parent
                running: visible
                visible: !slide.failed && (slide.full === "" || (!slide.video && image.status === Image.Loading))
            }
            ColumnLayout {
                anchors.centerIn: parent
                visible: slide.failed || image.status === Image.Error
                spacing: 12
                Label {
                    Layout.alignment: Qt.AlignHCenter
                    text: slide.video ? "Video did not load" : "Photo did not load"
                    color: viewer.fg
                    font.pixelSize: Theme.label
                    font.weight: Font.DemiBold
                }
                TonalButton {
                    Layout.alignment: Qt.AlignHCenter
                    text: "Try again"
                    onClicked: slide.load()
                }
            }
        }
    }

    signal turned(int by)
    function clock(ms) {
        const s = Math.floor(ms / 1000)
        return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0")
    }
    readonly property var item: pager.currentItem

    RowLayout {
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 12
        height: 40
        spacing: 4
        IconButton {
            iconName: "x"
            text: "Close"
            iconColor: viewer.fg
            onClicked: viewer.close()
        }
        Label {
            Layout.fillWidth: true
            Layout.leftMargin: 8
            text: viewer.item ? viewer.item.name + "  ·  " + viewer.item.taken : ""
            color: viewer.fg
            font.pixelSize: Theme.label
            font.weight: Font.DemiBold
            elide: Text.ElideMiddle
        }
        IconButton {
            iconName: "rotate"
            text: "Rotate left"
            iconColor: viewer.fg
            onClicked: viewer.turned(-90)
        }
        IconButton {
            iconName: "rotate"
            text: "Rotate right"
            iconColor: viewer.fg
            transform: Scale { origin.x: 20; xScale: -1 }
            onClicked: viewer.turned(90)
        }
        IconButton {
            iconName: "info"
            text: "Info"
            iconColor: viewer.info ? Theme.acc : viewer.fg
            onClicked: viewer.info = !viewer.info
        }
        IconButton {
            iconName: "copy"
            text: "Copy"
            iconColor: viewer.fg
            onClicked: viewer.api.copy([viewer.item.path])
        }
        IconButton {
            iconName: "down"
            text: "Save to Downloads"
            iconColor: viewer.fg
            onClicked: viewer.api.save([viewer.item.path])
        }
    }

    IconButton {
        anchors.left: parent.left
        anchors.leftMargin: 16
        anchors.verticalCenter: pager.verticalCenter
        visible: hover.hovered && viewer.index > 0
        iconName: "chev"
        text: "Previous"
        iconColor: viewer.fg
        fill: Qt.alpha("black", 0.45)
        rotation: 180
        onClicked: viewer.step(-1)
    }
    IconButton {
        anchors.right: pager.right
        anchors.rightMargin: 16
        anchors.verticalCenter: pager.verticalCenter
        visible: hover.hovered && viewer.index < viewer.count - 1
        iconName: "chev"
        text: "Next"
        iconColor: viewer.fg
        fill: Qt.alpha("black", 0.45)
        onClicked: viewer.step(1)
    }
    Label {
        anchors.right: pager.right
        anchors.bottom: parent.bottom
        anchors.margins: 16
        text: (viewer.index + 1) + " / " + viewer.count
        color: Qt.alpha(viewer.fg, 0.7)
        font.pixelSize: Theme.small
        font.features: Theme.tabular
    }

    Rectangle {
        visible: viewer.info && viewer.item !== null
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: parent.bottom
        anchors.topMargin: 64
        width: 280
        color: "#161A1D"
        ColumnLayout {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: 20
            spacing: 14
            Repeater {
                model: viewer.item ? [
                    ["Name", viewer.item.name],
                    ["Album", viewer.item.album],
                    ["Taken", viewer.item.taken],
                    ["Size", viewer.item.size],
                    ["Dimensions", viewer.item.video || viewer.item.source.width === 0 || viewer.item.full === ""
                        ? "" : viewer.item.source.width + " × " + viewer.item.source.height]
                ].filter(pair => pair[1] !== "") : []
                ColumnLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: 2
                    Label { text: modelData[0]; color: "#A3AAAD"; font.pixelSize: Theme.meta }
                    Label {
                        Layout.fillWidth: true
                        text: modelData[1]
                        color: viewer.fg
                        font.pixelSize: Theme.body
                        wrapMode: Text.Wrap
                    }
                }
            }
        }
    }
}

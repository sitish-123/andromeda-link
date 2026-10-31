import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtMultimedia
import org.andromeda.Link

// The phone's screen inside the page, as Phone Link shows it: the pointer is a finger, the wheel scrolls, typing and
// shortcuts go to the phone, and F11 shows the screen alone in fullscreen.
Item {
    id: page
    required property var link
    required property var api
    required property var settings
    property string phoneName
    readonly property bool live: link.serviceRunning && link.connected
    readonly property bool granted: (settings.grants[link.currentPhone] || []).indexOf("screen") >= 0
    readonly property bool mirroring: live && api.state === "live"
    property bool full: false

    function toggleFull() {
        const window = page.Window.window
        if (!window)
            return
        full = !full && mirroring
        if (full)
            window.showFullScreen()
        else
            window.showNormal()
        // Reparenting the phone takes focus off its keys.
        Qt.callLater(keys.forceActiveFocus)
    }

    onMirroringChanged: if (!mirroring && full) toggleFull()

    Rectangle {
        id: backdrop
        parent: page.Window.contentItem
        anchors.fill: parent
        z: 1000
        visible: page.full
        color: "black"
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 32
        anchors.topMargin: 24
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            Label {
                Layout.fillWidth: true
                text: "Screen"
                font.pixelSize: Theme.title
                font.weight: Font.DemiBold
                Accessible.role: Accessible.Heading
            }
            Row {
                visible: page.mirroring
                spacing: 8
                Rectangle { anchors.verticalCenter: parent.verticalCenter; width: 8; height: 8; radius: 4; color: Theme.ok }
                Label { text: "Live"; font.pixelSize: Theme.small; color: Theme.t2 }
            }
        }
        DisconnectedCard {
            Layout.fillWidth: true
            Layout.topMargin: 16
            visible: !page.live
            link: page.link
            phoneName: page.phoneName
        }
        OffCard {
            Layout.fillWidth: true
            Layout.topMargin: 16
            visible: page.live && !page.granted
            title: "Screen mirroring is off on " + page.phoneName
            text: "Turn it on to see and use the phone's screen from this computer."
            onTurnOn: page.settings.setGrant(page.link.currentPhone, "screen", true)
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            Rectangle {
                id: phone
                readonly property real aspect: page.api.videoWidth > 0 ? page.api.videoWidth / page.api.videoHeight : 0.47
                anchors.centerIn: parent
                anchors.verticalCenterOffset: page.live && page.granted ? -40 : 0
                height: page.mirroring ? parent.height - 112
                                       : Math.min(parent.height - (page.live && page.granted ? 140 : 40), 440)
                width: height * aspect
                radius: page.mirroring ? 20 : 32
                color: page.mirroring ? "black" : Theme.elev
                border.width: page.mirroring ? 0 : 1.5
                border.color: Theme.b1
                states: State {
                    when: page.full
                    ParentChange { target: phone; parent: backdrop }
                    PropertyChanges {
                        phone.anchors.verticalCenterOffset: 0
                        phone.height: Math.min(backdrop.height, backdrop.width / phone.aspect)
                        phone.radius: 0
                    }
                }
                Icon {
                    anchors.centerIn: parent
                    visible: !page.mirroring
                    glyph: "screen"
                    size: 24
                    color: Theme.t3
                }
                VideoOutput {
                    id: video
                    anchors.fill: parent
                    visible: page.mirroring
                    fillMode: VideoOutput.PreserveAspectFit
                    Component.onCompleted: page.api.setSink(videoSink)
                    Component.onDestruction: page.api.setSink(null)
                }
                MouseArea {
                    id: touch
                    // Phone units: 1/10000 of the picture's width and height.
                    function unit(px, py) {
                        const r = video.contentRect
                        return [Math.round(Math.max(0, Math.min(1, (px - r.x) / r.width)) * 10000),
                                Math.round(Math.max(0, Math.min(1, (py - r.y) / r.height)) * 10000)]
                    }
                    property real moved: 0
                    property point last
                    function finger(action, px, py) {
                        last = Qt.point(px, py)
                        const [x, y] = unit(px, py)
                        page.api.input(action, x, y, 0, 0)
                    }
                    anchors.fill: parent
                    enabled: page.mirroring
                    cursorShape: Qt.PointingHandCursor
                    onPressed: mouse => {
                        keys.forceActiveFocus()
                        touch.moved = Date.now()
                        touch.finger("touch-down", mouse.x, mouse.y)
                    }
                    // At most one move per 8 ms; the release carries the final point.
                    onPositionChanged: mouse => {
                        if (Date.now() - touch.moved >= 8) {
                            touch.moved = Date.now()
                            touch.finger("touch-move", mouse.x, mouse.y)
                        }
                    }
                    onReleased: mouse => touch.finger("touch-up", mouse.x, mouse.y)
                    onCanceled: touch.finger("touch-up", touch.last.x, touch.last.y)
                    // angleDelta is 120 per notch and positive away from the user; the phone takes 100 per notch,
                    // positive right and down.
                    onWheel: wheel => {
                        const [x, y] = unit(wheel.x, wheel.y)
                        const turn = d => Math.max(-10000, Math.min(10000, Math.round(-d / 120 * 100)))
                        page.api.input("wheel", x, y, turn(wheel.angleDelta.x), turn(wheel.angleDelta.y))
                    }
                }
                Item {
                    id: keys
                    focus: page.mirroring
                    onActiveFocusChanged: if (!activeFocus) page.api.releaseKeys()
                    Keys.onPressed: event => {
                        if (event.key === Qt.Key_F11)
                            page.toggleFull()
                        else
                            event.accepted = page.api.key(event.key, event.modifiers, event.text, true)
                    }
                    Keys.onReleased: event => {
                        if (event.key !== Qt.Key_F11)
                            event.accepted = page.api.key(event.key, event.modifiers, event.text, false)
                    }
                }
            }

            Hint {
                visible: page.mirroring && x > 0
                anchors.right: phone.left
                anchors.rightMargin: 32
                anchors.verticalCenter: phone.verticalCenter
                iconName: "mouse"
                title: "Mouse"
                text: "Click to tap. Drag to swipe. Scroll to scroll. F11 for fullscreen."
            }
            Hint {
                visible: page.mirroring && x + width < parent.width
                anchors.left: phone.right
                anchors.leftMargin: 32
                anchors.verticalCenter: phone.verticalCenter
                iconName: "kbd"
                title: "Keyboard"
                text: "Type to enter text on the phone. Shortcuts like Ctrl+C work too. Esc goes back."
            }

            ColumnLayout {
                visible: page.live && page.granted && !page.mirroring
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.top: phone.bottom
                anchors.topMargin: 16
                spacing: 8
                TonalButton {
                    Layout.alignment: Qt.AlignHCenter
                    implicitHeight: 44
                    radius: 22
                    kind: "primary"
                    iconName: "screen"
                    enabled: page.api.state === "idle"
                    text: page.api.state === "starting" ? "Starting…"
                        : page.api.state === "reconnecting" ? "Reconnecting…" : "Mirror the screen"
                    onClicked: page.api.start()
                }
                Label {
                    Layout.alignment: Qt.AlignHCenter
                    Layout.maximumWidth: 360
                    text: page.api.error !== "" && page.api.state === "idle" ? page.api.error
                        : page.phoneName + " asks for permission on its screen. Tap Start now there."
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    font.pixelSize: Theme.small
                    font.weight: page.api.error !== "" && page.api.state === "idle" ? Font.DemiBold : Font.Normal
                    color: page.api.error !== "" && page.api.state === "idle" ? Theme.warn : Theme.t2
                }
            }

            ScreenToolbar {
                visible: page.mirroring
                anchors.horizontalCenter: parent.horizontalCenter
                anchors.bottom: parent.bottom
                api: page.api
            }
        }
    }

    component Hint: Rectangle {
        property string iconName
        property string title
        property string text
        width: 172
        height: hint.implicitHeight + 32
        radius: 16
        color: Theme.elev
        border.width: 1
        border.color: Theme.b2
        Column {
            id: hint
            x: 16
            y: 16
            width: parent.width - 32
            spacing: 4
            Icon { glyph: parent.parent.iconName; color: Theme.acc }
            Label {
                topPadding: 8
                text: parent.parent.title
                font.pixelSize: Theme.body
                font.weight: Font.DemiBold
            }
            Label {
                width: parent.width
                text: parent.parent.text
                wrapMode: Text.WordWrap
                font.pixelSize: Theme.small
                color: Theme.t2
            }
        }
    }
}

import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtMultimedia
import org.andromeda.Link

// The phone's screen inside the page, as Phone Link shows it: click to tap, drag to swipe, the wheel to scroll, and
// typing goes to the phone.
Item {
    id: page
    required property var link
    required property var api
    required property var settings
    property string phoneName
    readonly property bool live: link.serviceRunning && link.connected
    readonly property bool granted: (settings.grants[link.currentPhone] || []).indexOf("screen") >= 0
    readonly property bool mirroring: live && api.state === "live"

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
                    property point from
                    property real at: 0
                    anchors.fill: parent
                    enabled: page.mirroring
                    cursorShape: Qt.PointingHandCursor
                    onPressed: mouse => { from = Qt.point(mouse.x, mouse.y); at = Date.now(); keys.forceActiveFocus() }
                    onReleased: mouse => {
                        const [x, y] = unit(from.x, from.y)
                        const held = Date.now() - at
                        if (Math.hypot(mouse.x - from.x, mouse.y - from.y) < 12)
                            page.api.input(held >= 500 ? "long-press" : "tap", x, y, 0, 0, held)
                        else {
                            const [x2, y2] = unit(mouse.x, mouse.y)
                            page.api.input("swipe", x, y, x2, y2, Math.max(50, Math.min(2000, held)))
                        }
                    }
                    // Content follows the wheel: a notch moves 0.15 of the screen.
                    onWheel: wheel => {
                        const [x, y] = unit(wheel.x, wheel.y)
                        const step = d => Math.max(-10000, Math.min(10000, Math.round(d / 120 * 1500)))
                        page.api.input("scroll", x, y, step(wheel.angleDelta.x), step(wheel.angleDelta.y), 0)
                    }
                }
                Item {
                    id: keys
                    focus: page.mirroring
                    Keys.onPressed: event => {
                        if (event.key === Qt.Key_Escape)
                            page.api.back()
                        else if (event.key === Qt.Key_Backspace)
                            page.api.typeText("\b")
                        else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter)
                            page.api.typeText("\n")
                        else if (event.text !== "" && event.text.charCodeAt(0) >= 32)
                            page.api.typeText(event.text)
                        else
                            return
                        event.accepted = true
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
                text: "Click to tap. Drag to swipe. Scroll to scroll."
            }
            Hint {
                visible: page.mirroring && x + width < parent.width
                anchors.left: phone.right
                anchors.leftMargin: 32
                anchors.verticalCenter: phone.verticalCenter
                iconName: "kbd"
                title: "Keyboard"
                text: "Type to enter text on the phone. Enter adds a new line."
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
                    text: page.api.state === "starting" ? "Starting…" : "Mirror the screen"
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

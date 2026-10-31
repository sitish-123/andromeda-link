import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import QtQuick.Shapes
import org.andromeda.Link

Item {
    id: page
    required property var api
    property bool active
    property real now: Date.now() / 1000
    readonly property bool done: api.state === "paired" || api.state === "failed"
    signal closed()

    onActiveChanged: {
        if (active && api.state !== "waiting")
            api.start()
        else if (!active && api.state === "waiting")
            api.cancel()
    }
    Timer {
        running: page.active && page.api.state === "waiting"
        interval: 1000
        repeat: true
        triggeredOnStart: true
        onTriggered: page.now = Date.now() / 1000
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 32
        anchors.topMargin: 24
        spacing: 8

        AbstractButton {
            id: back
            text: "Settings"
            onClicked: page.closed()
            contentItem: Row {
                spacing: 4
                Icon { anchors.verticalCenter: parent.verticalCenter; glyph: "chev"; size: 16; rotation: 180; color: Theme.acc }
                Label {
                    text: back.text
                    font.pixelSize: Theme.body
                    font.weight: Font.DemiBold
                    font.underline: back.hovered
                    color: Theme.acc
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.maximumHeight: 640
            visible: !page.done
            spacing: 48

            Rectangle {
                Layout.preferredWidth: 440
                Layout.fillHeight: true
                radius: 28
                color: Qt.alpha(Theme.acc, 0.10)
                Rectangle {
                    anchors.centerIn: parent
                    anchors.verticalCenterOffset: -20
                    width: Math.min(344, parent.height - 120)
                    height: width
                    radius: 20
                    color: "white"
                    border.width: 6
                    border.color: Qt.alpha(Theme.acc, 0.3)
                    Image {
                        anchors.fill: parent
                        anchors.margins: 26
                        source: page.api.qrUrl
                        sourceSize: Qt.size(width, height)
                        smooth: false
                    }
                }
                Row {
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: 28
                    spacing: 12
                    Spinner { anchors.verticalCenter: parent.verticalCenter }
                    Label {
                        text: "Waiting for the phone…"
                        font.pixelSize: Theme.label
                        font.weight: Font.DemiBold
                        color: Theme.t2
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.alignment: Qt.AlignTop
                Layout.topMargin: 16
                spacing: 0
                Label {
                    text: "Pair a phone"
                    font.pixelSize: 36
                    font.weight: Font.Bold
                    Accessible.role: Accessible.Heading
                }
                Label {
                    Layout.fillWidth: true
                    Layout.topMargin: 4
                    text: "Scan the code with the phone to link it to this computer."
                    font.pixelSize: 16
                    color: Theme.t2
                    wrapMode: Text.WordWrap
                }
                Step { n: 1; title: "Open Link on the phone" }
                Step { n: 2; title: "Tap Pair a desktop"; detail: "On the same network, or near with Bluetooth on." }
                Step { n: 3; title: "Point the camera at this code" }
                Label {
                    Layout.topMargin: 36
                    text: "No camera? Type this code on the phone"
                    font.pixelSize: Theme.label
                    font.weight: Font.DemiBold
                }
                Row {
                    Layout.topMargin: 12
                    spacing: 8
                    Repeater {
                        model: page.api.code.split("")
                        Item {
                            required property string modelData
                            required property int index
                            // A wider gap splits the six digits in two groups of three.
                            width: 56 + (index === 2 ? 16 : 0)
                            height: 72
                            Rectangle {
                                width: 56
                                height: 72
                                radius: 14
                                color: Theme.input
                                Label {
                                    anchors.centerIn: parent
                                    text: parent.parent.modelData
                                    font.pixelSize: 36
                                    font.weight: Font.Bold
                                    font.features: Theme.tabular
                                }
                            }
                        }
                    }
                }
                Label {
                    Layout.topMargin: 12
                    readonly property int remaining: Math.max(0, Math.round(page.api.expiresAt - page.now))
                    text: "The code works for " + Theme.clock(remaining * 1000) + " more."
                    font.pixelSize: Theme.body
                    font.features: Theme.tabular
                    color: Theme.t2
                }
            }
        }

        PairResult {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.maximumHeight: 640
            visible: page.done
            api: page.api
            onClosed: page.closed()
        }
    }

    component Step: RowLayout {
        property int n
        property string title
        property string detail
        Layout.topMargin: 20
        spacing: 16
        Rectangle {
            Layout.alignment: Qt.AlignTop
            width: 28
            height: 28
            radius: 14
            color: Qt.alpha(Theme.acc, 0.15)
            Label {
                anchors.centerIn: parent
                text: parent.parent.n
                font.pixelSize: Theme.body
                font.weight: Font.Bold
                color: Theme.acc
            }
        }
        Column {
            Label { text: parent.parent.title; font.pixelSize: 16; font.weight: Font.DemiBold }
            Label {
                visible: text !== ""
                text: parent.parent.detail
                font.pixelSize: Theme.body
                color: Theme.t2
            }
        }
    }

    component Spinner: Shape {
        width: 18
        height: 18
        preferredRendererType: Shape.CurveRenderer
        RotationAnimator on rotation { from: 0; to: 360; duration: 900; loops: Animation.Infinite; running: page.active }
        ShapePath {
            strokeColor: Qt.alpha(Theme.acc, 0.22)
            strokeWidth: 2.5
            fillColor: "transparent"
            PathAngleArc { centerX: 9; centerY: 9; radiusX: 7.75; radiusY: 7.75; sweepAngle: 360 }
        }
        ShapePath {
            strokeColor: Theme.acc
            strokeWidth: 2.5
            fillColor: "transparent"
            capStyle: ShapePath.RoundCap
            PathAngleArc { centerX: 9; centerY: 9; radiusX: 7.75; radiusY: 7.75; startAngle: -90; sweepAngle: 90 }
        }
    }
}

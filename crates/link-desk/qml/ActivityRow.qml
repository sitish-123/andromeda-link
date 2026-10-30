import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

// One transfer or clip; the day label sits in the left column on the first row of its day.
Item {
    id: row
    required property var api
    property bool firstOfDay
    required property var model
    required property string kind
    required property string name
    required property string detail
    required property string timeText
    required property string day
    required property string dayDetail
    readonly property string status: model.state
    required property real progress
    required property string sizeText
    required property string path
    required property url thumbUrl
    readonly property bool active: status === "active"
    readonly property bool failed: status === "failed"
    readonly property int dayWidth: 120

    implicitHeight: active ? 88 : 72

    Column {
        visible: row.firstOfDay
        y: 18
        width: 96
        Label {
            width: parent.width
            text: row.day
            font.pixelSize: Theme.title
            font.weight: Font.Bold
            lineHeight: 28
            lineHeightMode: Text.FixedHeight
            fontSizeMode: Text.HorizontalFit
            minimumPixelSize: Theme.label
        }
        Label {
            text: row.dayDetail
            font.pixelSize: Theme.small
            color: Theme.t3
        }
    }

    Rectangle {
        x: row.dayWidth - 16
        width: row.width - x + 16
        height: row.height
        radius: 16
        color: row.active ? Qt.alpha(Theme.acc, 0.10) : row.failed ? Qt.alpha(Theme.warn, 0.09) : "transparent"
    }
    Rectangle {
        visible: !row.firstOfDay && !row.active && !row.failed
        x: row.dayWidth + 64
        width: row.width - x
        height: 1
        color: Theme.b2
    }

    RowLayout {
        x: row.dayWidth
        width: row.width - x
        height: row.height
        spacing: 16

        Item {
            implicitWidth: 48
            implicitHeight: 48
            Art {
                anchors.fill: parent
                visible: !row.failed && row.thumbUrl.toString() !== ""
                radius: 16
                source: row.thumbUrl
            }
            Rectangle {
                anchors.fill: parent
                visible: row.failed || row.thumbUrl.toString() === ""
                radius: 16
                color: row.failed ? Qt.alpha(Theme.warn, 0.14) : Theme.tert
                Icon {
                    anchors.centerIn: parent
                    glyph: row.failed ? "alert" : row.kind === "clip" ? "clip" : row.kind === "photo" ? "photo" : "file"
                    color: row.failed ? Theme.warn : Theme.t2
                }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            Label {
                Layout.fillWidth: true
                text: row.name
                font.pixelSize: 16
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }
            Label {
                Layout.fillWidth: true
                text: row.active ? row.detail + ", " + row.sizeText : row.detail
                font.pixelSize: Theme.small
                font.weight: row.failed ? Font.DemiBold : Font.Normal
                font.features: Theme.tabular
                color: row.failed ? Theme.warn : Theme.t2
                elide: Text.ElideRight
            }
            Rectangle {
                Layout.fillWidth: true
                Layout.topMargin: 6
                visible: row.active
                implicitHeight: 6
                radius: 3
                color: Qt.alpha(Theme.acc, 0.22)
                Rectangle {
                    width: parent.width * Math.max(0, Math.min(1, row.progress))
                    height: parent.height
                    radius: 3
                    color: Theme.acc
                }
            }
        }
        Row {
            spacing: 8
            Pill { visible: row.status === "offered"; text: "Decline"; onClicked: row.api.decline(row.model.id) }
            Pill { visible: row.status === "offered"; text: "Accept"; kind: "ink"; onClicked: row.api.accept(row.model.id) }
            Pill { visible: row.active; text: "Cancel"; onClicked: row.api.cancel(row.model.id) }
            Pill {
                visible: row.status === "done" && row.kind === "clip"
                text: "Copy again"
                onClicked: row.api.copyAgain(row.model.id)
            }
            Pill { visible: row.status === "done" && row.path !== ""; text: "Show in folder"; onClicked: row.api.showInFolder(row.model.id) }
            Pill { visible: row.status === "done" && row.path !== ""; text: "Open"; kind: "ink"; onClicked: row.api.open(row.model.id) }
        }
        Label {
            Layout.preferredWidth: 56
            horizontalAlignment: Text.AlignRight
            text: row.timeText
            font.pixelSize: Theme.meta
            font.features: Theme.tabular
            color: Theme.t3
        }
    }

    component Pill: TonalButton {
        implicitHeight: 36
        radius: 12
    }
}

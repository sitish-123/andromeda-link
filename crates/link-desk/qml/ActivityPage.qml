import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Item {
    id: page
    required property var link
    required property var api
    property string phoneName
    readonly property bool live: link.serviceRunning && link.connected

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 40
        spacing: 0

        Label {
            Layout.topMargin: 32
            text: "Activity"
            font.pixelSize: 32
            font.weight: Font.Bold
            lineHeight: 40
            lineHeightMode: Text.FixedHeight
            Accessible.role: Accessible.Heading
        }
        Segmented {
            Layout.topMargin: 20
            options: [
                { label: "All", value: "all", count: page.api.fileCount + page.api.clipCount },
                { label: "Files", value: "files", count: page.api.fileCount },
                { label: "Clips", value: "clips", count: page.api.clipCount }
            ]
            current: page.api.filter
            onPicked: value => page.api.filter = value
        }
        Label {
            Layout.fillWidth: true
            Layout.topMargin: 16
            Layout.rightMargin: 32
            visible: !page.live
            text: page.phoneName + " is not connected. New transfers and clips wait until it connects."
            leftPadding: 16
            rightPadding: 16
            topPadding: 10
            bottomPadding: 10
            wrapMode: Text.WordWrap
            font.pixelSize: Theme.small
            color: Theme.t2
            background: Rectangle { radius: 12; color: Qt.alpha(Theme.warn, 0.12) }
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.topMargin: 12
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            bottomMargin: 24
            model: page.api.items
            ScrollBar.vertical: ScrollBar {}

            section.property: "day"
            section.delegate: Item {
                required property string section
                width: 1
                height: 28
            }

            // 32 px right gutter; live and failed rows bleed 16 px into it.
            delegate: ActivityRow {
                width: list.width - 32
                api: page.api
                firstOfDay: ListView.section !== ListView.previousSection
            }

            EmptyState {
                anchors.centerIn: parent
                anchors.verticalCenterOffset: -40
                visible: list.count === 0
                iconName: "swap"
                title: page.api.filter === "clips" ? "No clips yet" : page.api.filter === "files" ? "No files yet" : "Nothing here yet"
                text: "Files and clips you share with " + page.phoneName + " show here."
            }
        }
    }
}

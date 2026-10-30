import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

// Day-grouped grid. A hidden Repeater reads the model rows; the visible grid groups them by day.
Flickable {
    id: grid
    required property var model
    property var selected: ({})
    // 8 px inset on each side leaves room for the selection ring.
    readonly property int columns: Math.max(3, Math.round(width / 160))
    readonly property real cell: (width - 16 - 8 * (columns - 1)) / columns
    property var groups: []
    signal toggled(string path)

    function regroup() {
        const out = []
        for (let i = 0; i < rows.count; i++) {
            const r = rows.itemAt(i)
            if (!r)
                continue
            if (!out.length || out[out.length - 1].day !== r.day)
                out.push({ day: r.day, rows: [] })
            out[out.length - 1].rows.push(i)
        }
        groups = out
    }

    contentHeight: col.implicitHeight + 24
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: ScrollBar {}

    Repeater {
        id: rows
        model: grid.model
        onCountChanged: Qt.callLater(grid.regroup)
        delegate: Item {
            required property string path
            required property string name
            required property url thumbUrl
            required property bool video
            required property string durationText
            required property string day
            visible: false
            onDayChanged: Qt.callLater(grid.regroup)
        }
    }

    Column {
        id: col
        x: 8
        width: grid.width - 16
        Repeater {
            model: grid.groups
            Column {
                required property var modelData
                width: col.width
                Label {
                    height: 56
                    topPadding: 24
                    text: parent.modelData.day
                    font.pixelSize: Theme.label
                    font.weight: Font.DemiBold
                }
                Flow {
                    width: parent.width
                    spacing: 8
                    Repeater {
                        model: parent.parent.modelData.rows
                        PhotoCell {
                            required property int modelData
                            readonly property Item row: { grid.groups; return rows.itemAt(modelData) }
                            width: grid.cell
                            height: grid.cell
                            path: row ? row.path : ""
                            name: row ? row.name : ""
                            thumbUrl: row ? row.thumbUrl : ""
                            video: row ? row.video : false
                            durationText: row ? row.durationText : ""
                            selected: !!grid.selected[path]
                            onClicked: grid.toggled(path)
                        }
                    }
                }
            }
        }
    }
}

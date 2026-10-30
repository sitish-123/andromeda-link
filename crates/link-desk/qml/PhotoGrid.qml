import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

// Day-grouped grid. A hidden Repeater reads the model rows; a ListView of day headers and rows of cells shows them,
// so only the cells near the screen exist.
Item {
    id: grid
    required property var model
    property var selected: ({})
    // 8 px inset on each side leaves room for the selection ring.
    readonly property int columns: Math.max(3, Math.round(width / 160))
    readonly property real cell: (width - 16 - 8 * (columns - 1)) / columns
    property var groups: []
    signal toggled(string path)
    // A cell without a thumbnail came on screen.
    signal wanted(string path)

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

    ListView {
        id: list
        anchors.fill: parent
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}
        footer: Item { height: 24 }
        reuseItems: true
        // One line per day header and per row of cells.
        model: {
            const out = []
            for (const g of grid.groups) {
                out.push({ day: g.day, cells: [] })
                for (let i = 0; i < g.rows.length; i += grid.columns)
                    out.push({ day: "", cells: g.rows.slice(i, i + grid.columns) })
            }
            return out
        }
        delegate: Item {
            id: line
            required property var modelData
            x: 8
            width: list.width - 16
            height: modelData.day !== "" ? 56 : grid.cell + 8
            Label {
                visible: line.modelData.day !== ""
                height: 56
                topPadding: 24
                text: line.modelData.day
                font.pixelSize: Theme.label
                font.weight: Font.DemiBold
            }
            Row {
                spacing: 8
                Repeater {
                    model: line.modelData.cells
                    PhotoCell {
                        required property int modelData
                        readonly property Item row: rows.itemAt(modelData)
                        width: grid.cell
                        height: grid.cell
                        path: row ? row.path : ""
                        name: row ? row.name : ""
                        thumbUrl: row ? row.thumbUrl : ""
                        video: row ? row.video : false
                        durationText: row ? row.durationText : ""
                        selected: !!grid.selected[path]
                        onClicked: grid.toggled(path)
                        onPathChanged: if (path !== "" && thumbUrl.toString() === "") grid.wanted(path)
                        Component.onCompleted: if (path !== "" && thumbUrl.toString() === "") grid.wanted(path)
                    }
                }
            }
        }
    }
}

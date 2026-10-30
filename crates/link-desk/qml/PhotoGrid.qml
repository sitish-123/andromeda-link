import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

// Day-grouped grid over the backend's `lines` model: a day header or one row of cells per line, so only the cells
// near the screen exist.
Item {
    id: grid
    required property var model
    property var selected: ({})
    // 8 px inset on each side leaves room for the selection ring.
    readonly property int columns: Math.max(3, Math.round(width / 160))
    readonly property real cell: (width - 16 - 8 * (columns - 1)) / columns
    readonly property alias count: list.count
    signal toggled(string path)
    // A cell without a thumbnail came on screen.
    signal wanted(string path)

    ListView {
        id: list
        anchors.fill: parent
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}
        footer: Item { height: 24 }
        reuseItems: true
        model: grid.model
        delegate: Item {
            id: line
            required property string day
            required property var cells
            // Thumbnail urls by cell; they change apart from `cells`, so a new thumbnail keeps the cells.
            required property var thumbs
            x: 8
            width: list.width - 16
            height: day !== "" ? 56 : grid.cell + 8
            Label {
                visible: line.day !== ""
                height: 56
                topPadding: 24
                text: line.day
                font.pixelSize: Theme.label
                font.weight: Font.DemiBold
            }
            Row {
                spacing: 8
                Repeater {
                    model: line.cells
                    PhotoCell {
                        required property var modelData
                        required property int index
                        width: grid.cell
                        height: grid.cell
                        path: modelData.path
                        name: modelData.name
                        thumbUrl: line.thumbs[index] ?? ""
                        video: modelData.video
                        durationText: modelData.durationText
                        selected: !!grid.selected[path]
                        onClicked: grid.toggled(path)
                        Component.onCompleted: if (thumbUrl.toString() === "") grid.wanted(path)
                    }
                }
            }
        }
    }
}

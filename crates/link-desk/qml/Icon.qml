import QtQuick
import QtQuick.Controls.impl
import org.andromeda.Link

IconImage {
    property string glyph
    property int size: Theme.icon

    source: glyph ? Qt.resolvedUrl("icons/" + glyph + ".svg") : ""
    width: size
    height: size
    sourceSize: Qt.size(size, size)
    color: Theme.t2
}

import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

Column {
    property string title
    property string line: "This page comes in a later update."

    padding: 32
    topPadding: 24
    spacing: 4

    Label {
        text: parent.title
        font.pixelSize: Theme.title
        font.weight: Font.DemiBold
        Accessible.role: Accessible.Heading
    }
    Label {
        text: parent.line
        font.pixelSize: Theme.body
        color: Theme.t2
    }
}

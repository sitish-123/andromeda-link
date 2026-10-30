import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

AbstractButton {
    id: control

    padding: 0
    font.pixelSize: Theme.body
    font.weight: Font.DemiBold
    background: null
    contentItem: Label {
        text: control.text
        font.pixelSize: control.font.pixelSize
        font.weight: control.font.weight
        font.underline: control.hovered
        color: Theme.acc
    }
}

import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

// kind: "tonal" | "primary" | "ink" | "outline" | "warn"
AbstractButton {
    id: control
    property string iconName
    property string kind: "tonal"
    property int radius: 14

    implicitHeight: 40
    implicitWidth: implicitContentWidth + leftPadding + rightPadding
    leftPadding: iconName ? 14 : 16
    rightPadding: 16
    font.pixelSize: Theme.body
    font.weight: Font.DemiBold
    Accessible.name: text

    readonly property color ink: !enabled ? Theme.dis
                                : kind === "primary" ? Theme.accText
                                : kind === "ink" ? Theme.bg
                                : kind === "warn" ? Theme.warn : Theme.t1

    background: Rectangle {
        radius: control.radius
        color: !control.enabled || control.kind === "outline" || control.kind === "warn" ? "transparent"
             : control.kind === "primary" ? Theme.acc
             : control.kind === "ink" ? Theme.t1
             : Qt.alpha(Theme.t1, control.hovered ? 0.11 : 0.07)
        border.width: !control.enabled || control.kind === "outline" || control.kind === "warn" ? 1.5 : 0
        border.color: control.enabled && control.kind === "warn" ? Theme.warn : Theme.b1
        opacity: control.down ? 0.85 : 1
    }
    contentItem: Row {
        spacing: 8
        Icon {
            anchors.verticalCenter: parent.verticalCenter
            visible: control.iconName !== ""
            glyph: control.iconName
            color: control.ink
        }
        Label {
            anchors.verticalCenter: parent.verticalCenter
            text: control.text
            font: control.font
            color: control.ink
        }
    }
}

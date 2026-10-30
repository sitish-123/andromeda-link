import QtQuick
import org.andromeda.Link

Rectangle {
    property int level: -1
    property bool live: true

    implicitWidth: 22
    implicitHeight: 11
    radius: 3
    color: "transparent"
    border.width: 1.5
    border.color: live ? Theme.t2 : Theme.t3

    Rectangle {
        x: 3
        y: 3
        height: parent.height - 6
        width: (parent.width - 6) * Math.max(0, parent.level) / 100
        radius: 1
        visible: parent.live && parent.level >= 0
        color: parent.level <= 15 ? Theme.warn : Theme.ok
    }
}

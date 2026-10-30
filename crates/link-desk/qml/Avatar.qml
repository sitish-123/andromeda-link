import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

// Initial tile; the tint follows the name so a contact keeps its colour.
Rectangle {
    id: avatar
    property string initial
    property string seed
    readonly property int tint: {
        if (initial === "#")
            return 3
        let h = 0
        for (let i = 0; i < seed.length; i++)
            h = (h * 31 + seed.charCodeAt(i)) % 997
        return h % 4
    }

    implicitWidth: 48
    implicitHeight: 48
    radius: width / 3
    color: tint === 0 ? Qt.tint(Theme.bg, Qt.alpha(Theme.acc, 0.14))
         : tint === 1 ? Qt.tint(Theme.bg, Qt.alpha(Theme.ok, 0.18))
         : tint === 2 ? Qt.tint(Theme.bg, Qt.alpha(Theme.warn, 0.18)) : Theme.tert

    Label {
        anchors.centerIn: parent
        text: avatar.initial
        font.pixelSize: Theme.name
        font.weight: Font.Bold
        color: avatar.tint === 3 ? Theme.t2 : Theme.t1
    }
}

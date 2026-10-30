import QtQuick
import org.andromeda.Link

Preview {
    id: win
    property var cases: [[false, true], [true, true], [false, false], [true, false]]
    property int i: 0
    replyKey: "jonas"
    replyDraft: "Yes, on the hook by the door"

    Timer {
        id: first
        interval: 400
        running: true
        onTriggered: {
            const [dark, on] = win.cases[win.i]
            Theme.dark = dark
            win.mock.connected = on
            snap.start()
        }
    }
    Timer {
        id: snap
        interval: 400
        onTriggered: {
            const [dark, on] = win.cases[win.i]
            const name = "/tmp/link-desk-shots/" + (dark ? "dark" : "light") + "-" + (on ? "connected" : "disconnected") + ".png"
            win.body.grabToImage(r => {
                r.saveToFile(name)
                if (++win.i < win.cases.length)
                    first.restart()
                else
                    Qt.quit()
            })
        }
    }
}

import QtQuick
import org.andromeda.Link

QtObject {
    property string state: "idle"
    property string code: "384190"
    property url qrUrl: "file:///tmp/link-desk-pages/qr.svg"
    property real expiresAt: 0
    property string pairedName: "Pixel 8"
    property string failure: "The phone did not answer in 2 minutes"

    function start() {
        state = "waiting"
        expiresAt = Date.now() / 1000 + 102
    }
    function cancel() { state = "idle" }
}

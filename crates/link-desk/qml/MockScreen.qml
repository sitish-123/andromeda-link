import QtQuick
import org.andromeda.Link

QtObject {
    property string state: "live"
    property string error: ""
    property int videoWidth: 1080
    property int videoHeight: 2376

    function start() { state = "live" }
    function stop() { state = "idle" }
    function back() {}
    function home() {}
    function recents() {}
    function keyframe() {}
    function setSink(sink) {}
    function input(action, x, y, x2, y2, ms) {}
    function typeText(text) {}
}

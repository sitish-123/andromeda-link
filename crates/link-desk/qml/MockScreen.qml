import QtQuick
import org.andromeda.Link

QtObject {
    property string state: "live"
    property string error: ""

    function start() { state = "live" }
    function stop() { state = "idle" }
    function back() {}
    function home() {}
    function recents() {}
    function keyframe() {}
}

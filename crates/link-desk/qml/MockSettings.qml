import QtQuick
import org.andromeda.Link

QtObject {
    property var grants: ({
        op12: ["clipboard", "files", "notifications", "media", "input", "commands", "ring", "calls", "browse", "messages", "screen"],
        px8: ["clipboard"]
    })
    property var autoAccept: ["op12"]
    property bool localSendVisible: true
    property bool quickShareVisible: false
    property bool notificationsWhileLocked: false
    property var notificationsOff: []
    property var commands: [
        { name: "Lock screen", command: "loginctl lock-session" },
        { name: "Suspend", command: "systemctl suspend" }
    ]

    function setGrant(id, feature, on) {
        const g = Object.assign({}, grants)
        const list = (g[id] || []).filter(f => f !== feature)
        g[id] = on ? list.concat([feature]) : list
        grants = g
    }
    function setAutoAccept(id, on) { autoAccept = on ? autoAccept.concat([id]) : autoAccept.filter(p => p !== id) }
    function setLocalSendVisible(on) { localSendVisible = on }
    function setQuickShareVisible(on) { quickShareVisible = on }
    function setNotificationsWhileLocked(on) { notificationsWhileLocked = on }
    function setCommands(list) { commands = list }
    property string helperStatus: ""
    function unpair(id) { console.log("unpair", id) }
    function startHelper(name) { helperStatus = "Running until the phone restarts" }
}

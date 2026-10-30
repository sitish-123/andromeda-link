import QtQuick
import QtQuick.Controls.Basic
import org.andromeda.Link

Slider {
    id: control
    property string iconName: "vol"
    // The level the device reports. It moves the slider only while the user does not drag it, and once it comes close
    // to the level last set, so an older report does not pull the slider back.
    property real reported: 0
    property real held: -1
    // A level to set: at most every 100 ms while dragged, and once more on release.
    signal commit(int level)

    Binding on value {
        when: !control.pressed && control.held < 0
        value: control.reported
    }
    onReportedChanged: if (held >= 0 && Math.abs(reported - held) <= 2) held = -1
    onMoved: {
        held = value
        if (live.running) {
            live.dirty = true
        } else {
            commit(value)
            live.start()
        }
    }
    onPressedChanged: if (!pressed) {
        live.stop()
        held = value
        commit(value)
        settle.restart()
    }
    Timer {
        id: live
        property bool dirty
        interval: 100
        onTriggered: if (dirty) {
            dirty = false
            control.commit(control.value)
            restart()
        }
    }
    Timer {
        id: settle
        interval: 1500
        onTriggered: control.held = -1
    }

    from: 0
    to: 100
    stepSize: 1
    implicitHeight: 44
    padding: 0

    background: Rectangle {
        radius: 14
        color: control.enabled ? Theme.input : "transparent"
        border.width: control.enabled ? 0 : 1.5
        border.color: Theme.b1
        Rectangle {
            visible: control.enabled && control.visualPosition > 0
            width: Math.max(radius * 2, parent.width * control.visualPosition)
            height: parent.height
            radius: 14
            color: Theme.acc
        }
        Icon {
            x: 14
            anchors.verticalCenter: parent.verticalCenter
            glyph: control.iconName
            color: !control.enabled ? Theme.dis : control.visualPosition * parent.width > 32 ? Theme.accText : Theme.t2
        }
    }
    handle: null
}

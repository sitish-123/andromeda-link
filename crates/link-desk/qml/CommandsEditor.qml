import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

// Saved commands the phone can run on this computer.
ColumnLayout {
    id: editor
    required property var api
    readonly property var commands: api.commands || []

    spacing: 0

    function add() {
        const n = name.text.trim(), c = command.text.trim()
        if (!n || !c)
            return
        api.setCommands(commands.concat([{ name: n, command: c }]))
        name.clear()
        command.clear()
    }

    Repeater {
        model: editor.commands
        SettingRow {
            id: cmd
            required property var modelData
            required property int index
            divider: index > 0
            title: modelData.name
            text: modelData.command
            IconButton {
                iconName: "x"
                iconColor: Theme.t2
                text: "Remove " + cmd.title
                onClicked: editor.api.setCommands(editor.commands.filter((_, i) => i !== cmd.index))
            }
        }
    }
    Label {
        visible: editor.commands.length === 0
        text: "No commands yet."
        font.pixelSize: Theme.small
        color: Theme.t2
        bottomPadding: 8
    }
    RowLayout {
        Layout.fillWidth: true
        Layout.topMargin: 8
        spacing: 8
        Field { id: name; Layout.preferredWidth: 120; placeholderText: "Name" }
        Field { id: command; Layout.fillWidth: true; placeholderText: "Command"; onAccepted: editor.add() }
        TonalButton {
            iconName: "plus"
            text: "Add"
            enabled: name.text.trim() !== "" && command.text.trim() !== ""
            onClicked: editor.add()
        }
    }

    component Field: TextField {
        implicitHeight: 40
        leftPadding: 12
        color: Theme.t1
        placeholderTextColor: Theme.t3
        font.pixelSize: Theme.body
        background: Rectangle {
            radius: 12
            color: Theme.input
            border.width: parent.activeFocus ? 1.5 : 0
            border.color: Theme.acc
        }
    }
}

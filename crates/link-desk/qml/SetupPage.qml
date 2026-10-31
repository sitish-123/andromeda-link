import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

// After the first pairing: one step per permission this computer still lacks, then what the phone may do here and what
// to set on the phone, as the phone app's own setup does. A step done elsewhere moves on by itself.
Item {
    id: page
    required property var link
    required property var api
    property string phoneName
    readonly property bool mac: Qt.platform.os === "osx"
    readonly property var can: api ? api.sessionCapabilities || [] : []
    readonly property var phone: (link.phones || []).find(p => p.name === phoneName) ?? null
    readonly property var granted: phone && api ? api.grants[phone.id] || [] : []
    readonly property var steps: {
        const all = [
            { f: "input", title: "Let the phone use this computer's mouse and keyboard",
              detail: mac ? "In System Settings, open Privacy & Security, then Accessibility, and turn on Andromeda Link Helper. Lock and media keys need it too."
                          : "This desktop session gives no way to move the pointer or type. Use a Wayland desktop with virtual input, or X11.",
              url: "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility" },
            { f: "screen", title: "Let the phone see this screen",
              detail: mac ? "In System Settings, open Privacy & Security, then Screen & System Audio Recording, and turn on Andromeda Link Helper."
                          : "Remote control needs the desktop's screen-sharing portal and input.",
              url: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture" },
            { f: "notifications", title: "Show the phone's notifications here",
              detail: mac ? "In System Settings, open Notifications, then Andromeda Link Helper, and allow notifications."
                          : "This desktop has no notification service Link can use.",
              url: "x-apple.systempreferences:com.apple.preference.notifications" }
        ]
        // Only what is missing now, fixed when the page opens, so a step granted meanwhile shows as done.
        return all.filter(step => can.indexOf(step.f) < 0)
    }
    readonly property int total: steps.length + 2
    property int index: 0
    signal closed()

    function done(step) { return step && can.indexOf(step.f) >= 0 }

    onVisibleChanged: if (visible) index = 0

    // A permission given in System Settings restarts the helper, which reports it; the page moves on.
    onCanChanged: if (steps && index < steps.length && done(steps[index])) advance.restart()
    Timer { id: advance; interval: 600; onTriggered: if (page.index < page.total - 1) page.index++ }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(parent.width - 64, 560)
        spacing: 0

        Row {
            Layout.alignment: Qt.AlignHCenter
            spacing: 6
            Repeater {
                model: page.total
                Rectangle {
                    required property int index
                    width: index === page.index ? 28 : 10
                    height: 10
                    radius: 5
                    color: index < page.steps.length && page.done(page.steps[index]) ? Theme.acc
                         : index === page.index ? Theme.t2 : Theme.b2
                }
            }
        }

        Label {
            Layout.fillWidth: true
            Layout.topMargin: 28
            text: page.index < page.steps.length ? page.steps[page.index].title
                : page.index === page.steps.length ? "What " + page.phoneName + " can do here"
                : "On the phone"
            wrapMode: Text.WordWrap
            font.pixelSize: 28
            font.weight: Font.Bold
            Accessible.role: Accessible.Heading
        }

        // A permission step.
        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: 12
            visible: page.index < page.steps.length
            spacing: 16
            readonly property var step: page.steps[page.index] ?? null
            Label {
                Layout.fillWidth: true
                text: parent.step ? parent.step.detail : ""
                wrapMode: Text.WordWrap
                font.pixelSize: Theme.body
                color: Theme.t2
            }
            Label {
                text: page.done(parent.step) ? "Done" : "Not allowed yet"
                font.pixelSize: Theme.body
                font.weight: Font.DemiBold
                color: page.done(parent.step) ? Theme.acc : Theme.warn
            }
            PillButton {
                visible: page.mac && !page.done(parent.step)
                text: "Open System Settings"
                onClicked: Qt.openUrlExternally(parent.step.url)
            }
        }

        // What the phone may do on this computer.
        ColumnLayout {
            Layout.fillWidth: true
            Layout.topMargin: 12
            visible: page.index === page.steps.length
            spacing: 8
            Label {
                Layout.fillWidth: true
                text: "Each switch is for this phone only. You can change them later in Settings."
                wrapMode: Text.WordWrap
                font.pixelSize: Theme.body
                color: Theme.t2
            }
            Repeater {
                model: [
                    { f: "clipboard", name: "Clipboard", icon: "clip" },
                    { f: "files", name: "Files", icon: "file" },
                    { f: "notifications", name: "Notifications", icon: "bell" },
                    { f: "media", name: "Media", icon: "vol" },
                    { f: "input", name: "Touchpad, keyboard and remote control", icon: "pad" },
                    { f: "system", name: "Control system: volume, lock, sleep", icon: "sysctl" },
                    { f: "commands", name: "Run your saved commands", icon: "term" }
                ]
                GrantTile {
                    required property var modelData
                    Layout.fillWidth: true
                    enabled: page.phone !== null
                    text: modelData.name
                    iconName: modelData.icon
                    on: page.granted.indexOf(modelData.f) >= 0
                    onClicked: page.api.setGrant(page.phone.id, modelData.f, !on)
                }
            }
        }

        // What only the phone can allow.
        Label {
            Layout.fillWidth: true
            Layout.topMargin: 12
            visible: page.index === page.total - 1
            text: "Open Andromeda Link on " + page.phoneName + " and finish its setup: notification access, photos, and "
                  + "Do Not Disturb access.\n\nOn Xiaomi, Redmi, and POCO phones, also open the app's info, set Battery saver to "
                  + "No restrictions, and turn on Autostart. Otherwise the phone stops Link in the background, and photos, "
                  + "screenshots, and screen requests do not arrive.\n\nWhat you copy on the phone reaches this computer while "
                  + "Link is open there. To send it in the background, turn on the clipboard helper in this phone's settings here."
            wrapMode: Text.WordWrap
            font.pixelSize: Theme.body
            color: Theme.t2
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: 32
            Label {
                Layout.fillWidth: true
                text: (page.index + 1) + " of " + page.total
                font.pixelSize: Theme.body
                font.features: Theme.tabular
                color: Theme.t3
            }
            TonalButton {
                visible: page.index > 0
                text: "Back"
                onClicked: page.index--
            }
            TonalButton {
                kind: "primary"
                text: page.index === page.total - 1 ? "Done" : "Next"
                onClicked: page.index === page.total - 1 ? page.closed() : page.index++
            }
        }
    }
}

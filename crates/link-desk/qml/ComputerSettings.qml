import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

ColumnLayout {
    id: tab
    required property var link
    required property var api
    readonly property bool running: link.serviceRunning
    readonly property var connected: (link.phones || []).filter(p => p.connected).map(p => p.name)
    signal pair()

    spacing: 0

    Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: 24
        implicitHeight: hero.implicitHeight + 48
        radius: 20
        color: tab.running ? Qt.alpha(Theme.ok, 0.11) : Qt.alpha(Theme.warn, 0.13)
        RowLayout {
            id: hero
            x: 24
            y: 24
            width: parent.width - 48
            spacing: 24
            Rectangle {
                Layout.leftMargin: 8
                width: 56
                height: 56
                radius: 28
                color: tab.running ? Theme.ok : Theme.warn
                Rectangle {
                    anchors.centerIn: parent
                    width: 72
                    height: 72
                    radius: 36
                    z: -1
                    color: Qt.alpha(parent.color, 0.2)
                }
                Icon {
                    anchors.centerIn: parent
                    glyph: tab.running ? "bolt" : "pad"
                    size: 26
                    color: Theme.bg
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                Label {
                    Layout.fillWidth: true
                    text: tab.running ? "Link service is running" : "Link service is stopped"
                    font.pixelSize: 24
                    font.weight: Font.Bold
                    lineHeight: 32
                    lineHeightMode: Text.FixedHeight
                }
                Label {
                    Layout.fillWidth: true
                    text: !tab.running ? "Your phones cannot connect until it runs."
                        : tab.connected.length ? "andromeda-linkd. " + tab.connected.join(", ") + (tab.connected.length > 1 ? " are" : " is") + " connected."
                        : "andromeda-linkd. No phone is connected."
                    font.pixelSize: Theme.body
                    color: Theme.t2
                    wrapMode: Text.WordWrap
                }
            }
            PillButton {
                visible: !tab.running
                implicitHeight: 44
                text: "Start"
                onClicked: tab.link.startService()
            }
        }
    }

    GridLayout {
        Layout.fillWidth: true
        Layout.topMargin: 8
        columns: width > 760 ? 2 : 1
        columnSpacing: 48
        rowSpacing: 0

        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignTop
            Layout.preferredWidth: 1
            spacing: 0
            Heading { text: "Discovery" }
            SettingRow {
                title: "Quick Share"
                text: "Visible to nearby Android devices"
                LinkSwitch {
                    checkable: false
                    checked: tab.api.quickShareVisible
                    text: "Quick Share"
                    onClicked: tab.api.setQuickShareVisible(!checked)
                }
            }
            SettingRow {
                divider: false
                title: "LocalSend"
                text: "Receive from LocalSend on this network"
                LinkSwitch {
                    checkable: false
                    checked: tab.api.localSendVisible
                    text: "LocalSend"
                    onClicked: tab.api.setLocalSendVisible(!checked)
                }
            }
            Heading { text: "Notifications" }
            SettingRow {
                divider: false
                title: "Show full text while locked"
                text: "Show the text of a phone notification on the lock screen"
                LinkSwitch {
                    checkable: false
                    checked: tab.api.notificationsWhileLocked
                    text: "Show full text while locked"
                    onClicked: tab.api.setNotificationsWhileLocked(!checked)
                }
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignTop
            Layout.preferredWidth: 1
            spacing: 0
            Heading { text: "Phones" }
            SettingRow {
                divider: false
                title: "Pair a phone"
                text: "Scan a code or type a 6-digit code"
                TonalButton {
                    kind: "primary"
                    radius: 20
                    enabled: tab.running
                    text: "Pair"
                    onClicked: tab.pair()
                }
            }
            Heading { text: "Commands" }
            CommandsEditor {
                Layout.fillWidth: true
                api: tab.api
            }
        }
    }

    component Heading: Label {
        Layout.topMargin: 24
        Layout.bottomMargin: 4
        font.pixelSize: Theme.label
        font.weight: Font.DemiBold
    }
}

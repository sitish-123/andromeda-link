import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

ApplicationWindow {
    id: window
    property var link: typeof Link !== "undefined" ? Link : null
    property alias page: sidebar.page
    property alias replyKey: notes.replyKey
    property alias replyDraft: notes.draft
    property alias body: body
    property bool sheetOpen: false
    readonly property bool narrow: width < 1100
    readonly property bool sheet: narrow && sheetOpen
    readonly property string phoneName: sidebar.phone ? sidebar.phone.name : "Your phone"

    width: 1200
    height: 780
    minimumWidth: 960
    minimumHeight: 640
    visible: true
    title: "Link"
    color: Theme.bg
    font.family: Theme.family
    font.pixelSize: Theme.body
    palette.window: Theme.bg
    palette.windowText: Theme.t1
    palette.text: Theme.t1
    palette.base: Theme.elev
    palette.button: Theme.tert
    palette.buttonText: Theme.t1
    palette.highlight: Theme.acc
    palette.highlightedText: Theme.accText
    palette.mid: Theme.b1
    palette.dark: Theme.t3

    // Relative to this file; the backend must ship these fonts when it loads QML from resources.
    FontLoader { source: "fonts/source_sans_3_regular.ttf" }
    FontLoader { source: "fonts/source_sans_3_medium.ttf" }
    FontLoader { source: "fonts/source_sans_3_semibold.ttf" }
    FontLoader { source: "fonts/source_sans_3_bold.ttf" }

    Connections {
        target: window.link
        function onFailed(message) { toast.show(message) }
    }

    Rectangle {
        id: body
        anchors.fill: parent
        color: Theme.bg

        RowLayout {
            anchors.fill: parent
            spacing: 0
            visible: window.link !== null

            Sidebar {
                id: sidebar
                Layout.fillHeight: true
                Layout.preferredWidth: Theme.sidebarWidth
                link: window.link
            }

            StackLayout {
                id: stack
                Layout.fillHeight: true
                Layout.preferredWidth: Math.min(Theme.homeWidth, window.width - Theme.sidebarWidth - (window.narrow ? 0 : Theme.notesMinWidth))
                currentIndex: sidebar.page

                HomePage {
                    link: window.link
                    phoneName: window.phoneName
                    onSeeAll: sidebar.page = 6
                }
                PlaceholderPage { title: "Messages" }
                PlaceholderPage { title: "Calls" }
                PlaceholderPage { title: "Photos" }
                PlaceholderPage { title: "Files" }
                PlaceholderPage { title: "Screen" }
                PlaceholderPage { title: "Activity" }
                PlaceholderPage { title: "Settings" }
            }

            Item {
                id: notesSlot
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.minimumWidth: window.narrow ? 0 : Theme.notesMinWidth
            }
        }

        IconButton {
            visible: window.narrow && sidebar.page === 0
            x: stack.x + stack.width - 32 - width
            y: 16
            iconName: "bell"
            text: "Notifications, " + unread.count
            onClicked: window.sheetOpen = !window.sheetOpen
            Label {
                visible: unread.count > 0
                x: parent.width - 14
                y: -2
                text: unread.count
                font.pixelSize: Theme.meta
                font.weight: Font.DemiBold
                color: Theme.accText
                leftPadding: 5
                rightPadding: 5
                background: Rectangle { radius: height / 2; color: Theme.acc }
            }
        }
        Repeater { id: unread; model: window.link ? window.link.notifications : null; delegate: Item {} }

        Rectangle {
            anchors.fill: parent
            color: Qt.alpha("black", 0.4)
            opacity: window.sheet ? 1 : 0
            visible: opacity > 0
            Behavior on opacity { NumberAnimation { duration: 150 } }
            MouseArea { anchors.fill: parent; onClicked: window.sheetOpen = false }
        }

        NotificationsPane {
            id: notes
            readonly property int sheetWidth: 360
            x: window.narrow ? parent.width - (window.sheet ? sheetWidth : 0) : notesSlot.x
            y: 0
            width: window.narrow ? sheetWidth : notesSlot.width
            height: parent.height
            visible: !window.narrow || x < parent.width
            color: window.narrow ? Theme.elev : Theme.bg
            link: window.link
            Behavior on x { enabled: window.narrow; NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
            Shortcut { sequence: "Esc"; enabled: window.sheet; onActivated: window.sheetOpen = false }
        }
    }

    // E2E hook: `--shot DIR` saves light/dark shots once the phone is connected with a player and 3 notifications,
    // rings it, then saves the disconnected shots once the service stops, and quits.
    Timer {
        id: shot
        readonly property int arg: Qt.application.arguments.indexOf("--shot")
        readonly property string dir: arg > 0 ? Qt.application.arguments[arg + 1] : ""
        property int step: 0
        property int ticks: 0
        function state() {
            const l = window.link
            return JSON.stringify({ serviceRunning: l.serviceRunning, connected: l.connected, battery: l.battery,
                                    hasPlayer: l.hasPlayer, playerTitle: l.playerTitle,
                                    notifications: l.notifications.rowCount() })
        }
        function save(names, done) {
            Theme.dark = names[0].startsWith("dark")
            body.grabToImage(r => {
                r.saveToFile(dir + "/" + names[0] + ".png")
                console.log("SHOT saved " + names[0])
                names.length > 1 ? save(names.slice(1), done) : done()
            })
        }
        running: arg > 0 && window.link !== null
        interval: 200
        repeat: true
        onTriggered: {
            const l = window.link
            if (++ticks % 25 === 0)
                console.log("SHOT waiting " + step + " " + state())
            if (ticks > 1500) {
                console.log("SHOT timeout " + state())
                Qt.exit(1)
            } else if (step === 0 && l.serviceRunning) {
                console.log("SHOT listening")
                step = 1
            } else if (step === 1 && l.connected && l.hasPlayer && l.notifications.rowCount() >= 3) {
                console.log("SHOT state " + state())
                l.ring()
                step = 2
                save(["light-connected", "dark-connected"], () => { console.log("SHOT connected done"); step = 3 })
            } else if (step === 3 && !l.serviceRunning) {
                console.log("SHOT state " + state())
                step = 4
                save(["light-disconnected", "dark-disconnected"], () => Qt.quit())
            }
        }
    }

    Rectangle {
        id: toast
        property alias text: toastText.text
        function show(message) {
            text = message
            visible = true
            hide.restart()
        }

        visible: false
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 24
        width: toastText.implicitWidth + 32
        height: 40
        radius: 12
        color: Theme.t1
        Label {
            id: toastText
            anchors.centerIn: parent
            color: Theme.bg
            font.pixelSize: Theme.body
        }
        Timer { id: hide; interval: 4000; onTriggered: toast.visible = false }
    }
}

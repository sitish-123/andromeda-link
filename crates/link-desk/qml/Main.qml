import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

ApplicationWindow {
    id: window
    property var link: typeof Link !== "undefined" ? Link : null
    property var messages: typeof LinkMessages !== "undefined" ? LinkMessages : null
    property var calls: typeof LinkCalls !== "undefined" ? LinkCalls : null
    property var activity: typeof LinkActivity !== "undefined" ? LinkActivity : null
    property var photos: typeof LinkPhotos !== "undefined" ? LinkPhotos : null
    property var files: typeof LinkFiles !== "undefined" ? LinkFiles : null
    property var mirror: typeof LinkScreen !== "undefined" ? LinkScreen : null
    property var settings: typeof LinkSettings !== "undefined" ? LinkSettings : null
    property var pairing: typeof LinkPairing !== "undefined" ? LinkPairing : null
    property bool pairOpen: false
    // The phone just paired, whose setup shows; empty when none does.
    property string setupPhone: ""
    property alias page: sidebar.page
    property alias replyKey: notes.replyKey
    property alias replyDraft: notes.draft
    property alias body: body
    property bool sheetOpen: false
    readonly property bool home: page === 0 && !pairOpen && setupPhone === ""
    readonly property bool narrow: width < 1100
    readonly property bool sheet: narrow && sheetOpen
    readonly property int unreadCount: link ? link.notifications.count : 0
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
        function onRaiseRequested() { window.show(); window.raise(); window.requestActivate() }
    }
    Loader {
        active: Qt.platform.os === "osx" && window.link !== null
        sourceComponent: MenuBarIcon {
            link: window.link
            phoneName: window.phoneName
            onOpenRequested: { window.show(); window.raise(); window.requestActivate() }
        }
    }
    Connections {
        target: window.photos
        function onSaved(count, folder) { toast.show("Saved " + count + " to " + folder) }
    }
    Connections {
        target: window.files
        function onSaved(count, folder) { toast.show("Saved " + count + " to " + folder) }
        function onCopied(count, folder) { toast.show("Copied " + count + " to " + folder + " on " + window.phoneName) }
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
                onPageChanged: { window.pairOpen = false; window.setupPhone = "" }
            }

            StackLayout {
                id: stack
                Layout.fillHeight: true
                Layout.fillWidth: !window.home
                Layout.preferredWidth: window.home ? Math.min(Theme.homeWidth, window.width - Theme.sidebarWidth - (window.narrow ? 0 : Theme.notesMinWidth)) : -1
                currentIndex: window.setupPhone !== "" ? 9 : window.pairOpen ? 8 : sidebar.page

                HomePage {
                    link: window.link
                    phoneName: window.phoneName
                    onSeeAll: sidebar.page = 6
                }
                MessagesPage {
                    link: window.link
                    api: window.messages
                    settings: window.settings
                    phoneName: window.phoneName
                }
                CallsPage {
                    link: window.link
                    api: window.calls
                    settings: window.settings
                    phoneName: window.phoneName
                    onDialed: number => toast.show("Calling " + number + " on " + window.phoneName)
                }
                PhotosPage {
                    id: photosPage
                    objectName: "photosPage"
                    link: window.link
                    api: window.photos
                    settings: window.settings
                    phoneName: window.phoneName
                }
                FilesPage {
                    link: window.link
                    api: window.files
                    settings: window.settings
                    phoneName: window.phoneName
                }
                ScreenPage {
                    link: window.link
                    api: window.mirror
                    settings: window.settings
                    phoneName: window.phoneName
                }
                ActivityPage {
                    link: window.link
                    api: window.activity
                    phoneName: window.phoneName
                }
                SettingsPage {
                    objectName: "settingsPage"
                    link: window.link
                    api: window.settings
                    onPair: window.pairOpen = true
                }
                PairPage {
                    api: window.pairing
                    active: window.pairOpen
                    onClosed: {
                        if (window.pairing.state === "paired")
                            window.setupPhone = window.pairing.pairedName
                        window.pairOpen = false
                    }
                }
                SetupPage {
                    link: window.link
                    api: window.settings
                    phoneName: window.setupPhone
                    onClosed: window.setupPhone = ""
                }
            }

            Item {
                id: notesSlot
                visible: window.home
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.minimumWidth: window.narrow ? 0 : Theme.notesMinWidth
            }
        }

        IconButton {
            visible: window.narrow && window.home
            x: stack.x + stack.width - 32 - width
            y: 16
            iconName: "bell"
            text: "Notifications, " + window.unreadCount
            onClicked: window.sheetOpen = !window.sheetOpen
            Label {
                visible: window.unreadCount > 0
                x: parent.width - 14
                y: -2
                text: window.unreadCount
                font.pixelSize: Theme.meta
                font.weight: Font.DemiBold
                color: Theme.accText
                leftPadding: 5
                rightPadding: 5
                background: Rectangle { radius: height / 2; color: Theme.acc }
            }
        }

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
            visible: window.home && (!window.narrow || x < parent.width)
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
        readonly property int pagesArg: Qt.application.arguments.indexOf("--pages")
        readonly property string sendFile: pagesArg > 0 ? Qt.application.arguments[pagesArg + 1] : ""
        property int pageNo: -1
        property int pageTicks: 0
        property int pagesDone: 0
        // One column of a backend ListModel, by its role's position in the Rust ROLES list (from Qt.UserRole).
        function column(model, role) { return [...Array(model.rowCount()).keys()].map(i => model.data(model.index(i, 0), 256 + role)) }
        property var pageList: [
            { name: "messages", index: 1, ready: () => window.messages.threads.rowCount() > 0,
              state: () => ({ threads: window.messages.threads.rowCount() }) },
            { name: "photos", index: 3, ready: () => window.photos.photos.rowCount() > 0 && column(window.photos.photos, 2).every(u => u != ""),
              state: () => ({ photos: window.photos.photos.rowCount(), thumbs: column(window.photos.photos, 2).filter(u => u != "").length, error: window.photos.error }) },
            { name: "files", index: 4, ready: () => window.files.entries.rowCount() > 0,
              state: () => ({ entries: window.files.entries.rowCount(), error: window.files.error }) },
            { name: "activity", index: 6, ready: () => window.activity.items.rowCount() > 0 && column(window.activity.items, 7).every(s => s === "done"),
              state: () => ({ items: window.activity.items.rowCount(), states: column(window.activity.items, 7) }) },
            { name: "settings", index: 7, ready: () => (window.settings.grants[window.link.currentPhone] || []).length > 0,
              state: () => ({ grants: window.settings.grants[window.link.currentPhone] || [] }), dark: true },
            { name: "screen", index: 5, ready: () => window.mirror.state === "idle",
              state: () => ({ state: window.mirror.state, error: window.mirror.error }) },
            { name: "setup", index: 0, setup: true, ready: () => (window.settings.grants[window.link.currentPhone] || []).length > 0,
              state: () => ({ capabilities: window.settings.sessionCapabilities }) },
            { name: "pairing", index: 7, pair: true, ready: () => window.pairing.state === "waiting" && window.pairing.code.length === 6,
              state: () => ({ state: window.pairing.state, codeLength: window.pairing.code.length }) }
        ]
        function enter(i) {
            const p = pageList[i]
            pageNo = i
            pageTicks = 0
            window.pairOpen = false
            window.page = p.index
            window.setupPhone = p.setup ? window.phoneName : ""
            if (p.pair) {
                window.pairOpen = true
                window.pairing.start()
            }
            if (p.name === "activity" && sendFile !== "") {
                window.link.sendFiles(["file://" + sendFile])
                console.log("SHOT sent")
            }
        }
        function pageStep() {
            const p = pageList[pageNo]
            if (!p.ready() && ++pageTicks < 300)
                return
            console.log("SHOT page " + JSON.stringify(Object.assign({ page: p.name, ready: p.ready() }, p.state())))
            pageNo = -2
            const done = () => {
                if (p.pair) window.pairing.cancel()
                window.setupPhone = ""
                if (pagesDone + 1 < pageList.length) enter(++pagesDone)
                else { window.pairOpen = false; window.page = 0; console.log("SHOT pages done"); step = 3 }
            }
            save(p.dark ? [p.name + "-light", p.name + "-dark"] : [p.name + "-light"], done)
        }
        function save(names, done) {
            Theme.dark = names[0].split("-").includes("dark")
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
            if (ticks > 3000) {
                console.log("SHOT timeout " + state())
                Qt.exit(1)
            } else if (step === 0 && l.serviceRunning) {
                console.log("SHOT listening")
                step = 1
            } else if (step === 1 && l.connected && l.hasPlayer && l.notifications.rowCount() >= 3) {
                console.log("SHOT state " + state())
                l.ring()
                step = 2
                save(["light-connected", "dark-connected"], () => {
                    console.log("SHOT connected done")
                    if (pagesArg > 0) {
                        for (const f of ["messages", "browse", "screen"])
                            window.settings.setGrant(l.currentPhone, f, true)
                        step = 5
                        enter(0)
                    } else
                        step = 3
                })
            } else if (step === 5 && pageNo >= 0) {
                pageStep()
            } else if (step === 3 && !l.serviceRunning) {
                console.log("SHOT state " + state())
                step = 4
                save(["light-disconnected", "dark-disconnected"], () => Qt.quit())
            }
        }
    }

    // E2E hook: `--hold DIR` shows Home then Settings, light then dark, 4 s each, saving DIR/<page>-<theme>.png
    // after 2 s and logging "HOLD page dark" at each start; it quits after the last.
    Timer {
        id: hold
        property int n: -1
        readonly property var steps: [[0, false], [0, true], [7, false], [7, true]]
        running: Qt.application.arguments.indexOf("--hold") > 0 && window.link !== null
        interval: 200
        repeat: true
        onTriggered: {
            const l = window.link
            if (n < 0) {
                if (!listening && l.serviceRunning) { listening = true; console.log("HOLD listening") }
                if (l.connected && l.hasPlayer && l.notifications.rowCount() >= 3) { n = 0; next() }
            } else if (++ticks >= 20) {
                ticks = 0
                ++n < steps.length ? next() : Qt.quit()
            }
        }
        property int ticks: 0
        property bool listening: false
        function next() {
            window.page = steps[n][0]
            Theme.dark = steps[n][1]
            console.log("HOLD " + steps[n][0] + " " + steps[n][1])
            const name = (steps[n][0] === 0 ? "home-" : "settings-") + (steps[n][1] ? "dark" : "light")
            holdSave.name = name
            holdSave.restart()
        }
    }

    Timer {
        id: holdSave
        property string name
        interval: 2000
        onTriggered: body.grabToImage(r => {
            r.saveToFile(Qt.application.arguments[Qt.application.arguments.indexOf("--hold") + 1] + "/gnome-" + name + ".png")
            console.log("HOLD saved " + name)
        })
    }

    // E2E hook: `--page NAME DIR [ACTION ARG]...` opens page NAME once the phone is connected and logs "PAGE state
    // {...}" each second. Once the page is ready it saves DIR/NAME-light.png and NAME-dark.png, then runs each ACTION(ARG)
    // of the page in turn, a second apart, logging "PAGE ACTION result". "await FILE" waits until FILE exists; "shot
    // NAME" saves DIR/NAME-light.png and NAME-dark.png.
    Timer {
        id: pageHook
        readonly property int arg: Qt.application.arguments.indexOf("--page")
        readonly property var args: Qt.application.arguments.slice(arg + 1)
        readonly property var pages: ({
            calls: { index: 2, ready: () => window.calls.ready && window.calls.calls.rowCount() > 0,
                     state: () => ({ ready: window.calls.ready, refused: window.calls.refused,
                                     numbers: shot.column(window.calls.calls, 0), kinds: shot.column(window.calls.calls, 3) }),
                     dial: number => window.calls.dial(number) },
            files: { index: 4, ready: () => window.files.entries.rowCount() > 0,
                     state: () => ({ path: window.files.path, writable: window.files.writable, error: window.files.error,
                                     names: shot.column(window.files.entries, 0) }),
                     cd: path => { window.files.cd(path); return true },
                     upload: file => { window.files.upload(["file://" + file]); return true },
                     remove: path => { window.files.remove(path); return true } },
            photos: { index: 3, ready: () => window.photos.photos.rowCount() > 0,
                      state: () => ({ photos: window.photos.photos.rowCount(), error: window.photos.error }),
                      camera: () => { photosPage.taking = true; window.photos.takePhoto(); return true } }
        })
        property int ticks: 0
        property int step: 0
        property int next: 2
        function exists(file) {
            const x = new XMLHttpRequest()
            x.open("GET", "file://" + file, false)
            x.send()
            return x.responseText !== "" || x.status === 200
        }
        function shots(name, done) {
            Theme.dark = false
            body.grabToImage(r => {
                r.saveToFile(args[1] + "/" + name + "-light.png")
                Theme.dark = true
                body.grabToImage(r => {
                    r.saveToFile(args[1] + "/" + name + "-dark.png")
                    Theme.dark = false
                    done()
                })
            })
        }
        running: arg > 0 && window.link !== null
        interval: 200
        repeat: true
        onTriggered: {
            const p = pages[args[0]]
            if (!window.link.connected)
                return
            window.page = p.index
            if (++ticks % 5 === 0)
                console.log("PAGE state " + JSON.stringify(p.state()))
            if (step === 0 && p.ready()) {
                step = 1
                shots(args[0], () => { console.log("PAGE saved"); step = 2 })
            } else if (step === 2 && next + 1 < args.length && ticks % 5 === 0) {
                const [action, value] = [args[next], args[next + 1]]
                if (action === "await") {
                    if (!exists(value))
                        return
                    next += 2
                    console.log("PAGE await " + value)
                } else if (action === "shot") {
                    step = 1
                    next += 2
                    shots(value, () => { console.log("PAGE shot " + value); step = 2 })
                } else {
                    next += 2
                    console.log("PAGE " + action + " " + p[action](value))
                }
            }
        }
    }

    // E2E hook: `--service-test` logs each stage of: service up and phone connected, service stopped, then
    // startService() and the phone connected again.
    Timer {
        id: serviceTest
        property string stage: "up"
        property bool asked: false
        running: Qt.application.arguments.indexOf("--service-test") > 0 && window.link !== null
        interval: 200
        repeat: true
        onTriggered: {
            const l = window.link
            const now = "serviceRunning " + l.serviceRunning + " connected " + l.connected
            if (stage === "up" && l.serviceRunning && l.connected) {
                console.log("SVC up " + now); stage = "down"
            } else if (stage === "down" && !l.serviceRunning) {
                console.log("SVC down " + now); stage = "back"; l.startService()
            } else if (stage === "back" && l.serviceRunning && l.connected) {
                console.log("SVC back " + now); Qt.quit()
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
    // Joining the phone's hotspot takes this computer off its Wi-Fi, so the service waits for this answer (30 s).
    Popup {
        id: hotspotAsk
        readonly property string phone: ((window.link.phones || []).find(p => p.id === window.link.hotspotAsk) || {}).name || "Your phone"
        anchors.centerIn: parent
        modal: true
        closePolicy: Popup.NoAutoClose
        visible: window.link.hotspotAsk !== ""
        onVisibleChanged: if (visible) { window.show(); window.raise(); window.requestActivate() }
        padding: 24
        width: 440
        background: Rectangle {
            radius: 20
            color: Theme.elev
            border.width: 1
            border.color: Theme.b2
        }
        Overlay.modal: Rectangle { color: Qt.alpha("black", 0.4) }
        contentItem: ColumnLayout {
            spacing: 8
            Label {
                Layout.fillWidth: true
                text: "Use " + hotspotAsk.phone + "\u2019s hotspot?"
                font.pixelSize: Theme.title
                font.weight: Font.DemiBold
                wrapMode: Text.WordWrap
            }
            Label {
                Layout.fillWidth: true
                text: "This is too slow over Bluetooth. This computer leaves its Wi-Fi and has no internet until it is done, then joins your Wi-Fi again."
                wrapMode: Text.WordWrap
                font.pixelSize: Theme.body
                color: Theme.t2
            }
            RowLayout {
                Layout.topMargin: 16
                Layout.alignment: Qt.AlignRight
                spacing: 8
                TonalButton {
                    text: "Not now"
                    onClicked: window.link.answerHotspot(false)
                }
                TonalButton {
                    text: "Use hotspot"
                    onClicked: window.link.answerHotspot(true)
                }
            }
        }
    }
}

import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Dialogs
import QtQuick.Layouts
import org.andromeda.Link

Item {
    id: page
    required property var link
    required property var api
    required property var settings
    property string phoneName
    property var selected: ({})
    readonly property var paths: Object.keys(selected)
    readonly property bool live: link.serviceRunning && link.connected
    readonly property bool granted: (settings.grants[link.currentPhone] || []).indexOf("browse") >= 0
    readonly property bool ready: live && granted
    readonly property bool writable: ready && api.writable
    readonly property string folderName: api.crumbs.length > 0 ? api.crumbs[api.crumbs.length - 1].name : ""
    property string deleting: ""

    function send(urls) {
        if (page.writable)
            page.api.upload(urls)
        else
            page.link.sendFiles(urls)
    }

    function toggle(path) {
        const s = Object.assign({}, selected)
        if (s[path])
            delete s[path]
        else
            s[path] = true
        selected = s
    }

    onVisibleChanged: if (visible && ready && list.count === 0) api.reload()
    Connections {
        target: page.api
        function onPathChanged() { page.selected = {} }
    }

    FileDialog {
        id: picker
        fileMode: FileDialog.OpenFiles
        title: page.writable ? "Copy files into " + page.folderName : "Send files to " + page.phoneName
        onAccepted: page.send(selectedFiles)
    }

    DropArea {
        id: drop
        anchors.fill: parent
        enabled: page.live
        onDropped: event => {
            if (event.hasUrls)
                page.send(event.urls)
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 32
        anchors.topMargin: 24
        anchors.bottomMargin: 24
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            spacing: 4
            Icon { glyph: "phone"; size: 16; color: Theme.t3 }
            Repeater {
                model: page.ready ? page.api.crumbs : [{ name: "Phone", path: "/" }]
                Row {
                    id: crumb
                    required property var modelData
                    required property int index
                    readonly property bool last: index === (page.ready ? page.api.crumbs.length - 1 : 0)
                    spacing: 4
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: crumb.index > 0
                        glyph: "chev"
                        size: 16
                        color: Theme.t3
                    }
                    AbstractButton {
                        anchors.verticalCenter: parent.verticalCenter
                        padding: 4
                        enabled: !crumb.last
                        text: crumb.modelData.name
                        onClicked: page.api.cd(crumb.modelData.path)
                        contentItem: Label {
                            text: parent.text
                            font.pixelSize: Theme.label
                            font.weight: crumb.last ? Font.DemiBold : Font.Normal
                            font.underline: parent.hovered
                            color: crumb.last ? Theme.t1 : Theme.t2
                        }
                    }
                }
            }
            Item { Layout.fillWidth: true }
            TonalButton {
                visible: page.ready
                iconName: "up"
                text: "Send files to the phone"
                onClicked: picker.open()
            }
            TonalButton {
                Layout.leftMargin: 4
                visible: page.ready
                enabled: page.paths.length > 0
                iconName: "down"
                kind: "primary"
                text: "Save to this computer"
                onClicked: {
                    page.api.save(page.paths)
                    page.selected = {}
                }
            }
        }

        DisconnectedCard {
            Layout.fillWidth: true
            Layout.topMargin: 16
            visible: !page.live
            link: page.link
            phoneName: page.phoneName
        }
        OffCard {
            Layout.fillWidth: true
            Layout.maximumWidth: 640
            Layout.topMargin: 16
            visible: page.live && !page.granted
            title: "All files access is off on " + page.phoneName
            text: "Turn it on to browse the phone's storage here. The phone opens its settings, and you allow it there."
            onTurnOn: page.settings.setGrant(page.link.currentPhone, "browse", true)
        }

        Label {
            Layout.fillWidth: true
            Layout.topMargin: 8
            visible: page.ready && page.api.path === "/" && !page.api.loading && list.count > 0 && !page.hasStorage
            text: "All files access is off on " + page.phoneName + ", so only Photos shows and nothing can be copied in. "
                  + "Turn it on in Link on the phone, under Browse files."
            wrapMode: Text.WordWrap
            font.pixelSize: Theme.body
            color: Theme.t2
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: 8
            Layout.preferredHeight: 28
            visible: page.ready && list.count > 0
            spacing: 16
            Item { implicitWidth: 34 }
            Head { Layout.fillWidth: true; text: "Name" }
            Head { Layout.preferredWidth: 96; horizontalAlignment: Text.AlignRight; text: "Size" }
            Head { Layout.preferredWidth: 140; Layout.leftMargin: 16; text: "Modified" }
            Item { implicitWidth: 32 }
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            visible: page.ready && list.count > 0
            color: Theme.b2
        }

        ListView {
            id: list
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: page.ready && page.api.error === ""
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            model: page.api.entries
            ScrollBar.vertical: ScrollBar {}
            delegate: FileRow {
                required property int index
                width: list.width
                first: index === 0
                selected: !!page.selected[path]
                deletable: page.writable && !folder
                onDeleteAsked: page.deleting = path
                onClicked: folder ? page.api.cd(path) : page.toggle(path)
                onOpened: if (!folder) page.api.openFile(path)
            }
            EmptyState {
                anchors.centerIn: parent
                visible: list.count === 0 && !page.api.loading
                iconName: "folder"
                title: "This folder is empty"
            }
            BusyIndicator {
                anchors.centerIn: parent
                visible: page.api.loading && list.count === 0
                running: visible
            }
        }
        EmptyState {
            Layout.alignment: Qt.AlignCenter
            Layout.fillHeight: true
            visible: page.ready && page.api.error !== ""
            iconName: "folder"
            title: "This folder did not open"
            text: page.phoneName + " said: " + page.api.error
            action: "Try again"
            primary: false
            onTriggered: page.api.reload()
        }
        Item {
            Layout.fillHeight: true
            visible: !page.ready
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.topMargin: 16
            implicitHeight: 72
            visible: page.ready
            radius: 16
            color: drop.containsDrag ? Qt.alpha(Theme.acc, 0.10) : "transparent"
            border.width: 1.5
            border.color: drop.containsDrag ? Theme.acc : Theme.b1
            Row {
                anchors.centerIn: parent
                spacing: 8
                Icon { anchors.verticalCenter: parent.verticalCenter; glyph: "up"; size: 16 }
                Label {
                    anchors.verticalCenter: parent.verticalCenter
                    textFormat: Text.StyledText
                    text: page.writable ? "<b>Drop files here</b> to copy them into " + page.folderName
                                        : "<b>Drop files here</b> to send them to " + page.phoneName
                    font.pixelSize: Theme.body
                    color: Theme.t2
                }
            }
        }
    }

    readonly property bool hasStorage: {
        const m = page.api.entries
        for (let i = 0; i < list.count; i++)
            if (m.data(m.index(i, 0), 256) !== "Photos")
                return true
        return false
    }

    Popup {
        id: confirm
        anchors.centerIn: parent
        modal: true
        visible: page.deleting !== ""
        onClosed: page.deleting = ""
        padding: 24
        width: 420
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
                text: "Delete \u201c" + page.deleting.split("/").pop() + "\u201d?"
                font.pixelSize: Theme.title
                font.weight: Font.DemiBold
                elide: Text.ElideMiddle
            }
            Label {
                Layout.fillWidth: true
                text: "It is deleted from " + page.phoneName + ". This cannot be undone."
                wrapMode: Text.WordWrap
                font.pixelSize: Theme.body
                color: Theme.t2
            }
            RowLayout {
                Layout.topMargin: 16
                Layout.alignment: Qt.AlignRight
                spacing: 8
                TonalButton {
                    text: "Cancel"
                    onClicked: confirm.close()
                }
                TonalButton {
                    kind: "warn"
                    text: "Delete"
                    onClicked: {
                        page.api.remove(page.deleting)
                        confirm.close()
                    }
                }
            }
        }
    }

    component Head: Label {
        font.pixelSize: Theme.meta
        color: Theme.t3
    }
}

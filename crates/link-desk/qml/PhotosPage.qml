import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

Item {
    id: page
    required property var link
    required property var api
    required property var settings
    property string phoneName
    property var selected: ({})
    property bool taking: false
    readonly property var paths: Object.keys(selected)
    readonly property bool live: link.serviceRunning && link.connected
    readonly property bool granted: (settings.grants[link.currentPhone] || []).indexOf("browse") >= 0
    readonly property bool ready: live && granted

    function toggle(path) {
        const s = Object.assign({}, selected)
        if (s[path])
            delete s[path]
        else
            s[path] = true
        selected = s
    }

    onVisibleChanged: if (visible && ready && grid.count === 0) api.reload()
    onReadyChanged: if (!ready) { selected = {}; taking = false }
    Connections {
        target: page.api
        function onSaved() { page.taking = false }
    }
    Connections {
        target: page.link
        function onFailed() { page.taking = false }
    }
    Connections {
        target: page.link
        function onCurrentPhoneChanged() { page.selected = {} }
    }
    Binding {
        target: page.api
        property: "columns"
        value: grid.columns
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 32
        anchors.rightMargin: 32
        anchors.topMargin: 24
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            spacing: 12
            visible: page.paths.length === 0
            Label {
                Layout.fillWidth: true
                text: "Photos"
                font.pixelSize: Theme.title
                font.weight: Font.DemiBold
                Accessible.role: Accessible.Heading
            }
            Segmented {
                visible: page.ready
                options: [{ label: "All", value: "all" }, { label: "Photos", value: "photos" }, { label: "Videos", value: "videos" }]
                current: page.api.filter
                onPicked: value => page.api.filter = value
            }
            TonalButton {
                objectName: "takePhoto"
                visible: page.ready
                enabled: !page.taking
                iconName: "camera"
                text: page.taking ? "Take the photo on " + page.phoneName : "Take a photo"
                onClicked: {
                    page.taking = true
                    page.api.takePhoto()
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            spacing: 12
            visible: page.paths.length > 0
            IconButton {
                iconName: "x"
                text: "Clear selection"
                iconColor: Theme.t2
                background: Rectangle { radius: 20; color: "transparent"; border.width: 1.5; border.color: Theme.b1 }
                onClicked: page.selected = {}
            }
            Label {
                Layout.fillWidth: true
                text: page.paths.length + " selected"
                font.pixelSize: Theme.title
                font.weight: Font.DemiBold
            }
            TonalButton {
                iconName: "copy"
                text: "Copy"
                enabled: page.live
                onClicked: page.api.copy(page.paths)
            }
            TonalButton {
                iconName: "down"
                text: "Save to Downloads"
                kind: "primary"
                enabled: page.live
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
            Layout.topMargin: 16
            visible: page.live && !page.granted
            title: "Photo access is off on " + page.phoneName
            text: "Turn it on to see the phone's photos here. The phone opens its settings, and you allow it there."
            onTurnOn: page.settings.setGrant(page.link.currentPhone, "browse", true)
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            PhotoGrid {
                id: grid
                anchors.fill: parent
                anchors.leftMargin: -8
                anchors.rightMargin: -8
                visible: page.ready && page.api.error === ""
                model: page.api.lines
                selected: page.selected
                onToggled: path => page.toggle(path)
                onWanted: path => page.api.want(path)
            }
            Grid {
                id: skeleton
                y: 24
                width: parent.width
                visible: !page.ready || (page.api.loading && grid.count === 0)
                columns: 6
                spacing: 8
                Repeater {
                    model: 12
                    Rectangle {
                        width: (skeleton.width - 40) / 6
                        height: width
                        radius: 10
                        color: Theme.tert
                        opacity: 0.6
                    }
                }
            }
            EmptyState {
                anchors.centerIn: parent
                anchors.verticalCenterOffset: -40
                visible: page.ready && !page.api.loading && (page.api.error !== "" || grid.count === 0)
                iconName: "photo"
                title: page.api.error !== "" ? "Photos did not load" : "No photos yet"
                text: page.api.error !== "" ? Theme.browseError(page.phoneName, page.api.error)
                                            : "Photos you take with " + page.phoneName + " appear here."
                action: page.api.error !== "" ? "Try again" : "Refresh"
                primary: false
                onTriggered: page.api.reload()
            }
        }
    }
}

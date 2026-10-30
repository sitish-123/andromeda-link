import QtQuick
import QtQuick.Layouts
import org.andromeda.Link

Item {
    id: page
    required property var link
    required property var api
    required property var settings
    property string phoneName
    readonly property bool granted: (settings.grants[link.currentPhone] || []).indexOf("messages") >= 0

    EmptyState {
        anchors.centerIn: parent
        visible: !page.granted
        tile: 72
        iconName: "msg"
        title: "Texts are off on " + page.phoneName
        text: "Turn on to read and send texts from this computer. " + page.phoneName + " asks you to allow it."
        action: "Turn on"
        onTriggered: page.settings.setGrant(page.link.currentPhone, "messages", true)
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0
        visible: page.granted

        ThreadList {
            Layout.preferredWidth: 304
            Layout.fillWidth: false
            Layout.fillHeight: true
            api: page.api
            live: card.live
            onCompose: {
                page.api.currentThread = ""
                card.composing = true
            }
        }
        ThreadCard {
            id: card
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.topMargin: 16
            Layout.rightMargin: 16
            Layout.bottomMargin: 16
            api: page.api
            link: page.link
        }
    }

    Connections {
        target: page.api
        function onCurrentThreadChanged() {
            if (page.api.currentThread !== "")
                card.composing = false
        }
    }
}

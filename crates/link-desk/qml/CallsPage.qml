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
    readonly property bool live: link.serviceRunning && link.connected
    readonly property bool granted: (settings.grants[link.currentPhone] || []).indexOf("calls") >= 0
    property alias number: field.text
    signal dialed(string number)

    function call(number) {
        if (page.api.dial(number))
            page.dialed(number)
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.leftMargin: 32
        anchors.rightMargin: 32
        anchors.topMargin: 24
        anchors.bottomMargin: 24
        spacing: 0

        Label {
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            verticalAlignment: Text.AlignVCenter
            text: "Calls"
            font.pixelSize: Theme.title
            font.weight: Font.DemiBold
            Accessible.role: Accessible.Heading
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
            title: "Calls are off for " + page.phoneName
            text: "Turn on to see its calls here and call from this computer. The audio stays on the phone."
            onTurnOn: page.settings.setGrant(page.link.currentPhone, "calls", true)
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.topMargin: 16
            spacing: 24
            visible: page.live && page.granted

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                TextField {
                    id: search
                    Layout.fillWidth: true
                    Layout.bottomMargin: 8
                    implicitHeight: 44
                    leftPadding: 40
                    enabled: page.api.refused === ""
                    placeholderText: "Search contacts and calls"
                    placeholderTextColor: Theme.t3
                    color: Theme.t1
                    font.pixelSize: Theme.body
                    onTextChanged: page.api.query = text
                    background: Rectangle {
                        radius: 14
                        color: Theme.input
                        border.width: search.activeFocus ? 1.5 : 0
                        border.color: Theme.acc
                        Icon {
                            x: 12
                            anchors.verticalCenter: parent.verticalCenter
                            glyph: "search"
                            color: Theme.t3
                        }
                    }
                }

                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true

                    ListView {
                        id: view
                        anchors.fill: parent
                        anchors.leftMargin: -8
                        anchors.rightMargin: -8
                        clip: true
                        boundsBehavior: Flickable.StopAtBounds
                        model: page.api.calls
                        visible: page.api.refused === ""
                        ScrollBar.vertical: ScrollBar {}

                        section.property: "section"
                        section.delegate: Label {
                            required property string section
                            width: ListView.view.width
                            height: 44
                            leftPadding: 20
                            topPadding: 16
                            text: section
                            font.pixelSize: Theme.label
                            font.weight: Font.DemiBold
                        }

                        delegate: AbstractButton {
                            id: row
                            required property string number
                            required property string name
                            required property string initial
                            required property string kind
                            required property string detail
                            readonly property bool missed: kind === "missed"

                            x: 8
                            width: ListView.view.width - 16
                            height: 64
                            leftPadding: 12
                            rightPadding: 8
                            text: name
                            Accessible.description: detail
                            onClicked: page.number = number

                            background: Rectangle {
                                radius: 16
                                color: row.hovered ? Theme.hover : "transparent"
                            }
                            contentItem: RowLayout {
                                spacing: 12
                                Avatar {
                                    implicitWidth: 40
                                    implicitHeight: 40
                                    initial: row.initial
                                    seed: row.name
                                }
                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 0
                                    Label {
                                        Layout.fillWidth: true
                                        text: row.name
                                        font.pixelSize: Theme.label
                                        font.weight: Font.Medium
                                        color: row.missed ? Theme.warn : Theme.t1
                                        elide: Text.ElideRight
                                    }
                                    RowLayout {
                                        Layout.fillWidth: true
                                        spacing: 4
                                        Icon {
                                            visible: row.kind !== "contact"
                                            glyph: row.kind === "outgoing" ? "up" : "down"
                                            rotation: 45
                                            size: 14
                                            color: row.missed ? Theme.warn : Theme.t3
                                        }
                                        Label {
                                            Layout.fillWidth: true
                                            text: row.detail
                                            font.pixelSize: Theme.small
                                            font.features: Theme.tabular
                                            color: row.missed ? Theme.warn : Theme.t2
                                            elide: Text.ElideRight
                                        }
                                    }
                                }
                                IconButton {
                                    iconName: "call"
                                    text: "Call " + row.name
                                    fill: Theme.wash
                                    onClicked: page.call(row.number)
                                }
                            }
                        }
                    }

                    EmptyState {
                        anchors.centerIn: parent
                        anchors.verticalCenterOffset: -40
                        visible: page.api.refused !== "" || (page.api.ready && view.count === 0)
                        iconName: "call"
                        title: page.api.refused === "not-allowed" ? "Calls are off on " + page.phoneName
                             : page.api.refused !== "" ? page.phoneName + " does not share its call log"
                             : search.text !== "" ? "No match" : "No calls yet"
                        text: page.api.refused === "not-allowed" ? "Turn on Calls for this computer in Link on " + page.phoneName + "."
                            : page.api.refused !== "" ? "Allow Call logs for Link in the phone's settings."
                            : search.text !== "" ? "No contact or call holds “" + search.text + "”."
                            : "Calls on " + page.phoneName + " appear here."
                    }
                }
            }

            Rectangle {
                Layout.preferredWidth: 344
                Layout.alignment: Qt.AlignTop
                implicitHeight: pad.implicitHeight + 48
                radius: 20
                color: Theme.elev
                border.width: 1
                border.color: Theme.b2

                ColumnLayout {
                    id: pad
                    x: 24
                    y: 24
                    width: parent.width - 48
                    spacing: 16

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 4
                        TextField {
                            id: field
                            objectName: "dialNumber"
                            Layout.fillWidth: true
                            implicitHeight: 56
                            horizontalAlignment: Text.AlignHCenter
                            placeholderText: "Number"
                            placeholderTextColor: Theme.t3
                            color: Theme.t1
                            font.pixelSize: 28
                            font.weight: Font.DemiBold
                            font.features: Theme.tabular
                            validator: RegularExpressionValidator { regularExpression: /[0-9+*#() -]{0,64}/ }
                            background: Item {}
                            onAccepted: page.call(text)
                        }
                        IconButton {
                            visible: field.text !== ""
                            iconName: "x"
                            text: "Delete a digit"
                            iconColor: Theme.t2
                            onClicked: field.text = field.text.slice(0, -1)
                        }
                    }

                    GridLayout {
                        Layout.alignment: Qt.AlignHCenter
                        columns: 3
                        rowSpacing: 12
                        columnSpacing: 20
                        Repeater {
                            model: [["1", ""], ["2", "ABC"], ["3", "DEF"], ["4", "GHI"], ["5", "JKL"], ["6", "MNO"],
                                    ["7", "PQRS"], ["8", "TUV"], ["9", "WXYZ"], ["*", ""], ["0", "+"], ["#", ""]]
                            AbstractButton {
                                id: key
                                required property var modelData
                                implicitWidth: 72
                                implicitHeight: 56
                                text: modelData[0]
                                Accessible.name: modelData[0]
                                onClicked: field.text += modelData[0]
                                onPressAndHold: if (modelData[0] === "0") field.text += "+"
                                background: Rectangle {
                                    radius: 18
                                    color: Qt.alpha(Theme.t1, key.down ? 0.13 : key.hovered ? 0.1 : 0.07)
                                }
                                contentItem: Column {
                                    topPadding: key.modelData[1] ? 6 : 14
                                    Label {
                                        width: key.width
                                        horizontalAlignment: Text.AlignHCenter
                                        text: key.modelData[0]
                                        font.pixelSize: 22
                                        font.weight: Font.DemiBold
                                    }
                                    Label {
                                        width: key.width
                                        visible: key.modelData[1] !== ""
                                        horizontalAlignment: Text.AlignHCenter
                                        text: key.modelData[1]
                                        font.pixelSize: Theme.meta
                                        font.letterSpacing: 1
                                        color: Theme.t3
                                    }
                                }
                            }
                        }
                    }

                    AbstractButton {
                        id: callButton
                        Layout.alignment: Qt.AlignHCenter
                        Layout.topMargin: 4
                        implicitWidth: 64
                        implicitHeight: 64
                        enabled: field.text.replace(/[^0-9+*#]/g, "") !== ""
                        text: "Call"
                        Accessible.name: "Call " + field.text
                        onClicked: page.call(field.text)
                        background: Rectangle {
                            radius: width / 2
                            color: callButton.enabled ? Theme.acc : Theme.wash
                            opacity: callButton.down ? 0.85 : 1
                        }
                        contentItem: Item {
                            Icon {
                                anchors.centerIn: parent
                                glyph: "call"
                                size: 24
                                color: callButton.enabled ? Theme.accText : Theme.dis
                            }
                        }
                    }
                    Label {
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignHCenter
                        text: page.phoneName + " places the call. Talk on the phone."
                        font.pixelSize: Theme.meta
                        color: Theme.t3
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }
    }
}

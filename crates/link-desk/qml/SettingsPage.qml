import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import org.andromeda.Link

// Tabs: "This computer", then one per paired phone.
Flickable {
    id: page
    required property var link
    required property var api
    property string tab: "computer"
    readonly property var phone: (link.phones || []).find(p => p.id === tab) ?? null
    signal pair()

    contentHeight: col.implicitHeight + 32
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: ScrollBar {}

    ColumnLayout {
        id: col
        x: 32
        y: 24
        width: page.width - 64
        spacing: 0

        RowLayout {
            Layout.fillWidth: true
            Label {
                Layout.fillWidth: true
                text: "Settings"
                font.pixelSize: 28
                font.weight: Font.Bold
                lineHeight: 36
                lineHeightMode: Text.FixedHeight
                Accessible.role: Accessible.Heading
            }
            Segmented {
                quiet: true
                options: [{ label: "This computer", value: "computer" }]
                         .concat((page.link.phones || []).map(p => ({ label: p.name, value: p.id })))
                current: page.phone ? page.tab : "computer"
                onPicked: value => page.tab = value
            }
        }

        ComputerSettings {
            Layout.fillWidth: true
            visible: !page.phone
            link: page.link
            api: page.api
            onPair: page.pair()
        }
        PhoneSettings {
            Layout.fillWidth: true
            visible: page.phone !== null
            link: page.link
            api: page.api
            phone: page.phone ?? { id: "", name: "", connected: false, lastSeen: 0 }
        }
    }
}

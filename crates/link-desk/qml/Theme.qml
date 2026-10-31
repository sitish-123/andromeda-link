pragma Singleton
import QtQuick

QtObject {
    property bool dark: Qt.styleHints.colorScheme === Qt.ColorScheme.Dark

    readonly property color bg: dark ? "#0D1012" : "#F3F1EC"
    readonly property color tert: dark ? "#161B1E" : "#EAE7E0"
    readonly property color elev: dark ? "#161A1D" : "#FAF9F6"
    readonly property color input: dark ? "#161A1D" : "#E8E5DE"
    readonly property color t1: dark ? "#EDEFEF" : "#15191B"
    readonly property color t2: dark ? "#A3AAAD" : "#555B5E"
    readonly property color t3: dark ? "#6E777B" : "#7C8184"
    readonly property color dis: dark ? "#4A5154" : "#B4B3AE"
    readonly property color acc: dark ? "#00D7FF" : "#00789C"
    readonly property color accText: dark ? "#0D1012" : "#F3F1EC"
    readonly property color b1: dark ? "#262D31" : "#D5D1C8"
    readonly property color b2: dark ? "#1E2427" : "#E2DED6"
    readonly property color ok: dark ? "#4CC38A" : "#2E7D4F"
    readonly property color warn: dark ? "#FFB13B" : "#C77700"
    readonly property color wash: Qt.alpha(t1, 0.07)
    readonly property color hover: Qt.alpha(t1, 0.04)

    readonly property string family: "Source Sans 3"
    readonly property var tabular: ({ "tnum": 1 })

    readonly property int gutter: 24
    readonly property int sidebarWidth: 240
    readonly property int homeWidth: 528
    readonly property int notesMinWidth: 320
    readonly property int icon: 18

    readonly property int meta: 12
    readonly property int small: 13
    readonly property int body: 14
    readonly property int label: 15
    readonly property int name: 17
    readonly property int title: 20

    // A browse error as the user can act on it: the service's refusal codes, else its own words.
    function browseError(phone, error) {
        switch (error) {
        case "not-allowed": return "Browse files is off for this computer. Turn it on in Link on " + phone + "."
        case "denied": return phone + " does not let Link read this. Allow photos and files for Link in the phone's settings."
        case "busy": return phone + " is busy with other requests. Try again in a moment."
        case "not-found": return "This is no longer on " + phone + "."
        case "the phone did not answer in time":
            return phone + " did not answer. Open Link on the phone; if it then works, the phone stops Link in the background."
        default: return phone + " said: " + error
        }
    }

    function since(unixS) {
        if (unixS <= 0)
            return ""
        const s = Math.max(0, Date.now() / 1000 - unixS)
        if (s < 60)
            return "just now"
        if (s < 3600)
            return Math.floor(s / 60) + " min ago"
        if (s < 86400)
            return Math.floor(s / 3600) + " h ago"
        if (s < 172800)
            return "yesterday"
        return Math.floor(s / 86400) + " days ago"
    }

    function clock(ms) {
        const s = Math.max(0, Math.floor(ms / 1000))
        return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0")
    }
}

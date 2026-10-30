import QtQuick
import org.andromeda.Link

QtObject {
    property bool loading: false
    property string filter: "all"
    property string error: ""
    property bool empty: false
    property ListModel photos: ListModel {}

    signal saved(int count, string folder)

    function reload() {
        photos.clear()
        if (empty)
            return
        const days = [["Today", 6], ["Yesterday", 12], ["Wed 28 Oct", 6]]
        let n = 0
        for (const [day, count] of days)
            for (let i = 0; i < count; i++, n++)
                photos.append({ path: "/DCIM/Camera/IMG_" + (2041 - n) + ".jpg", name: "IMG_" + (2041 - n) + ".jpg",
                                thumbUrl: "file:///tmp/link-desk-pages/thumbs/" + (n % 18) + ".png",
                                video: n % 7 === 3, durationText: n % 7 === 3 ? "0:42" : "", day: day })
    }
    function copy(paths) { saved(paths.length, "clipboard") }
    function save(paths) { saved(paths.length, "Downloads") }

    Component.onCompleted: reload()
}

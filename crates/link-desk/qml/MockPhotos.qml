import QtQuick
import org.andromeda.Link

QtObject {
    property bool loading: false
    property string filter: "all"
    property string error: ""
    property bool empty: false
    property ListModel photos: ListModel {}
    property int columns: 6
    property var lines: []

    signal saved(int count, string folder)
    signal viewed(string path, string url)

    function reload() {
        photos.clear()
        lines = []
        if (empty)
            return
        const days = [["Today", 6], ["Yesterday", 12], ["Wed 28 Oct", 6]]
        let n = 0
        for (const [day, count] of days)
            for (let i = 0; i < count; i++, n++)
                photos.append({ path: "/DCIM/Camera/IMG_" + (2041 - n) + ".jpg", name: "IMG_" + (2041 - n) + ".jpg",
                                thumbUrl: "file:///tmp/link-desk-pages/thumbs/" + (n % 18) + ".png",
                                video: n % 7 === 3, durationText: n % 7 === 3 ? "0:42" : "", day: day,
                                album: "Camera", taken: day + " 14:02", size: "3.1 MB" })
        regroup()
    }
    function regroup() {
        const out = []
        let day = ""
        let row = []
        const flush = () => {
            if (row.length)
                out.push({ day: "", cells: row, thumbs: row.map(c => c.thumbUrl) })
            row = []
        }
        for (let i = 0; i < photos.count; i++) {
            const p = photos.get(i)
            if (p.day !== day) {
                flush()
                day = p.day
                out.push({ day: day, cells: [], thumbs: [] })
            }
            row.push({ path: p.path, name: p.name, thumbUrl: p.thumbUrl, video: p.video, durationText: p.durationText })
            if (row.length === columns)
                flush()
        }
        flush()
        lines = out
    }
    function want(path) {}
    function view(path) {
        for (let i = 0; i < photos.count; i++)
            if (photos.get(i).path === path)
                return viewed(path, photos.get(i).thumbUrl)
    }
    function rowOf(path) {
        for (let i = 0; i < photos.count; i++)
            if (photos.get(i).path === path)
                return i
        return -1
    }
    function copy(paths) { saved(paths.length, "clipboard") }
    function save(paths) { saved(paths.length, "Downloads") }

    onColumnsChanged: regroup()
    Component.onCompleted: reload()
}

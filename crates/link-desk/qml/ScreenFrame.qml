import QtQuick
import QtQuick.Shapes

// The phone's screen edge over the mirrored picture: its rounded corners and camera cutouts, in a thin bezel.
Item {
    id: frame
    // Radii in 1/10000 of the width: top-left, top-right, bottom-right, bottom-left; all 0 when the phone did not say.
    property var corners: []
    // Cutouts as x, y, width, height after one another, in 1/10000 of the picture.
    property var cutouts: []
    property real bezel: 6
    property color color: "#0B0C0D"
    readonly property bool known: corners.length === 4 && Array.from(corners).some(c => c > 0)
    readonly property var radii: known ? Array.from(corners).map(c => c / 10000 * width)
                                       : [1, 1, 1, 1].map(() => Math.min(width, height) * 0.08)

    // An SVG path of a rectangle with radii [top-left, top-right, bottom-right, bottom-left].
    function rounded(x, y, w, h, r) {
        return "M" + (x + r[0]) + "," + y
            + " H" + (x + w - r[1]) + " A" + r[1] + "," + r[1] + " 0 0 1 " + (x + w) + "," + (y + r[1])
            + " V" + (y + h - r[2]) + " A" + r[2] + "," + r[2] + " 0 0 1 " + (x + w - r[2]) + "," + (y + h)
            + " H" + (x + r[3]) + " A" + r[3] + "," + r[3] + " 0 0 1 " + x + "," + (y + h - r[3])
            + " V" + (y + r[0]) + " A" + r[0] + "," + r[0] + " 0 0 1 " + (x + r[0]) + "," + y + " Z"
    }

    Shape {
        anchors.fill: parent
        anchors.margins: -frame.bezel
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            fillColor: frame.color
            strokeWidth: -1
            fillRule: ShapePath.OddEvenFill
            PathSvg {
                path: frame.rounded(0, 0, frame.width + 2 * frame.bezel, frame.height + 2 * frame.bezel,
                                    frame.radii.map(r => r + frame.bezel))
                      + " " + frame.rounded(frame.bezel, frame.bezel, frame.width, frame.height, frame.radii)
            }
        }
    }
    Repeater {
        model: Math.floor(frame.cutouts.length / 4)
        Rectangle {
            required property int index
            x: frame.cutouts[index * 4] / 10000 * frame.width
            y: frame.cutouts[index * 4 + 1] / 10000 * frame.height
            width: frame.cutouts[index * 4 + 2] / 10000 * frame.width
            height: frame.cutouts[index * 4 + 3] / 10000 * frame.height
            radius: Math.min(width, height) / 2
            color: frame.color
        }
    }
}

import QtQuick
import QtQuick.Shapes
import QtQuick.Window
import org.andromeda.Link

Item {
    id: art
    property url source
    property real radius: 14
    property bool gradient: true
    // Device pixels in steps of 160, so a resize within a step keeps the loaded image.
    readonly property int pixels: Math.max(1, Math.ceil(Math.max(width, height) * Screen.devicePixelRatio / 160)) * 160

    Shape {
        anchors.fill: parent
        visible: art.gradient && !photo.visible
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeWidth: -1
            fillGradient: LinearGradient {
                x1: 0; y1: 0; x2: art.width; y2: art.height
                GradientStop { position: 0; color: "#3a4a5a" }
                GradientStop { position: 0.6; color: "#8a6a4a" }
                GradientStop { position: 1; color: "#d8b070" }
            }
            PathRectangle { width: art.width; height: art.height; radius: art.radius }
        }
    }
    Rectangle {
        anchors.fill: parent
        visible: !art.gradient && !photo.visible
        radius: art.radius
        color: Theme.tert
    }
    // The shape fills its rounded rectangle with the image's texture, so no cell needs an offscreen layer.
    Shape {
        id: photo
        anchors.fill: parent
        visible: cover.status === Image.Ready
        preferredRendererType: Shape.CurveRenderer
        ShapePath {
            strokeWidth: -1
            fillItem: cover
            // The texture maps at its own pixel size from the origin; scale and centre it as PreserveAspectCrop.
            fillTransform: PlanarTransform.fromAffineMatrix(cover.fit, 0, 0, cover.fit,
                                                            (art.width - cover.implicitWidth * cover.fit) / 2,
                                                            (art.height - cover.implicitHeight * cover.fit) / 2)
            PathRectangle { width: art.width; height: art.height; radius: art.radius }
        }
    }
    Image {
        id: cover
        readonly property real fit: Math.max(art.width / Math.max(1, implicitWidth),
                                             art.height / Math.max(1, implicitHeight))
        visible: false
        source: art.source
        asynchronous: true
        // Crop mode loads the image to cover the square, so neither side is upscaled.
        fillMode: Image.PreserveAspectCrop
        sourceSize: Qt.size(art.pixels, art.pixels)
    }
}

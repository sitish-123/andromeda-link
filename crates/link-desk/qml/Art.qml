import QtQuick
import QtQuick.Effects
import QtQuick.Shapes
import org.andromeda.Link

Item {
    id: art
    property url source
    property real radius: 14
    property bool gradient: true

    Shape {
        anchors.fill: parent
        visible: art.gradient && !cover.visible
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
        visible: !art.gradient && !cover.visible
        radius: art.radius
        color: Theme.tert
    }
    Image {
        id: cover
        anchors.fill: parent
        source: art.source
        visible: status === Image.Ready
        asynchronous: true
        fillMode: Image.PreserveAspectCrop
        // A size of 0 or less before layout would fail the load.
        sourceSize: Qt.size(Math.max(64, art.width * 2), Math.max(64, art.height * 2))
        layer.enabled: true
        layer.effect: MultiEffect {
            maskEnabled: true
            maskSource: mask
        }
    }
    Rectangle {
        id: mask
        anchors.fill: parent
        radius: art.radius
        visible: false
        layer.enabled: true
    }
}

import QtQuick
import org.andromeda.Link

Main {
    property alias mock: mock
    link: MockLink { id: mock }
    messages: MockMessages {}
    activity: MockActivity {}
    photos: MockPhotos {}
    files: MockFiles {}
    mirror: MockScreen {}
    settings: MockSettings {}
    pairing: MockPairing {}
}

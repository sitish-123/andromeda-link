import QtQuick
import org.andromeda.Link

Main {
    property alias mock: mock
    link: MockLink { id: mock }
}

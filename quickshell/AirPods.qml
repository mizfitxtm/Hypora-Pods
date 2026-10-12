import Quickshell
import QtQuick

// Bar indicator for AirPods. Hidden until a pair is connected; click for the panel.
Rectangle {
    id: root
    required property var window

    visible: svc.connected
    implicitWidth: label.implicitWidth + 16
    implicitHeight: 24
    radius: height / 2
    color: drop.open ? Theme.surface : (area.containsMouse ? Qt.rgba(1, 1, 1, 0.06) : "transparent")

    AirPodsService { id: svc }

    Text {
        id: label
        anchors.centerIn: parent
        text: svc.lowest >= 0 ? "Pods " + svc.lowest + "%" : "Pods"
        font.family: Theme.font
        font.pixelSize: Theme.fontSize
        color: svc.lowest >= 0 && svc.lowest < 15 ? Theme.error : Theme.fg
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: drop.open = true
    }

    Dropdown {
        id: drop
        screen: root.window.screen
        barHeight: root.window.height
        anchorItem: root

        AirPodsPanel { svc: svc }
    }
}

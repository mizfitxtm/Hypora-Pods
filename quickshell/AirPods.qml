import Quickshell
import QtQuick

// Bar indicator for AirPods. Hidden until a pair is connected; click for the panel.
Rectangle {
    id: root
    required property var window

    visible: svc.connected
    implicitWidth: glyph.width + 6 + label.implicitWidth + 16
    implicitHeight: 24
    radius: height / 2
    color: drop.open ? Theme.surface : (area.containsMouse ? Qt.rgba(1, 1, 1, 0.06) : "transparent")

    AirPodsService { id: svc }

    readonly property color tint: svc.lowest >= 0 && svc.lowest < 15 ? Theme.error : Theme.fg

    AirPodsIcon {
        id: glyph
        anchors { left: parent.left; leftMargin: 8; verticalCenter: parent.verticalCenter }
        iconSize: 16
        variant: svc.iconVariant
        color: root.tint
    }

    Text {
        id: label
        anchors { left: glyph.right; leftMargin: 6; verticalCenter: parent.verticalCenter }
        text: svc.lowest >= 0 ? svc.lowest + "%" : ""
        visible: text !== ""
        font.family: Theme.font
        font.pixelSize: Theme.fontSize
        color: root.tint
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

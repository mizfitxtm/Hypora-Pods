import QtQuick

// Dropdown content: batteries, listening modes and toggles. Rows appear only when the
// daemon says the device has them.
Rectangle {
    id: root
    required property var svc

    implicitWidth: 280
    implicitHeight: col.implicitHeight + 24
    radius: 10
    color: Theme.bg
    border.color: Theme.surface

    component Row2: Item {
        property string title
        property string value
        property color valueColor: Theme.fg
        width: parent.width
        height: 22
        Text { text: parent.title; color: Theme.dim; font.family: Theme.font; font.pixelSize: Theme.fontSize
               anchors.verticalCenter: parent.verticalCenter }
        Text { text: parent.value; color: parent.valueColor; font.family: Theme.font; font.pixelSize: Theme.fontSize
               anchors { right: parent.right; verticalCenter: parent.verticalCenter } }
    }

    component Choice: Rectangle {
        id: choice
        property string text
        property bool selected: false
        signal clicked
        width: parent.width
        height: 28
        radius: 6
        color: selected ? Theme.accent : (m.containsMouse ? Theme.surface : "transparent")
        Text { anchors { left: parent.left; leftMargin: 8; verticalCenter: parent.verticalCenter }
               text: choice.text; font.family: Theme.font; font.pixelSize: Theme.fontSize
               color: choice.selected ? Theme.bg : Theme.fg }
        MouseArea { id: m; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
                    onClicked: choice.clicked() }
    }

    function pct(l) { return l >= 0 ? l + "%" : "—" }
    function earText(b) { return ["Pause when one is out", "Pause when both are out", "Never pause"][b] ?? "" }

    Column {
        id: col
        x: 12; y: 12
        width: parent.width - 24
        spacing: 4

        Row {
            spacing: 10
            AirPodsIcon {
                anchors.verticalCenter: parent.verticalCenter
                iconSize: 28
                variant: svc.iconVariant
                color: Theme.fg
            }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: svc.s.model_name || svc.s.device_name || "AirPods"
                textFormat: Text.PlainText   // names come from the device; never read them as markup
                color: Theme.fg; font.family: Theme.font; font.pixelSize: Theme.fontSize + 2; font.bold: true
            }
        }

        Row2 { visible: svc.isHeadset; title: "Battery"; value: pct(svc.headsetLevel) }
        Row2 { visible: !svc.isHeadset; title: "Left" + (svc.s.left?.in_ear ? " (in ear)" : ""); value: pct(svc.leftLevel) }
        Row2 { visible: !svc.isHeadset; title: "Right" + (svc.s.right?.in_ear ? " (in ear)" : ""); value: pct(svc.rightLevel) }
        Row2 { visible: !svc.isHeadset && svc.caseLevel >= 0; title: "Case"; value: pct(svc.caseLevel) }

        Item { width: 1; height: 6; visible: svc.modes.length > 0 }
        Repeater {
            model: svc.modes
            Choice {
                required property var modelData
                text: modelData[1]
                selected: svc.s.noise_mode === modelData[0]
                onClicked: svc.send(modelData[2])
            }
        }

        Item { width: 1; height: 6 }
        Choice {
            visible: svc.s.supports_conversational_awareness === true
            text: "Conversation Awareness: " + (svc.s.conversational_awareness ? "on" : "off")
            onClicked: svc.send(svc.s.conversational_awareness ? "ca:off" : "ca:on")
        }
        Choice {
            visible: svc.s.supports_one_bud_anc === true
            text: "One-Bud ANC: " + (svc.s.one_bud_anc_mode ? "on" : "off")
            onClicked: svc.send(svc.s.one_bud_anc_mode ? "onebud:off" : "onebud:on")
        }
        Choice {
            text: earText(svc.s.ear_detection_behavior ?? 0)
            onClicked: svc.send(["ear:both", "ear:off", "ear:one"][svc.s.ear_detection_behavior ?? 0])
        }
        Row2 {
            visible: svc.s.noise_mode === 3
            title: "Adaptive level"; value: (svc.s.adaptive_noise_level ?? 0) + "%"
        }
        Text {
            visible: svc.error !== ""
            width: parent.width; wrapMode: Text.Wrap
            text: svc.error; textFormat: Text.PlainText; color: Theme.error; font.family: Theme.font; font.pixelSize: Theme.fontSize - 2
        }
    }
}

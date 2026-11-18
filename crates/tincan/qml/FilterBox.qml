import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Palette.js" as P

// One filter's parameters: a tall, narrow box in the edit window's filter list.
Frame {
    id: root

    property int index: 0
    // Palette index of the filter's color, which stays with it when filters are reordered.
    property int colorIndex: index
    property var filter: ({ type: "bell", channel: "LR", frequency: 1000, gain: 0, q: 1, enabled: true })
    property bool selected: false
    // The filter's dot is hovered on the graph.
    property bool highlighted: false
    readonly property alias boxHovered: hover.hovered

    signal changed(var filter)
    signal removeRequested()
    signal selectRequested()
    // Reordering by dragging the header; `x` is in the box's coordinates.
    signal reorderDragged(real x)
    signal reorderDropped(real x)

    width: 150
    padding: 8

    function withChange(key, value) {
        var f = Object.assign({}, filter);
        f[key] = value;
        if (key === "type" && filter.type !== value && Math.abs(filter.q - P.defaultQ(filter.type)) < 1e-3)
            f.q = P.defaultQ(value);
        return f;
    }

    background: Rectangle {
        color: root.palette.base
        border.width: root.selected || root.highlighted ? 2 : 1
        border.color: root.selected ? P.filterColor(root.colorIndex)
                    : root.highlighted ? Qt.alpha(P.filterColor(root.colorIndex), 0.5)
                    : Qt.alpha(root.palette.text, 0.2)
        radius: 4
    }

    HoverHandler { id: hover }

    TapHandler {
        onTapped: root.selectRequested()
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 6
        opacity: root.filter.enabled ? 1 : 0.6

        RowLayout {
            Layout.fillWidth: true

            Rectangle {
                id: handle
                Layout.fillWidth: true
                implicitHeight: 24
                radius: 3
                color: P.filterColor(root.colorIndex)
                Label {
                    anchors.centerIn: parent
                    text: (root.index + 1)
                    color: "white"
                    font.bold: true
                }
                ToolTip.visible: dragArea.containsMouse && !dragArea.pressed
                ToolTip.delay: 800
                ToolTip.text: "Drag to reorder"
                MouseArea {
                    id: dragArea
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: pressed ? Qt.ClosedHandCursor : Qt.OpenHandCursor
                    property bool moved: false
                    onPressed: { moved = false; root.selectRequested(); }
                    onPositionChanged: (mouse) => {
                        if (!pressed) return;
                        moved = true;
                        root.reorderDragged(mapToItem(root, mouse.x, mouse.y).x);
                    }
                    onReleased: (mouse) => {
                        if (moved) root.reorderDropped(mapToItem(root, mouse.x, mouse.y).x);
                    }
                }
            }

            Switch {
                padding: 0
                checked: root.filter.enabled
                onToggled: root.changed(root.withChange("enabled", checked))
                ToolTip.visible: hovered
                ToolTip.delay: 800
                ToolTip.text: root.filter.enabled ? "On" : "Bypassed"
            }

            ToolButton {
                text: "✕"
                implicitWidth: 28
                implicitHeight: 28
                ToolTip.visible: hovered
                ToolTip.delay: 600
                ToolTip.text: "Delete filter"
                onClicked: root.removeRequested()
            }
        }

        ComboBox {
            Layout.fillWidth: true
            model: P.kindLabels
            currentIndex: P.kinds.indexOf(root.filter.type)
            onActivated: (i) => root.changed(root.withChange("type", P.kinds[i]))
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: 0
            Repeater {
                model: P.channels
                Button {
                    Layout.fillWidth: true
                    implicitWidth: 30
                    text: modelData
                    checkable: true
                    checked: root.filter.channel === modelData
                    onClicked: {
                        checked = Qt.binding(function() { return root.filter.channel === modelData; });
                        root.changed(root.withChange("channel", modelData));
                    }
                }
            }
        }

        NumberField {
            Layout.fillWidth: true
            label: "Frequency"
            unit: "Hz"
            kind: "frequency"
            value: root.filter.frequency
            onEdited: (v) => root.changed(root.withChange("frequency", v))
        }

        NumberField {
            Layout.fillWidth: true
            label: "Gain"
            unit: "dB"
            kind: "gain"
            value: root.filter.gain
            onEdited: (v) => root.changed(root.withChange("gain", v))
        }

        NumberField {
            Layout.fillWidth: true
            label: "Sharpness (Q)"
            kind: "q"
            value: root.filter.q
            onEdited: (v) => root.changed(root.withChange("q", v))
        }

        Item { Layout.fillHeight: true }
    }
}

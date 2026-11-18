import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Palette.js" as P

// A labelled numeric input. Scrolling over it steps the value (Shift for fine steps).
ColumnLayout {
    id: root

    property string label
    property string unit
    // "frequency", "gain" or "q".
    property string kind: "gain"
    property real value: 0
    property bool readOnly: false

    signal edited(real value)

    spacing: 2

    Label {
        visible: root.label !== ""
        text: root.label
        font.pixelSize: 11
        opacity: 0.75
    }

    RowLayout {
        spacing: 4
        Layout.fillWidth: true

        TextField {
            id: field
            Layout.fillWidth: true
            Layout.preferredWidth: 70
            readOnly: root.readOnly
            horizontalAlignment: TextInput.AlignRight
            selectByMouse: true
            text: P.formatValue(root.kind, root.value)

            function commit() {
                var v = P.parseValue(root.kind, text);
                if (!isNaN(v) && v !== root.value)
                    root.edited(v);
                text = Qt.binding(function() { return P.formatValue(root.kind, root.value); });
            }

            onEditingFinished: commit()
            Keys.onEscapePressed: {
                text = Qt.binding(function() { return P.formatValue(root.kind, root.value); });
                focus = false;
            }

            WheelHandler {
                enabled: !root.readOnly
                acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                onWheel: (event) => {
                    var steps = event.angleDelta.y / 120;
                    if (steps === 0) return;
                    root.edited(P.stepValue(root.kind, root.value, steps, (event.modifiers & Qt.ShiftModifier) !== 0));
                }
            }
        }

        Label {
            text: root.unit
            opacity: 0.75
            visible: root.unit !== ""
        }
    }
}

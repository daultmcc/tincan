import QtQuick
import QtQuick.Controls

// A switch whose label names its current state. It reserves the width of its longest label,
// so changing state doesn't shift the surrounding layout.
Switch {
    id: root

    // Every label the switch can show.
    property var labels: []

    FontMetrics { id: metrics; font: root.font }

    readonly property real labelWidth:
        Math.ceil(labels.reduce(function(w, label) { return Math.max(w, metrics.advanceWidth(label)); }, 0))

    implicitWidth: leftPadding + (indicator ? indicator.width : 0) + spacing + labelWidth + rightPadding
}

import QtQuick
import "Palette.js" as P

// Frequency response plot. In editable mode it shows a dot per filter, which can be dragged,
// selected, deleted and scrolled (DESIGN.md, Edit window).
Item {
    id: root

    property var frequencies: []
    // { same, left, right, filters: [curve per filter] (optional), tilt: curve (optional) }
    property var curves: null
    // Filters for the dots (editable mode only), and each one's color index.
    property var filters: []
    property var colors: []
    property bool editable: false
    property int selectedIndex: -1
    // Dot under the mouse, and a dot highlighted from outside (its filter box is hovered).
    property int hoverIndex: -1
    property int linkedIndex: -1
    readonly property int highlightIndex: hoverIndex >= 0 ? hoverIndex : linkedIndex
    property bool dragging: false

    signal filterSelected(int index)
    signal filterMoved(int index, real frequency, real gain)
    signal filterCreated(real frequency, real gain)
    signal filterDeleted(int index)
    signal qScrolled(int index, real steps, bool fine)
    signal dragFinished()

    // Gain axis half-range in dB: at least 6, extended in 3 dB steps to fit the curves and dots
    // with 1 dB to spare.
    property real range: 6
    readonly property real leftMargin: 40
    readonly property real bottomMargin: 20
    readonly property real topMargin: 8
    readonly property real rightMargin: 10
    readonly property real plotWidth: Math.max(1, width - leftMargin - rightMargin)
    readonly property real plotHeight: Math.max(1, height - topMargin - bottomMargin)

    focus: editable
    activeFocusOnTab: editable

    function updateRange() {
        if (dragging)
            return;
        var m = 0;
        function scan(c) { if (c) for (var i = 0; i < c.length; i++) m = Math.max(m, Math.abs(c[i])); }
        if (curves) {
            scan(curves.left);
            scan(curves.right);
            if (curves.filters)
                for (var j = 0; j < curves.filters.length; j++) scan(curves.filters[j]);
            scan(curves.tilt);
        }
        for (var k = 0; k < filters.length; k++)
            m = Math.max(m, Math.abs(filters[k].gain));
        range = Math.max(6, Math.ceil((m + 1) / 3) * 3);
    }

    function repaint() {
        canvas.requestPaint();
        leftCanvas.requestPaint();
        rightCanvas.requestPaint();
        overlay.requestPaint();
    }

    readonly property bool darkTheme: palette.base.hslLightness < 0.5
    readonly property var channelColors: P.channelColors(darkTheme)

    // Clip `ctx` to the plot area.
    function clipToPlot(ctx) {
        ctx.beginPath();
        ctx.rect(leftMargin, topMargin, plotWidth, plotHeight);
        ctx.clip();
    }

    function plotCurve(ctx, curve, color, lineWidth) {
        if (!curve || curve.length !== frequencies.length) return;
        ctx.strokeStyle = color;
        ctx.lineWidth = lineWidth;
        // Round joins, so wide strokes (such as halos) never spike out at a corner, as the
        // default miter joins do.
        ctx.lineJoin = "round";
        // Quadratic segments between the midpoints of the polyline, with the points as control
        // points, so the path is smooth. Drawn as a polyline, with a join every few pixels, the
        // stroke's edges come out notched and speckled.
        var n = curve.length;
        var xs = [], ys = [];
        for (var i = 0; i < n; i++) {
            xs.push(xOf(frequencies[i]));
            ys.push(yOf(clampOver(curve[i])));
        }
        ctx.beginPath();
        ctx.moveTo(xs[0], ys[0]);
        for (var j = 1; j < n - 1; j++)
            ctx.quadraticCurveTo(xs[j], ys[j], (xs[j] + xs[j + 1]) / 2, (ys[j] + ys[j + 1]) / 2);
        ctx.lineTo(xs[n - 1], ys[n - 1]);
        ctx.stroke();
    }

    onCurvesChanged: { updateRange(); repaint(); }
    onFiltersChanged: { updateRange(); repaint(); }
    onSelectedIndexChanged: repaint()
    onHighlightIndexChanged: repaint()
    onRangeChanged: repaint()
    // The canvas only redraws on request, so follow theme changes explicitly.
    onPaletteChanged: repaint()
    onDraggingChanged: if (!dragging) updateRange()

    function xOf(f) { return leftMargin + Math.log(f / 20) / Math.log(1000) * plotWidth; }
    function fOf(x) { return 20 * Math.pow(1000, (x - leftMargin) / plotWidth); }
    function yOf(g) { return topMargin + plotHeight / 2 - g / range * plotHeight / 2; }
    function gOf(y) { return (topMargin + plotHeight / 2 - y) / (plotHeight / 2) * range; }
    // Keeps far off-scale values near the plot, where the clip hides them.
    function clampOver(g) { return Math.max(-range * 1.2, Math.min(range * 1.2, g)); }

    function colorOf(i) { return P.filterColor(colors[i] !== undefined ? colors[i] : i); }

    function dotAt(x, y) {
        var best = -1, bestD = 12;
        for (var i = 0; i < filters.length; i++) {
            var d = Math.hypot(xOf(filters[i].frequency) - x, yOf(filters[i].gain) - y);
            if (d < bestD) { bestD = d; best = i; }
        }
        return best;
    }

    Canvas {
        id: canvas
        anchors.fill: parent
        // All the canvases render to images, with the raster paint engine, which antialiases
        // with exact pixel coverage. FramebufferObject uses the OpenGL paint engine, whose few
        // samples per pixel leave the edges of shallow slopes ragged.
        renderTarget: Canvas.Image
        onPaint: {
            var ctx = getContext("2d");
            ctx.reset();
            var pal = root.palette;
            ctx.fillStyle = pal.base;
            ctx.fillRect(0, 0, width, height);

            // Grid.
            ctx.font = "10px sans-serif";
            ctx.lineWidth = 1;
            var gridColor = Qt.alpha(pal.text, 0.12);
            var labelColor = Qt.alpha(pal.text, 0.6);
            var decades = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
            ctx.textAlign = "center";
            for (var d = 0; d < decades.length; d++) {
                var x = Math.round(root.xOf(decades[d])) + 0.5;
                ctx.strokeStyle = gridColor;
                ctx.beginPath(); ctx.moveTo(x, root.topMargin); ctx.lineTo(x, root.topMargin + root.plotHeight); ctx.stroke();
                ctx.fillStyle = labelColor;
                ctx.fillText(P.formatFrequencyLabel(decades[d]), x, height - 6);
            }
            ctx.strokeStyle = Qt.alpha(pal.text, 0.06);
            for (var m = 10; m < 20000; m *= 10)
                for (var k = 2; k <= 9; k++) {
                    var f = m * k;
                    if (f <= 20 || f >= 20000 || k === 2 || k === 5) continue;
                    var xm = Math.round(root.xOf(f)) + 0.5;
                    ctx.beginPath(); ctx.moveTo(xm, root.topMargin); ctx.lineTo(xm, root.topMargin + root.plotHeight); ctx.stroke();
                }
            var step = root.range <= 12 ? 3 : 6;
            ctx.textAlign = "right";
            // Lines on multiples of the step, so 0 dB always has one.
            for (var g = -Math.floor(root.range / step) * step; g <= root.range; g += step) {
                var y = Math.round(root.yOf(g)) + 0.5;
                ctx.strokeStyle = g === 0 ? Qt.alpha(pal.text, 0.35) : gridColor;
                ctx.beginPath(); ctx.moveTo(root.leftMargin, y); ctx.lineTo(root.leftMargin + root.plotWidth, y); ctx.stroke();
                ctx.fillStyle = labelColor;
                ctx.fillText((g > 0 ? "+" : "") + g, root.leftMargin - 4, y + 3);
            }

            var freqs = root.frequencies;
            function fill(curve, color) {
                if (!curve || curve.length !== freqs.length) return;
                ctx.fillStyle = color;
                ctx.beginPath();
                ctx.moveTo(root.xOf(freqs[0]), root.yOf(0));
                for (var i = 0; i < curve.length; i++)
                    ctx.lineTo(root.xOf(freqs[i]), root.yOf(root.clampOver(curve[i])));
                ctx.lineTo(root.xOf(freqs[freqs.length - 1]), root.yOf(0));
                ctx.closePath();
                ctx.fill();
            }
            function plot(curve, color, lineWidth) { root.plotCurve(ctx, curve, color, lineWidth); }

            ctx.save();
            root.clipToPlot(ctx);
            var c = root.curves;
            if (c) {
                if (c.filters)
                    for (var fi = 0; fi < c.filters.length; fi++) {
                        var on = root.filters[fi] && root.filters[fi].enabled;
                        var emphasis = fi === root.selectedIndex ? 2 : fi === root.highlightIndex ? 1 : 0;
                        fill(c.filters[fi], Qt.alpha(root.colorOf(fi), on ? [0.1, 0.16, 0.22][emphasis] : 0.04));
                        plot(c.filters[fi], Qt.alpha(root.colorOf(fi), on ? [0.5, 0.75, 0.9][emphasis] : 0.15),
                             [1.25, 1.75, 2][emphasis]);
                    }
                if (c.tilt) {
                    ctx.setLineDash([5, 4]);
                    plot(c.tilt, Qt.alpha(pal.text, 0.55), 1.5);
                    ctx.setLineDash([]);
                }
            }
            ctx.restore();
        }
    }

    // The response curves, each with a halo that fades out away from the line. When the channels
    // match there is one curve, in the text color, on the left layer. Otherwise L and R are each
    // drawn normally on a transparent layer of their own, and the two layers are then blended
    // with each other only: added on dark themes, where their colors sum to white, and
    // multiplied on light ones, where they multiply to dark grey. The result is drawn normally
    // over the graph. The halos blend the same way as the lines, which softens the change where
    // the curves meet or part. The blend is a shader over both layers at full device
    // resolution, so it stays sharp at any display scale.
    component ChannelCanvas: Canvas {
        // Index into root.channelColors and curve key: 0 = left, 1 = right.
        property int channel
        anchors.fill: parent
        renderTarget: Canvas.Image

        readonly property real lineWidth: 2.5
        // The halo's opacity falls off as a Gaussian of the distance from the line's edge:
        // haloOpacity at the edge, with standard deviation haloSigma in pixels. It is drawn out
        // to 3 sigma (about 1% of the peak), in haloSteps strokes.
        readonly property real haloOpacity: 0.35
        readonly property real haloSigma: 1.5
        readonly property int haloSteps: 12
        readonly property real haloReach: 3 * haloSigma
        // Alpha of each stroke, widest last. Stroke k reaches haloReach * k / haloSteps beyond
        // the edge, so the band between strokes k - 1 and k is covered by strokes k and up;
        // their alphas are chosen so the opacity there, 1 - Π(1 - alpha), equals the Gaussian
        // at the band's middle.
        readonly property var haloAlphas: {
            var alphas = [], outer = 1;  // 1 - opacity of strokes k + 1 and up
            for (var k = haloSteps; k >= 1; k--) {
                var d = haloReach * (k - 0.5) / haloSteps;
                var inner = 1 - haloOpacity * Math.exp(-d * d / (2 * haloSigma * haloSigma));
                alphas[k - 1] = 1 - inner / outer;
                outer = inner;
            }
            return alphas;
        }

        onPaint: {
            var ctx = getContext("2d");
            ctx.reset();
            var c = root.curves;
            if (!c || (c.same && channel === 1)) return;
            var curve = channel === 0 ? c.left : c.right;
            var color = c.same ? root.palette.text : root.channelColors[channel];
            ctx.save();
            root.clipToPlot(ctx);
            for (var k = haloSteps; k >= 1; k--)
                root.plotCurve(ctx, curve, Qt.alpha(color, haloAlphas[k - 1]),
                               lineWidth + 2 * haloReach * k / haloSteps);
            root.plotCurve(ctx, curve, color, lineWidth);
            ctx.restore();
        }
    }

    ChannelCanvas { id: leftCanvas; channel: 0 }
    ChannelCanvas { id: rightCanvas; channel: 1 }

    ShaderEffect {
        anchors.fill: parent
        visible: !!root.curves
        property var leftLayer: ShaderEffectSource { sourceItem: leftCanvas; hideSource: true }
        property var rightLayer: ShaderEffectSource { sourceItem: rightCanvas; hideSource: true }
        property real multiply: root.darkTheme ? 0 : 1
        fragmentShader: "qrc:/qt/qml/org/tincan/shaders/channelblend.frag.qsb"
    }

    // Channel labels and the filter dots, above the curves.
    Canvas {
        id: overlay
        anchors.fill: parent
        renderTarget: Canvas.Image
        onPaint: {
            var ctx = getContext("2d");
            ctx.reset();
            var pal = root.palette;
            var c = root.curves;
            if (c && !c.same) {
                ctx.textAlign = "left";
                ctx.font = "bold 11px sans-serif";
                ctx.fillStyle = root.channelColors[0];
                ctx.fillText("L", root.leftMargin + 8, root.topMargin + 14);
                ctx.fillStyle = root.channelColors[1];
                ctx.fillText("R", root.leftMargin + 20, root.topMargin + 14);
            }

            if (root.editable) {
                function dotPos(i) {
                    return [root.xOf(root.filters[i].frequency),
                            root.yOf(Math.max(-root.range, Math.min(root.range, root.filters[i].gain)))];
                }
                for (var di = 0; di < root.filters.length; di++) {
                    var flt = root.filters[di];
                    var pos = dotPos(di);
                    var sel = di === root.selectedIndex, hov = di === root.highlightIndex;
                    if (hov) {
                        // A soft halo marks the dot under the mouse (or whose box is hovered).
                        ctx.beginPath();
                        ctx.arc(pos[0], pos[1], 13, 0, 2 * Math.PI);
                        ctx.fillStyle = Qt.alpha(root.colorOf(di), 0.25);
                        ctx.fill();
                    }
                    ctx.beginPath();
                    ctx.arc(pos[0], pos[1], sel ? 8 : hov ? 7.5 : 6, 0, 2 * Math.PI);
                    ctx.fillStyle = Qt.alpha(root.colorOf(di), flt.enabled ? 1 : 0.35);
                    ctx.fill();
                    ctx.lineWidth = sel ? 2.5 : 1;
                    ctx.strokeStyle = sel ? pal.text : pal.base;
                    ctx.stroke();
                }
                // Filter numbers beside the selected and highlighted dots, drawn last so dots
                // don't cover them.
                ctx.font = "bold 11px sans-serif";
                ctx.textAlign = "left";
                var labelled = [root.selectedIndex];
                if (root.highlightIndex !== root.selectedIndex) labelled.push(root.highlightIndex);
                for (var li = 0; li < labelled.length; li++) {
                    var n = labelled[li];
                    if (n < 0 || n >= root.filters.length) continue;
                    var lp = dotPos(n);
                    var text = String(n + 1);
                    var tw = ctx.measureText(text).width;
                    // Upper right of the dot, flipped inward near the plot's edges.
                    var lx = lp[0] + 11, ly = lp[1] - 11;
                    if (lx + tw + 6 > root.leftMargin + root.plotWidth) lx = lp[0] - 11 - tw - 6;
                    if (ly - 14 < root.topMargin) ly = lp[1] + 11 + 14;
                    ctx.fillStyle = Qt.alpha(pal.base, 0.85);
                    ctx.fillRect(lx, ly - 13, tw + 6, 16);
                    ctx.fillStyle = root.colorOf(n);
                    ctx.fillText(text, lx + 3, ly);
                }
            }
        }
    }

    Connections {
        target: root
        function onWidthChanged() { repaint(); }
        function onHeightChanged() { repaint(); }
    }

    MouseArea {
        anchors.fill: parent
        enabled: root.editable
        acceptedButtons: Qt.LeftButton
        hoverEnabled: true
        cursorShape: dragIndex >= 0 ? Qt.ClosedHandCursor : root.hoverIndex >= 0 ? Qt.OpenHandCursor : Qt.ArrowCursor
        property int dragIndex: -1
        property real offsetX: 0
        property real offsetY: 0

        onPressed: (mouse) => {
            root.forceActiveFocus();
            var i = root.dotAt(mouse.x, mouse.y);
            dragIndex = i;
            if (i >= 0) {
                offsetX = root.xOf(root.filters[i].frequency) - mouse.x;
                offsetY = root.yOf(root.filters[i].gain) - mouse.y;
            }
            root.filterSelected(i);
        }
        onPositionChanged: (mouse) => {
            if (!pressed) {
                root.hoverIndex = root.dotAt(mouse.x, mouse.y);
                return;
            }
            if (dragIndex < 0) return;
            root.hoverIndex = dragIndex;
            root.dragging = true;
            var f = P.clamp(root.fOf(mouse.x + offsetX), P.minFrequency, P.maxFrequency);
            var g = P.clamp(Math.round(root.gOf(mouse.y + offsetY) * 10) / 10, P.minGain, P.maxGain);
            root.filterMoved(dragIndex, f, g);
        }
        onReleased: {
            if (root.dragging) {
                root.dragging = false;
                root.dragFinished();
            }
            dragIndex = -1;
            root.hoverIndex = containsMouse ? root.dotAt(mouseX, mouseY) : -1;
        }
        onExited: if (!pressed) root.hoverIndex = -1
        onDoubleClicked: (mouse) => {
            if (root.dotAt(mouse.x, mouse.y) >= 0) return;
            if (mouse.x < root.leftMargin || mouse.y > root.topMargin + root.plotHeight) return;
            var f = P.clamp(root.fOf(mouse.x), P.minFrequency, P.maxFrequency);
            var g = P.clamp(Math.round(root.gOf(mouse.y) * 10) / 10, P.minGain, P.maxGain);
            root.filterCreated(f, g);
        }
        onWheel: (wheel) => {
            // The hovered dot, else the selected one.
            var i = root.hoverIndex >= 0 ? root.hoverIndex : root.selectedIndex;
            if (i < 0) { wheel.accepted = false; return; }
            root.qScrolled(i, wheel.angleDelta.y / 120, (wheel.modifiers & Qt.ShiftModifier) !== 0);
        }
    }

    Keys.onPressed: (event) => {
        if ((event.key === Qt.Key_Delete || event.key === Qt.Key_Backspace) && root.selectedIndex >= 0) {
            root.filterDeleted(root.selectedIndex);
            event.accepted = true;
        }
    }
}

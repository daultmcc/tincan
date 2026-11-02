.pragma library

// Filter colours, chosen to stay distinct on light and dark backgrounds.
var filterColors = ["#e6194b", "#3cb44b", "#4363d8", "#f58231", "#911eb4", "#42d4f4",
                    "#f032e6", "#9a9a00", "#469990", "#9a6324", "#800000", "#000075"];

function filterColor(index) {
    return filterColors[index % filterColors.length];
}

var leftColor = "#3a8fd8";
var rightColor = "#e0645a";

var kinds = ["bell", "high_shelf", "low_shelf"];
var kindLabels = ["Bell", "High shelf", "Low shelf"];
var channels = ["L", "R", "LR"];

function defaultQ(kind) {
    return kind === "bell" ? 1.0 : 0.7071;
}

// Parameter ranges and scroll steps (DESIGN.md, Parameter ranges and defaults).
var minFrequency = 20, maxFrequency = 20000;
var minGain = -24, maxGain = 24;
var minQ = 0.1, maxQ = 40;

function clamp(v, lo, hi) {
    return Math.min(hi, Math.max(lo, v));
}

// `steps` is in wheel notches; `fine` when Shift is held.
function stepValue(kind, value, steps, fine) {
    if (kind === "frequency")
        return clamp(value * Math.pow(2, steps / (fine ? 96 : 24)), minFrequency, maxFrequency);
    if (kind === "gain")
        return clamp(Math.round((value + steps * (fine ? 0.01 : 0.1)) * 100) / 100, minGain, maxGain);
    return clamp(value * Math.pow(fine ? 1.01 : 1.05, steps), minQ, maxQ);
}

function formatValue(kind, v) {
    if (kind === "frequency") {
        if (v >= 1000)
            return (v / 1000).toFixed(v >= 10000 ? 2 : 3).replace(/\.?0+$/, "") + "k";
        return v.toFixed(v >= 100 ? 0 : 1);
    }
    if (kind === "gain")
        return v.toFixed(2);
    return v.toFixed(v >= 10 ? 2 : 3);
}

// Parses user input such as "1.2k", "6150" or "6150 Hz". Returns NaN when invalid.
function parseValue(kind, text) {
    var t = text.trim().toLowerCase().replace(/hz$|db$/, "").trim();
    var mult = 1;
    if (kind === "frequency" && t.endsWith("k")) {
        mult = 1000;
        t = t.slice(0, -1);
    }
    if (!/^[-+]?(\d+\.?\d*|\.\d+)$/.test(t))
        return NaN;
    var v = parseFloat(t) * mult;
    if (kind === "frequency")
        return clamp(v, minFrequency, maxFrequency);
    if (kind === "gain")
        return clamp(v, minGain, maxGain);
    return clamp(v, minQ, maxQ);
}

function formatFrequencyLabel(f) {
    return f >= 1000 ? (f / 1000) + "k" : "" + f;
}

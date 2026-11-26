.pragma library

// Filter colours, chosen to stay distinct on light and dark backgrounds. The first several
// stay clear of the channel colours (blue and orange, below); those hues come last.
var filterColors = ["#3cb44b", "#f032e6", "#c09a00", "#469990", "#911eb4", "#e6194b",
                    "#42d4f4", "#9a6324", "#4363d8", "#f58231"];

function filterColor(index) {
    return filterColors[index % filterColors.length];
}

// Channel colors for split L and R curves: complementary blue and orange, which blend where
// the curves overlap. On dark themes the curves are added, and these sum to white in every
// component; on light themes they are multiplied, and these multiply to dark grey (0.2).
var darkChannelColors = ["#3a8fd8", "#d8833a"];
var lightChannelColors = ["#4080e6", "#cc6638"];

function channelColors(dark) {
    return dark ? darkChannelColors : lightChannelColors;
}

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
var minTilt = -3, maxTilt = 3;

function clamp(v, lo, hi) {
    return Math.min(hi, Math.max(lo, v));
}

// The 1-2-5 series value (…, 0.1, 0.2, 0.5, 1, 2, 5, …) nearest `x` on a log scale.
function niceIncrement(x) {
    var p = Math.pow(10, Math.floor(Math.log10(x)));
    var best = p;
    [1, 2, 5, 10].forEach(function(m) {
        if (Math.abs(Math.log(m * p / x)) < Math.abs(Math.log(best / x))) best = m * p;
    });
    return best;
}

// Snap `target` to a multiple of `inc`, always moving at least one increment away from
// `value` in the direction of travel (so a notch never does nothing).
function snapTo(value, target, inc) {
    var r = Math.round(target / inc) * inc;
    if (target > value && r <= value) r = (Math.floor(value / inc + 1e-9) + 1) * inc;
    if (target < value && r >= value) r = (Math.ceil(value / inc - 1e-9) - 1) * inc;
    return parseFloat(r.toPrecision(12));
}

// `steps` is in wheel notches; `fine` when Shift is held. Each notch moves by a roughly
// constant ratio (frequency, Q) or amount (gain), landing on a round value: a multiple of
// the 1-2-5 increment nearest the step size.
function stepValue(kind, value, steps, fine) {
    if (kind === "gain" || kind === "tilt") {
        var inc = fine ? 0.01 : 0.1;
        var r0 = snapTo(value, value + steps * inc, inc);
        return kind === "gain" ? clamp(r0, minGain, maxGain) : clamp(r0, minTilt, maxTilt);
    }
    var ratio = kind === "frequency" ? Math.pow(2, steps / (fine ? 96 : 24))
                                     : Math.pow(fine ? 1.01 : 1.05, steps);
    var target = value * ratio;
    var r = snapTo(value, target, niceIncrement(Math.abs(target - value)));
    return kind === "frequency" ? clamp(r, minFrequency, maxFrequency) : clamp(r, minQ, maxQ);
}

function formatValue(kind, v) {
    if (kind === "frequency") {
        if (v >= 1000)
            return (v / 1000).toFixed(v >= 10000 ? 2 : 3).replace(/\.?0+$/, "") + "k";
        return v.toFixed(v >= 100 ? 0 : 1).replace(/\.0$/, "");
    }
    if (kind === "gain" || kind === "tilt")
        return v.toFixed(2);
    // Q: up to three decimals, trailing zeros dropped down to one ("0.707", "0.75", "2.0").
    return v.toFixed(v >= 10 ? 2 : 3).replace(/(\.\d*?)0+$/, "$1").replace(/\.$/, ".0");
}

// Parses user input such as "1.2k", "6150", "6150 Hz" or "-1 dB/oct". Returns NaN when
// invalid.
function parseValue(kind, text) {
    var t = text.trim().toLowerCase().replace(/hz$|db(\s*\/\s*oct(ave)?)?$/, "").trim();
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
    if (kind === "tilt")
        return clamp(v, minTilt, maxTilt);
    return clamp(v, minQ, maxQ);
}

function formatFrequencyLabel(f) {
    return f >= 1000 ? (f / 1000) + "k" : "" + f;
}

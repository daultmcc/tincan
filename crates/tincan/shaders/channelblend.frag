#version 440
// Blends the left and right channel layers of ResponseGraph.qml with each other: added on
// dark themes, multiplied on light ones. Both layers and the output are premultiplied.

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4 qt_Matrix;
    float qt_Opacity;
    float multiply;
};

layout(binding = 1) uniform sampler2D leftLayer;
layout(binding = 2) uniform sampler2D rightLayer;

void main() {
    vec4 a = texture(leftLayer, qt_TexCoord0);
    vec4 b = texture(rightLayer, qt_TexCoord0);
    // Plus: the sum, clamped. Multiply (premultiplied, as in the W3C compositing spec): where
    // only one layer covers a pixel it shows unchanged, where both do their colors multiply, and
    // the coverage is the union of both.
    vec4 added = min(a + b, vec4(1.0));
    vec4 multiplied = a * b + a * (1.0 - b.a) + b * (1.0 - a.a);
    fragColor = mix(added, multiplied, multiply) * qt_Opacity;
}

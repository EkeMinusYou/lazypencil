precision highp float;

/** @resolution */
uniform vec2 size;
const int PALETTE_SIZE = 3;
uniform vec3 palette[PALETTE_SIZE];
uniform float stops[2], contrast;
uniform mat2 uvTransform;

void main() {
    vec2 p = uvTransform * (gl_FragCoord.xy / size - vec2(0.5)) + vec2(0.5);
    float t = clamp(p.x, 0.0, 1.0);
    float middle = clamp(stops[0], 0.01, 0.99);
    vec3 color = t < middle
        ? mix(palette[0], palette[1], t / middle)
        : mix(palette[1], palette[2], (t - middle) / (1.0 - middle));
    float stripe = step(0.96, fract(p.y * max(stops[1], 1.0)));
    color = mix(color, vec3(1.0), stripe * 0.16);
    gl_FragColor = vec4(pow(max(color, vec3(0.0)), vec3(contrast)), 1.0);
}

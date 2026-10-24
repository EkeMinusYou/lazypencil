precision highp float;
/** @resolution */ uniform vec2 resolution;
/** @time */ uniform float clock;
uniform vec4 accent;
void main() {
  vec2 uv = gl_FragCoord.xy / resolution;
  float wave = 0.5 + 0.5 * sin(uv.x * 15.0 + uv.y * 11.0 + clock);
  vec3 color = mix(vec3(0.06, 0.09, 0.16), accent.rgb, uv.x);
  gl_FragColor = vec4(color + vec3(0.12 * wave), 1.0);
}

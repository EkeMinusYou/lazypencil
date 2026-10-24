precision highp float;
#define gl_FragCoord(v) vec4(v)
void main(){gl_FragColor=vec4(gl_FragCoord.xy/vec2(32.,16.),gl_FragCoord(0.25).x,1.);}

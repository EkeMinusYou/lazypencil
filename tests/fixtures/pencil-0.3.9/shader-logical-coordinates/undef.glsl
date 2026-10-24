precision highp float;
#define gl_FragCoord vec4(16.,8.,0.,1.)
#undef gl_FragCoord
void main(){gl_FragColor=vec4(gl_FragCoord.xy/vec2(32.,16.),0.,1.);}

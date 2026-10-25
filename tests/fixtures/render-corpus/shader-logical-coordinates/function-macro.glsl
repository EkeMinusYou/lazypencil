precision highp float;
#define UV(p) ((p).xy/vec2(32.,16.))
void main(){gl_FragColor=vec4(UV(gl_FragCoord),0.,1.);}

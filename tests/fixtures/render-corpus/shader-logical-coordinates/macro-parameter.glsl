precision highp float;
#define UV(gl_FragCoord) ((gl_FragCoord).xy/vec2(32.,16.))
void main(){gl_FragColor=vec4(UV(vec4(16.,8.,0.,1.)),0.,1.);}

precision highp float;
/** @resolution */
#define UNUSED 1
uniform vec2 size;
void main(){gl_FragColor=vec4(size/vec2(128.,64.),0.,1.);}

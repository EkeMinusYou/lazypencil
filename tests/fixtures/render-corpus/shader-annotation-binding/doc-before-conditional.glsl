precision highp float;
/** @resolution */
#if 1
uniform vec2 size;
#endif
void main(){gl_FragColor=vec4(size/vec2(128.,64.),0.,1.);}

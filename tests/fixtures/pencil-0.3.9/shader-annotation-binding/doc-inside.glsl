precision highp float;
uniform /** @resolution */ vec2 size;
void main(){gl_FragColor=vec4(size/vec2(128.,64.),0.,1.);}

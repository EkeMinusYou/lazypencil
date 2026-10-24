precision highp float;
uniform vec2 size; /** @resolution */
void main(){gl_FragColor=vec4(size/vec2(128.,64.),0.,1.);}

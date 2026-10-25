precision highp float;
/** @resolution */
uniform vec2 size,other;
void main(){gl_FragColor=vec4((size+other)/vec2(128.,64.),0.,1.);}

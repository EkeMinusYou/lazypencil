precision highp float;
/** @resolution */ uniform vec2 size;
void main(){gl_FragColor=vec4(gl_FragCoord.xy/size,0.,1.);}

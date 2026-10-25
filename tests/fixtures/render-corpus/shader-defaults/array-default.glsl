precision highp float;
/** @default 0.25 */
uniform float value[2];
void main(){gl_FragColor=vec4(value[0],value[1],0.0,1.0);}

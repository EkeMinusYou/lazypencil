precision highp float;
/** @default 0x2 */
uniform float value;
void main(){gl_FragColor=vec4(value/4.0,0.0,0.0,1.0);}

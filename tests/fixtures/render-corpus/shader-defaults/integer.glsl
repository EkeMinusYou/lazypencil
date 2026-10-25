precision highp float;
/** @default 3 */
uniform int value;
void main(){gl_FragColor=vec4(float(value)/4.0,0.0,0.0,1.0);}

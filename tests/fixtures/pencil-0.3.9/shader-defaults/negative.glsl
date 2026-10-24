precision highp float;
/** @default -0.25 */
uniform float value;
void main(){gl_FragColor=vec4(value+0.5,0.0,0.0,1.0);}

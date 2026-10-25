precision highp float;
#define UV (gl_FragCoord.xy/vec2(32.,16.))
void main(){gl_FragColor=vec4(UV,0.,1.);}

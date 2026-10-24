/**
 * @schema 2.11
 * @input marker: number = 1
 */
const nodes=[];
const positions=[...Array(32).keys(),63,255,1023,4095,65535];
void pencil.width; void pencil.height; void pencil.input.marker;
let i=0;
for(let draw=0;draw<=65535;draw++) {
 const value=Math.random();
 if(draw!==positions[i]) continue;
 const n=Math.floor(value*281474976710656);
 for(let half=0;half<2;half++) {
  const bits=half ? n%16777216 : Math.floor(n/16777216);
  nodes.push({type:"rectangle",x:i*16,y:half*8,width:12,height:8,fill:"#"+bits.toString(16).padStart(6,"0")});
 }
 i++;
}
return nodes;

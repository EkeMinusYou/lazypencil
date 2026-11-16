/**
 * @schema 2.19
 * @input value: number = 4
 */
const text=JSON.stringify([typeof pencil.input.value,pencil.input.value,Object.keys(pencil.input).sort(),pencil.input.extra]);
const nodes=[{type:"rectangle",x:0,y:0,width:4,height:4,fill:"#"+text.length.toString(16).padStart(6,"0")}];
for(let i=0;i<Math.min(text.length,62);i++)nodes.push({type:"rectangle",x:(i+1)*4,y:0,width:4,height:4,fill:"#"+text.charCodeAt(i).toString(16).padStart(6,"0")});
return nodes;

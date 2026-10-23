/**
 * @schema 2.21
 * @input color: color = #2563EB
 * @input maximum: number(min=1) = 200
 * @input rounded: boolean = true
 */
const values = [64, 95, 82, 130, 110, 154, 144];
const labels = ["月", "火", "水", "木", "金", "土", "日"];
const baseline = Math.max(0, pencil.height - 32);
const column = pencil.width / values.length;
const nodes = [{
  type: "rectangle", x: 0, y: baseline,
  width: pencil.width, height: 1, fill: "#CBD5E1",
}];
for (let index = 0; index < values.length; index++) {
  const height = baseline * Math.min(1, values[index] / pencil.input.maximum);
  nodes.push({
    type: "rectangle",
    name: `Sample bar ${index + 1}`,
    x: (index + 0.15) * column, y: baseline - height,
    width: column * 0.7, height,
    cornerRadius: pencil.input.rounded ? 6 : 0,
    fill: pencil.input.color,
  });
  nodes.push({
    type: "text", x: index * column, y: baseline + 8,
    width: column, textGrowth: "fixed-width", textAlign: "center",
    fontFamily: "Noto Sans JP", fontSize: 14, fill: "#475569",
    content: labels[index],
  });
}
return nodes;

(() => {
  "use strict";
  const parse = JSON.parse;
  const stringify = JSON.stringify;
  const isArray = Array.isArray;
  const finite = Number.isFinite;
  const width = globalThis.__script_width;
  const height = globalThis.__script_height;
  const input = Object.freeze(Object.assign(Object.create(null), parse(globalThis.__script_inputs)));
  delete globalThis.__script_inputs;
  delete globalThis.__script_width;
  delete globalThis.__script_height;
  let reads_width = false;
  let reads_height = false;
  let state = 0x6d2b79f5;
  Object.defineProperty(Math, "random", {
    value() {
      state ^= state << 13;
      state ^= state >>> 17;
      state ^= state << 5;
      return (state >>> 0) / 4294967296;
    },
    writable: false,
    configurable: false,
  });
  const pencil = Object.freeze({
    get width() { reads_width = true; return width; },
    get height() { reads_height = true; return height; },
    input,
  });
  return function run(script) {
    const nodes = script(pencil);
    if (!isArray(nodes)) throw new TypeError("A script must synchronously return a node array");
    const data = stringify(nodes, (key, value) => {
      const type = typeof value;
      if (type === "number" && !finite(value)) throw new TypeError("Script output contains a non-finite number");
      if (type === "function" || type === "symbol" || type === "bigint" || type === "undefined") throw new TypeError("Script output must contain only JSON data");
      return value;
    });
    const normalized = parse(data);
    if (!isArray(normalized)) throw new TypeError("A script must return a JSON node array");
    return stringify({ nodes: normalized, reads_width, reads_height });
  };
})()

// Exercise the actual Wasm ABI and memory boundary without a model or browser.
import assert from "node:assert/strict";
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const binary = fs.readFileSync(process.argv[2]);
assert(binary.length < 16 * 1024 * 1024);
const module = new WebAssembly.Module(binary);
assert.deepEqual(WebAssembly.Module.imports(module), [{ module: "env", name: "memory", kind: "memory" }]);
const execute = (source, options = {}) => {
  const memory = new WebAssembly.Memory({ initial: 256, maximum: 1024 });
  const instance = new WebAssembly.Instance(module, { env: { memory } });
  const input = new TextEncoder().encode(JSON.stringify({ source, defines: ["GL_ES", "GL_FRAGMENT_PRECISION_HIGH"], coordinate_scale: [0.5, 0.5], ...options }));
  const pointer = instance.exports.input_buffer(input.length);
  assert(pointer > 0 && pointer + input.length <= memory.buffer.byteLength);
  new Uint8Array(memory.buffer, pointer, input.length).set(input);
  const result = instance.exports.process_shader();
  const length = instance.exports.output_length();
  assert(length > 0 && length <= 8 * 1024 * 1024 && result + length <= memory.buffer.byteLength);
  const output = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(memory.buffer, result, length)));
  assert(memory.buffer.byteLength <= 64 * 1024 * 1024);
  return output;
};
const compact = source => source.replace(/\s+/g, "");
const prepare = (source, options) => {
  const result = execute(source, options);
  assert.equal(result.error, undefined, JSON.stringify(result));
  assert.equal(typeof result.source, "string");
  return compact(result.source);
};
const body = expression => `precision highp float;void main(){gl_FragColor=${expression};}`;

if (process.argv[3] === "--bomb") {
  let definitions = "#define F0 1.\n";
  for (let level = 1; level <= 32; level++) definitions += `#define F${level} (F${level - 1}+F${level - 1})\n`;
  try {
    const result = execute(definitions + body("vec4(F32)"));
    assert.equal(typeof result.error, "string", JSON.stringify(result).slice(0, 512));
    console.log(JSON.stringify({ contained: true, reason: result.error.slice(0, 256) }));
  } catch (error) {
    assert(error instanceof WebAssembly.RuntimeError, String(error));
    console.log(JSON.stringify({ contained: true, reason: "Wasm trapped within its memory/stack boundary" }));
  }
} else {
  const cases = [];
  const check = (name, callback) => { callback(); cases.push(name); };
  check("nested function arguments are fully expanded", () => {
    const result = prepare("#define F(x) ((x)+(x))\n" + body("vec4(F(F(0.125)))"));
    assert(!result.includes("F("), result);
    assert.equal((result.match(/0\.125/g) || []).length, 4, result);
  });
  check("unused macro arguments are not expanded", () => {
    const result = prepare("#define IGNORE(x) vec4(0.5)\n#define TAKES_TWO(x,y) x\n" + body("IGNORE(TAKES_TWO(1.))"));
    assert(result.includes("vec4(0.5)"), result);
  });
  check("ten original native coordinate fixtures retain their builtin bindings", () => {
    const directory = new URL("../fixtures/render-corpus/shader-logical-coordinates/", import.meta.url);
    const files = fs.readdirSync(directory).filter(name => name.endsWith(".glsl"));
    assert.equal(files.length, 10);
    for (const file of files) {
      const result = prepare(fs.readFileSync(new URL(file, directory), "utf8"));
      const replaced = !["undefined-builtin.glsl", "object-override.glsl", "macro-parameter.glsl"].includes(file);
      assert.equal((result.match(/\bgl_FragCoord\b/g) || []).length, replaced ? 2 : 0, file);
      assert(!result.includes("#define"), file);
      if (replaced) {
        assert(result.includes("gl_FragCoord.xy*vec2(5.00000000000000000e-1,5.00000000000000000e-1)"), file);
        assert(result.includes("gl_FragCoord.zw"), file);
      } else if (file === "undefined-builtin.glsl") {
        assert(result.includes("vec4(0.,1.,0.,1.)"), file);
      } else {
        assert(result.includes("vec4(16.,8.,0.,1.)"), file);
      }
      if (file === "function-override.glsl") assert(result.includes("vec4(0.25)"), file);
    }
  });
  check("only actual browser predefined macros are available", () => {
    const result = prepare("#if !defined(GL_ES) || __VERSION__ != 100 || !defined(GL_FRAGMENT_PRECISION_HIGH)\n#error incorrect builtins\n#endif\n#ifdef GL_ARB_shading_language_include\n#error desktop extension leaked\n#endif\n" + body("vec4(0.,1.,0.,1.)"));
    assert(result.includes("vec4(0.,1.,0.,1.)"));
  });
  check("extension directives remain before shader code", () => {
    const result = prepare("/* header */\n#version 100\n#extension GL_OES_standard_derivatives : enable\n" + body("vec4(dFdx(gl_FragCoord.x))"), { defines: ["GL_ES", "GL_OES_standard_derivatives"] });
    assert(result.indexOf("#extension") < result.indexOf("precision"));
    assert(result.includes("dFdx("));
  });
  check("line and file builtins are expanded without changing version", () => {
    const result = prepare("#line 20 7\nconst int file = __FILE__;const int version = __VERSION__;const int line = __LINE__;\n" + body("vec4(1.)"));
    assert(result.includes("file=7;"));
    assert(result.includes("version=100;"));
    assert(!result.includes("__LINE__") && !result.includes("__FILE__"));
    assert(result.includes("line=20;"), result);
  });
  check("line numbers match source lines and explicit zero resets", () => {
    let result = prepare("int first=__LINE__;\nint second=__LINE__;");
    assert(result.includes("first=1;") && result.includes("second=2;"), result);
    result = prepare("#version 100\nint second=__LINE__;");
    assert(result.includes("second=2;"), result);
    result = prepare("/* first\nsecond */\nint third=__LINE__;");
    assert(result.includes("third=3;"), result);
    result = prepare("#line 0\nint zero=__LINE__;\nint one=__LINE__;");
    assert(result.includes("zero=0;") && result.includes("one=1;"), result);
  });
  check("invalid requests and include requests fail", () => {
    for (const [source, options] of [["a".repeat(1024 * 1024 + 1), {}], [body("vec4(1.)"), { coordinate_scale: [0, 1] }], [body("vec4(1.)"), { defines: ["arbitrary"] }], ["#include \"secret\"\n" + body("vec4(1.)"), {}]]) {
      assert.equal(typeof execute(source, options).error, "string");
    }
  });
  const bomb = spawnSync(process.execPath, [fileURLToPath(import.meta.url), process.argv[2], "--bomb"], { encoding: "utf8", timeout: 5000, maxBuffer: 65536 });
  assert.equal(bomb.error, undefined, String(bomb.error));
  assert.equal(bomb.status, 0, bomb.stderr);
  assert.equal(JSON.parse(bomb.stdout).contained, true);
  cases.push("amplifying macro is contained in five seconds and 64 MiB");
  const report = { passed: cases.length, cases, imports: WebAssembly.Module.imports(module), exports: WebAssembly.Module.exports(module), memory_maximum_bytes: 64 * 1024 * 1024, node: process.version, scope: "Actual standalone Wasm ABI and preprocessing; browser drawing and full native parity require separate tests." };
  fs.writeFileSync(process.argv[3], JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify(report));
}

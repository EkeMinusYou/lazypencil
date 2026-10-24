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
  const expression = "F(".repeat(32) + "1." + ")".repeat(32);
  try {
    const result = execute("#define F(x) ((x)+(x))\n" + body(`vec4(${expression})`));
    assert.equal(typeof result.error, "string");
    console.log(JSON.stringify({ contained: true, reason: result.error.slice(0, 256) }));
  } catch (error) {
    assert(error instanceof WebAssembly.RuntimeError, String(error));
    console.log(JSON.stringify({ contained: true, reason: "Wasm trapped within its memory/stack boundary" }));
  }
} else {
  const cases = [];
  const check = (name, callback) => { callback(); cases.push(name); };
  check("object macro retains logical builtin coordinates", () => {
    const result = prepare("#define UV (gl_FragCoord.xy/vec2(32.,16.))\n" + body("vec4(UV,0.,1.)"));
    assert(result.includes("gl_FragCoord.xy*vec2(5.00000000000000000e-1,5.00000000000000000e-1)"));
    assert(!result.includes("#define"));
  });
  check("function macro expands the genuine builtin", () => {
    const result = prepare("#define UV(p) ((p).xy/vec2(32.,16.))\n" + body("vec4(UV(gl_FragCoord),0.,1.)"));
    assert(result.includes("gl_FragCoord.xy*vec2("));
    assert(result.includes("gl_FragCoord.zw"));
  });
  check("builtin is undefined to conditional preprocessing", () => {
    const result = prepare("#ifdef gl_FragCoord\n#error builtin unexpectedly defined\n#endif\n" + body("vec4(0.,1.,0.,1.)"));
    assert(result.includes("vec4(0.,1.,0.,1.)"));
  });
  check("user object override is not scaled", () => {
    const result = prepare("#define gl_FragCoord vec4(16.,8.,0.,1.)\n" + body("gl_FragCoord"));
    assert(!result.includes("gl_FragCoord"));
    assert(result.includes("vec4(16.,8.,0.,1.)"));
  });
  check("macro parameter named like the builtin remains a parameter", () => {
    const result = prepare("#define UV(gl_FragCoord) ((gl_FragCoord).xy/vec2(32.,16.))\n" + body("vec4(UV(vec4(16.,8.,0.,1.)),0.,1.)"));
    assert(!result.includes("gl_FragCoord"));
    assert(result.includes("vec4(16.,8.,0.,1.)"));
  });
  check("function override preserves bare builtin and overridden call", () => {
    const result = prepare("#define gl_FragCoord(x) vec4(x)\n" + body("gl_FragCoord + gl_FragCoord(0.25)"));
    assert(result.includes("gl_FragCoord.xy*vec2("));
    assert(result.includes("vec4(0.25)"));
  });
  check("ten original native coordinate fixtures retain their builtin bindings", () => {
    const directory = new URL("../fixtures/pencil-0.3.9/shader-logical-coordinates/", import.meta.url);
    const files = fs.readdirSync(directory).filter(name => name.endsWith(".glsl"));
    assert.equal(files.length, 10);
    for (const file of files) {
      const result = prepare(fs.readFileSync(new URL(file, directory), "utf8"));
      const replaced = !["undefined-builtin.glsl", "object-override.glsl", "macro-parameter.glsl"].includes(file);
      assert.equal((result.match(/\bgl_FragCoord\b/g) || []).length, replaced ? 2 : 0, file);
    }
  });
  check("undef restores the genuine builtin", () => {
    const result = prepare("#define gl_FragCoord vec4(1.)\n#undef gl_FragCoord\n" + body("gl_FragCoord"));
    assert(result.includes("gl_FragCoord.xy*vec2("));
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
  });
  check("invalid requests and include requests fail", () => {
    for (const [source, options] of [["a".repeat(1024 * 1024 + 1), {}], [body("vec4(1.)"), { coordinate_scale: [0, 1] }], [body("vec4(1.)"), { defines: ["arbitrary"] }], ["#include \"secret\"\n" + body("vec4(1.)"), {}]]) {
      assert.equal(typeof execute(source, options).error, "string");
    }
  });
  check("module memory maximum is enforced by the engine", () => {
    const memory = new WebAssembly.Memory({ initial: 256, maximum: 2048 });
    assert.throws(() => new WebAssembly.Instance(module, { env: { memory } }), WebAssembly.LinkError);
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

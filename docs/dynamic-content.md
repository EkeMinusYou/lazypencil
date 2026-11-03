# Dynamic content

Script nodes generate ordinary layout nodes in a QuickJS worker. Browser nodes capture
web pages, and GLSL fills/strokes/effects use software WebGL in a separate Chromium
process. Skia composites the results with native graphics. Chromium is installed
[explicitly](runtimes.md); rendering never downloads a missing runtime.

## JavaScript

A `script` node supplies `scriptUri`, dimensions, and optional `inputs`. Its source
starts with a header comment such as:

```javascript
/**
 * @schema 2.21
 * @input label: string = "Hello"
 */
return [{type: "text", content: pencil.input.label, fontSize: 16}];
```

Headers accept 2.11, 2.19, and 2.21; supported document codecs remain 2.19/2.21.
The worker exposes `pencil.width`, `pencil.height`, and `pencil.input` and synchronously
expects a node array. It has no DOM, host filesystem/network, timers, or module loader.
Each evaluation has a fresh context. `Math.random` uses Mulberry32, reseeded to zero
for every evaluation.

Typed inputs are `number`, `string`, `boolean`, `color`, `enum`, and `ref`. Omission or
null uses a declared default; otherwise the implicit values are clamped zero, empty
string, false, `#000000`, first enum option, and empty ref respectively. Empty ref means
no component selection. Nonempty refs select existing reusable definitions. Saved keys
absent from the header remain stored but do not enter execution or variable lookup.
See the [script guide](../resources/guidance/scripts.md) for input and diagnostic rules.

Generated records are derived data with `generated_by` provenance. Read them through
`inspect --calculated` or MCP calculated `read_nodes`; edit their owning script, inputs,
or source node. A retained `snapshot_id` reads the same calculation without rerunning
code. A script reading a content-sized dimension needs a definite dependency or explicit
fallback; nonconvergent calculations fail rather than inventing a size.

The defaults are 2 seconds/64 MiB per evaluation, 8 layout passes, 128 evaluations, and
a 10-second generated-layout budget. Code is limited to 1 MiB, protocol frames to 4 MiB,
output to 1,000 nodes/64 levels, and the JS stack to 512 KiB. Captured
[JavaScript policy](configuration.md#javascript-execution-limits) controls configurable
limits; catching an interpreter interruption cannot turn that evaluation into success.

## Script copy and detach

Detachment preserves linked code/input ownership instead of flattening generated
geometry. An independent component copy closes component, variable, theme, and static
asset dependencies without executing or rewriting JavaScript. Use `options.script_assets`
for files selected only inside code, keyed by the qualified source script node.

Code, output, saved-input, variable, and browser-resource namespaces remain distinct
through nested generation and recopy. An explicit code/input update selects the caller's
context for that field even when its JSON is unchanged; other fields keep inheritance.
Private `metadata.lazypencil` scope/binding/input-generation records preserve this
ownership. They are not user-editable path permissions or a Pencil round-trip contract.
Missing legacy provenance produces a diagnostic and requires recopying from the source.
See [component import](mcp.md#importing-components).

## Browser nodes

Browser properties include `url`, `width`, `height`, `zoom`, `scrollX`, `scrollY`, and
optional `deviceId`. HTTP(S), HTML data URLs, and `about:blank` are supported. Credentials
embedded in URLs are rejected. Basic desktop/phone/tablet modes do not select a named
device model or User-Agent. Captures receive ordinary transforms, opacity, clipping,
borders, and supported effects.

### Local resources

A file page needs a declared captured directory, within the existing project read roots:

```json
{"op":"set_browser_resources","roots":[{"url":"file:///original/site/","path":"web"}]}
```

The canonical original directory URL ends in `/`; `path` is relative to the owning
document. A node can then reference `file:///original/site/index.html`. Only captured
files from its namespace are served. `pack_browser_resources` saves them as a portable
managed bundle. Empty roots remove the declaration. Changed resources invalidate old
rasters and retained DOM. Code/URL text is not rewritten during copy. Complete native
file metadata, range/media loading, worklets, popups, and all worker bootstrap cases
are outside the current compatibility guarantee.

### Capture policies and reproducibility

Capture waits for load, web fonts, decoded images, and two animation frames. Optional
network-idle, selector, extra-wait, animation, and online/offline settings share the
existing acquisition/render deadline. These are host settings, not extra node fields;
see [browser configuration](configuration.md#browser-shader-and-raster-cache-limits).
They do not guarantee that future requests or application state have settled.

CLI/MCP normally reuse captured rasters but create fresh isolated contexts for new
anonymous acquisitions. `--refresh` / `refresh: true` requests fresh pixels. The TUI
opts into up to eight retained per-node page contexts, refreshing DOM without navigation
where identity remains compatible. Dedicated [authentication profiles](runtimes.md#dedicated-authentication-profiles)
are explicit and separate from personal browser profiles.

Acquisition metadata includes its identity, time, actual browser/renderer, view, limits,
inputs, and image hashes. URLs are redacted. A calculation snapshot and page acquisition
have distinct identities; cached metadata describes its original acquisition, not a
promise that a page process is still alive.

## GLSL fills and effects

A shader references captured GLSL and a flat `uniforms` object. WebGL 1.0 fragment GLSL
supports numeric/boolean scalars, vectors, colors, column-major matrices, flat numeric
arrays, sampler2D images/arrays, and qualified structure leaves. Literal bounds are
checked before launch; expression dimensions also depend on the linked active interface.
Unused tails can be optimized away by the compiler. Unset numeric leaves use zero and
unset samplers use transparent images.

### Declarations and numeric inputs

An immediately preceding documentation comment can bind exact `@time` to float or
`@resolution` to vec2; grouped names share annotations. `@default` supplies ordinary
uniform defaults. Intervening comments, code, or directives break adjacency. Time and
resolution cannot be arrays or overridden in the document. Resolution and
`gl_FragCoord.xy` use local logical dimensions even at enlarged output; z/w retain
WebGL semantics. Render time defaults to zero.

Bounded Wasm preprocessing uses the actual compiler's predefined macros, preserving
selected annotations, original source hashes, and diagnostics. The checked-in component
requires neither Node nor a Wasm toolchain for ordinary application builds. Exact type,
default, structure, and array syntax is described in the shared
[shader guide](../resources/guidance/shaders.md).

Effects can sample `pencil:object` and the previously painted `pencil:background`.
Effect order and parent/backdrop context matter. The shader process explicitly uses
SwiftShader, with Chromium's OS sandbox enabled; it cannot fetch HTTP(S), file, or FTP
inputs directly. Failure to create a context or compile/link/bind a shader is an error,
not permission to drop the effect.

Limits include 1 MiB of source, 262,144 declaration tokens/events, 256 names/leaf patterns,
256 array elements, 4,096 supplied/active numeric components, 16 structure levels,
1,024-byte identifiers, and 4,096-byte qualified names. Preprocessing has a 64 MiB memory
maximum and a 4 MiB expanded-source bound. Sampler inputs total at most 64 MiB per request.

### Image origins

Image fills and samplers share captured static PNG/JPEG/WebP inputs. String variables
retain their original text, aliases, and themes; an image resolves relative to the
document defining the selected value. Copy captures all relevant theme alternatives,
using managed image bindings when path projections conflict or URLs are nonrelative.
Changing the copied value later selects a new consumer input. Local file URLs retain
canonical-path/root checks and reject credentials, remote authorities, queries, and
fragments. Code is never fetched through HTTP(S) image support.

### Embedded images

`data:image/png`, `data:image/jpeg`, and `data:image/webp` accept standard base64 with
a trailing `;base64` marker, or percent-encoded bytes. The declared MIME must match a
static image. Decoded source bytes are limited to 20 MiB. These inputs work offline;
copy/detach preserve the exact URL without creating another managed image file.
Payload contents are omitted from diagnostics.

### HTTP image acquisition

A project captures remote image bytes in memory and reuses them during verification;
a new project capture can fetch new bytes. Browser-profile cookies are not imported.
Policy follows `runtime.browser.network_policy` and its timeout, capped at 30 seconds
across one project's HTTP image acquisition, 20 MiB per encoded image, 4,096-byte URLs,
and five redirects. Embedded credentials, protocol changes, and HTTPS-to-HTTP downgrade
are rejected. A cancellation/owner-bound worker contains network work.

Missing remote images remain explicit incomplete assets for structural repair. Copies
retain acquired bytes locally and remain renderable after source removal. This does not
freeze future remote content or guarantee authenticated-service compatibility.

## Runtime lifetime and cache

JS, page, and shader processes independently expire after 60 seconds without actual
execution by default. Cache hits do not extend their life. Cached pixels and unexpired
calculations can survive cleanup; a later execution or refresh restarts workers.
Library hosts schedule `expire_idle` using the runtime's `idle_deadline`.

Dynamic images are limited to 8,192 pixels per side, 16 megapixels, and 32 MiB PNG.
Browser/shader launch defaults to 15 seconds; acquisitions default to 30/15 seconds and
a shared render to 45 seconds. The default 32-entry/64 MiB LRU covers encoded rasters
and expanded shader-source text, not complete process RSS or decoded/native memory.
Policy changes invalidate acquisitions; zero capacity disables reuse. Native operations
are cooperatively checked, so an individual synchronous call can finish after a deadline.

TUI playback requests up to 5 fps by default (1–10 configurable) and drops obsolete
frames. Page DOM follows browser time; seeking shader time does not rewind a web page
or put document script generation on a clock. Pausing stops new requests while an
already requested frame may finish. See [TUI controls](tui.md).

## Initial compatibility limitations

- Pencil-specific device presets beyond omitted/desktop/phone/tablet return
  `UNSUPPORTED_FEATURE`; basic modes do not emulate a particular named device.
- Shader fills cannot sample `pencil:object` and return `SHADER_COMPILE_FAILED`.
  Shader effects can sample it.

These settings remain stored during unrelated edits. Rendering rejects the unsupported
combination instead of returning a complete-looking partial image. Other graphics,
font, network, and device behavior is limited to the documented implementation;
complete Pencil parity is not claimed.

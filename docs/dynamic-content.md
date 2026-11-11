# Dynamic content

Add generated charts with JavaScript, embed web-page captures with browser nodes,
or create animated fills and effects with GLSL. Start from the bundled examples:

```sh
lazypencil render examples/script-chart.pen --output chart.png
lazypencil runtime setup chromium
lazypencil render examples/dynamic-panel.pen --output dynamic-panel.png
```

Keep `examples/assets` beside the documents. JavaScript generates ordinary design
nodes; browser content and GLSL use the [Chromium runtime](runtimes.md).

## JavaScript

A `script` node supplies `scriptUri`, dimensions, and optional `inputs`. Its source
starts with a header comment such as:

```javascript
/**
 * @schema 2.22
 * @input label: string = "Hello"
 */
return [{type: "text", content: pencil.input.label, fontSize: 16}];
```

Headers accept 2.11, 2.19, 2.21, and 2.22; supported document codecs are 2.19/2.21/2.22.
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

Defaults allow 2 seconds and a 64 MiB JavaScript heap per evaluation, 8 layout
passes, 128 evaluations, and 10 seconds for generated layout. Code is limited to 1 MiB, protocol frames to 4 MiB,
output to 1,000 nodes/64 levels, and the JS stack to 512 KiB. Captured
[JavaScript policy](configuration.md#javascript-execution-limits) controls configurable
limits. If a calculation exceeds one, simplify the script or adjust that setting.

## Script copy and detach

Detachment preserves linked code/input ownership instead of flattening generated
geometry. An independent component copy closes component, variable, theme, and static
asset dependencies without executing or rewriting JavaScript. Use `options.script_assets`
for files selected only inside code, keyed by the qualified source script node.

Copied components preserve the origins of code, inputs and referenced resources.
Edit the owning script or its inputs to change generated content. Internal
`metadata.lazypencil` records keep those relationships; leave them intact when
editing files by hand. If a legacy component lacks the required origin data,
recopy it from its source. See [component import](mcp.md#importing-components).

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
rasters and retained DOM. Code/URL text is not rewritten during copy. Local page capture supports declared files. Pages relying on native file metadata,
range/media loading, worklets, popups or complex worker startup may need adaptation.

### Capture policies and reproducibility

Capture waits for load, web fonts, decoded images, and two animation frames. Optional
network-idle, selector, extra-wait, animation, and online/offline settings share the
existing acquisition/render deadline. These are host settings, not extra node fields;
see [browser configuration](configuration.md#browser-shader-and-raster-cache-limits).
Use an application-specific ready selector for content that appears after load.

CLI/MCP normally reuse captured rasters but create fresh isolated contexts for new
anonymous acquisitions. `--refresh` / `refresh: true` requests fresh pixels. The TUI
opts into up to eight retained per-node page contexts, refreshing DOM without navigation
where identity remains compatible. Dedicated [authentication profiles](runtimes.md#dedicated-authentication-profiles)
are explicit and separate from personal browser profiles.

Capture metadata identifies the browser, view, settings, time and image hashes.
A cached image describes its original capture. Use refresh when you need the
current page state.

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

See the shared [shader guide](../resources/guidance/shaders.md) for declaration,
default, structure and array syntax.

Effects can sample `pencil:object` and the previously painted `pencil:background`.
Effect order and parent/backdrop context matter. The shader process explicitly uses
SwiftShader, with Chromium's OS sandbox enabled; it cannot fetch HTTP(S), file, or FTP
inputs directly. Shader compilation or binding errors identify the failing effect so you can repair it.

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

Ordinary inspection declares HTTP image URLs without requesting them. `get_document`
with `acquire_images: true`, CLI `inspect --acquire-images`, layout/rendering, and image
copy/import explicitly acquire bytes. These operations can contact external services
or local network addresses. Declared URL identities participate in document dependency
preconditions as `remote_image_reference`; acquired byte hashes participate separately
in runtime dependencies and calculation identity.

A project captures remote image bytes in memory and reuses them during verification;
a new project capture can fetch new bytes. Browser-profile cookies are not imported.
Policy follows `runtime.browser.network_policy` and its timeout, capped at 30 seconds
across one project's HTTP image acquisition, 20 MiB per encoded image, 4,096-byte URLs,
and five redirects. Embedded credentials, protocol changes, and HTTPS-to-HTTP downgrade
are rejected. `runtime.browser.image_origins` optionally restricts requests and every redirect to exact
HTTP(S) scheme/host/port origins. Omit it to inherit; `[]` denies all HTTP images. A project
can only narrow a user allowlist. This restriction applies to image acquisition, not
Chromium page requests. `network_policy = "offline"` also denies all HTTP images.

Missing remote images remain explicit incomplete assets for structural repair. Copies
retain acquired bytes locally and remain renderable after source removal. A later capture can fetch changed remote content. For authenticated pages, use a
[browser profile](runtimes.md#dedicated-authentication-profiles).

## Runtime lifetime and cache

JS, page, and shader processes independently expire after 60 seconds without actual
execution by default. Cache hits do not extend their life. Cached pixels and unexpired
calculations can survive cleanup; a later execution or refresh restarts workers.

Dynamic images are limited to 8,192 pixels per side, 16 megapixels, and 32 MiB PNG.
Browser/shader launch defaults to 15 seconds; acquisitions default to 30/15 seconds and
a shared render to 45 seconds. The default 32-entry/64 MiB LRU covers encoded rasters
and expanded shader-source text, not complete process RSS or decoded/native memory.
Policy changes invalidate captures; zero cache capacity disables reuse.

TUI playback requests up to 5 fps by default (1–10 configurable) and drops obsolete
frames. Page DOM follows browser time; seeking shader time does not rewind a web page
or put document script generation on a clock. Pausing stops new requests while an
already requested frame may finish. See [TUI controls](tui.md).

## Compatibility limits

- Pencil-specific device presets beyond omitted/desktop/phone/tablet return
  `UNSUPPORTED_FEATURE`; basic modes do not emulate a particular named device.
- Shader fills cannot sample `pencil:object` and return `SHADER_COMPILE_FAILED`.
  Shader effects can sample it.

Unsupported settings remain stored during unrelated edits. To render the affected
document, replace the unsupported feature or use an editor that supports it. See
[document semantics](pen-semantics.md) for the supported format.

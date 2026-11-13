# Shader inputs, dependencies, and limits

Read [shaders](shaders.md) for source creation, uniform annotations and values, image samplers, and render time. Use this guide for advanced declarations, image dependencies, preprocessing, and runtime policy.

## Structured uniforms

Structure uniforms use qualified leaf names in the flat uniforms object, for example {"lamp.color":"#3a7fc1","lights[1].strength":0.75}. Named, inline, anonymous and nested structure types are supported, including arrays of structures and primitive arrays inside fields. A field such as lights[1].weights is a numeric list when the GLSL field is an array. Use canonical decimal indices, starting at zero. Unset typed fields use zero or a declared field default; unset sampler fields use transparent images. Field annotations belong to their own declaration and are not inherited from a comment on the aggregate structure. Aggregate @default, @time and @resolution are rejected. A structure-array field cannot use an automatic @time or @resolution binding.

Actual linked leaf types are checked before values are applied. Literal structure-array bounds are checked before launch; constant-expression bounds still depend on the active interface reported by the compiler. Inspect the linked interface and rendered result when changing these declarations.

## URL ownership and copied assets

For a variable-valued image URL, lazypencil follows aliases and the active theme, then resolves local images relative to the document defining the selected value. The same variable's text uses keep their original strings. Detachment retains these linked variables and their image origins.

Standalone component copying retains local relative string sampler values in ordinary imported variable libraries with captured images. Absolute local paths, local file:// URL strings, HTTP(S) URLs and conflicting path projections use managed image bindings while preserving their original text. Shared strings, aliases and themes remain editable; no managed image URL is substituted into the text value. Canonical-path, allowed-root and dependency checks still apply to local inputs. Local file URLs cannot contain credentials, a remote host, a query or a fragment; percent-encode reserved filename characters.

## HTTP image acquisition

HTTP(S) images use the captured runtime.browser network policy and timeout, capped at 30 seconds across one project's image acquisition, with 20 MiB per encoded static PNG/JPEG/WebP and at most five redirects. Each snapshot keeps its acquired bytes in memory without refetching during verification. A copied component retains those images locally, including inactive themes, so it can render after the remote server and original files are gone. HTTP image URLs cannot contain credentials, and HTTPS redirects cannot downgrade to HTTP. Browser-profile cookies are not imported. Missing HTTP images report incomplete assets and can be structurally repaired.

Captured script asset manifests accept local paths and local file URLs for images, shaders and scripts; HTTP(S) keys are image-only. Copy preserves URL spelling and source bytes, and repeated declarations of captured URLs use the program's captured code namespace without refetching. Inspect capabilities before choosing an independent copy.

## Embedded images

Embedded data:image/png, data:image/jpeg and data:image/webp URLs work in image paints and string samplers, including copied script output. Use standard base64 with a trailing ;base64 header marker, or percent-encoded image bytes otherwise. The decoded encoded-image payload is limited to 20 MiB and must match its declared static-image MIME type. Offline policy does not disable these self-contained images. Copy and detach preserve the original URL and its text/theme uses without separate image files. Do not use these URLs for scripts, shader code or library imports. Malformed embedded images remain incomplete assets with repair diagnostics; their payload is not included in those diagnostics.

## Capture policy and runtime limits

Use get_document with include_configuration:true to inspect configuration.settings.runtime.shader and runtime.dynamic. Keep runtime preferences in host configuration; source and uniform values belong in the document.

Captured runtime.shader settings control acquisition/launch deadlines and idle expiry; runtime.dynamic controls the shared render budget and cache. Expanded declaration sources and encoded rasters share its byte/entry limits and LRU. Interface acquisition, typed preparation and drawing share one shader deadline; cached reads do not keep the browser alive. Acquisition metadata reports effective limits, and changing policy invalidates old cached rasters. Compilation, linking, invalid uniforms, missing inputs, context loss, and deadlines are errors. Generated pixels do not replace editable source code in the document.

## Preprocessing and resource limits

The original WebGL source is validated before bounded preprocessing using the compiler's predefined macros. Object/function macros and conditional branches select the declarations used for typed inputs. Unknown names and declarations excluded by preprocessing cannot receive overrides. Original source positions and line/file macros are retained; saved source bytes and hashes stay unchanged. Only the adjacent original documentation comment supplies a declaration's annotations, even if an intervening directive disappears during expansion.

Literal dimensions produced by expansion use ordinary bounds checks. Constant expressions are checked against the linked interface. GLSL ES 1.00 rejects direct arrays of arrays and structure definitions declared inside another structure's fields; already-declared types can be nested. Common vec2 UV varying aliases are provided.

| Resource | Limit |
| --- | --- |
| Original GLSL / expanded source | 1 MiB / 4 MiB |
| Declaration tokens / preprocessing events | 262,144 each |
| Uniform names or typed leaf patterns | 256 |
| Array elements / supplied or active numeric components | 256 / 4,096 |
| Structure nesting | 16 levels |
| Identifier / qualified leaf name | 1,024 / 4,096 bytes |
| Dynamic raster | 8,192 pixels per side and 16 megapixels |
| Encoded sampler inputs | 64 MiB per request |
| Preprocessor memory | 64 MiB |
| Distinct GL_ names probed | 256 |

These are lazypencil's supported rules. When exchanging shaders with another renderer, verify that renderer's syntax, annotations, coordinates, and output against the same source and inputs.

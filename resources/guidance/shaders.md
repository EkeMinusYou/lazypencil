# GLSL fills and effects

Read an existing shader asset before editing it. Create new source with put_asset using kind: shader, then update the fill or effect URL through an atomic editing batch. Source uses WebGL 1.0 fragment GLSL, usually with an explicit precision declaration. It is compiled by software WebGL in a dedicated Chromium process and composited by Skia.

## Automatic time, resolution, and coordinates

Use documentation-style block comments (/** ... */) for uniform metadata. The nearest documentation comment (/** ... */) immediately before a uniform can bind the exact directive @resolution to a vec2, or @time to a float with the explicit render time, whose default is zero. Blank lines are allowed. Another comment, ordinary code, or a preprocessor directive between that comment and the declaration clears its metadata. An annotation after a semicolon belongs to the next declaration, including on the same line; it never modifies the preceding declaration. Annotations inside declarations do not bind. A grouped declaration shares its annotations across all declared names. Names such as @time_step do not bind built-in values.

Built-ins must not be arrays or be overridden in the document. @time cannot be combined with @default.

gl_FragCoord.xy uses local logical coordinates for shader fills even at enlarged output; its z/w components keep WebGL semantics. @resolution uses the logical shader extent, not the exported pixel dimensions. Effects use the logical extent of their composited image.

## Uniform values and defaults

Store uniform overrides by name in uniforms: numeric float or int, boolean, numeric vectors, hexadecimal colors for vec3 or vec4, matrices, or image inputs for sampler2D. ivec2/3/4 use integers; bvec2/3/4 use numeric 0 or 1. mat2/3/4 use 4/9/16 column-major numbers. Numeric arrays use flat numeric lists: vec3 colors[2] requires six numbers. Boolean arrays use numeric 0 or 1.

Literal dimensions are checked before browser startup; constant-expression dimensions are compiled by WebGL and their active elements are checked at binding. An optimized unused tail is ignored by the linker. Use complete declared values when the full dimension is known. Grouped declarations such as uniform float first, second[3]; are accepted.

A documentation comment before a uniform can set @default, for example /** @default 0.75 */ before a float declaration. Use comma-separated components for vectors and matrices; @color with vec3 or vec4 also accepts a hexadecimal color default. Grouped names share the default.

Stored values, including zero and false, take precedence; built-in resolution and time remain automatic. A numeric array default initializes its first element, with zero for remaining linked elements; this does not relax validation of explicit array overrides. Prefer decimal numbers: native decimal-prefix parsing treats 0x2 as zero. Missing numeric defaults use zero, and missing sampler inputs get a transparent image. Rendering does not rewrite metadata or saved uniform values.

## Image samplers

Image sampler URLs resolve through captured document assets. A sampler2D array uses a list of image URLs, for example {"images":["front.png","back.png"]}. Each list element may reference a string variable, whose aliases and theme retain their own source document. Literal dimensions require exactly that many values; expression dimensions require at least the linked active prefix. The runtime binds each active element to a texture unit, reuses identical image inputs, and checks its actual fragment texture-unit limit.

Lists also accept pencil:object and pencil:background when that render stage provides them. An effect can bind pencil:object to the already rendered node. pencil:background captures the scene before that node, including relevant parent layers. Effect order matters: each shader receives the result of previous effects. Fill-stage pencil:object is currently unavailable. Internal texture orientation is handled by the runtime; do not compensate for it by flipping ordinary input images again.

## Render time

Use render time to evaluate animation at a reproducible instant. Request a new time to change it. The result includes the actual browser and software renderer, enabled extensions, request hash, and PNG hash.

## Advanced inputs and runtime policy

Read [shader-reference](shader-reference.md) through get_guidance for structured uniforms, macro preprocessing, resource limits, image URL ownership and acquisition, copied assets, or runtime configuration.

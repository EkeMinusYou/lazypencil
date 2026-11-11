# Graphics and PNG export

Use `lazypencil render` to export a design, a selected node's area, or a region as
PNG. Shapes, text, icons, gradients, borders, shadows and background blur render
locally. Browser nodes and GLSL use the optional [Chromium runtime](runtimes.md).

## Examples

From a repository checkout, try:

```sh
lazypencil render examples/graphics-study.pen --output graphics-study.png
lazypencil render examples/backdrop-panel.pen --output backdrop-panel.png
lazypencil render examples/backdrop-panel.pen --region 64 156 872 424 --output backdrop-crop.png
```

| Example | What it shows |
| --- | --- |
| [Graphics study](../examples/graphics-study.pen) | Text decorations and outlines, borders, inner shadows, and image crop modes |
| [Backdrop panel](../examples/backdrop-panel.pen) | A translucent panel with background blur, text, a border and a shadow |
| [Shadow blends](../examples/shadow-blends.pen) | The eighteen blend modes applied to inner and outer shadows |
| [Mesh study](../examples/mesh-study.pen) | Mesh gradients with moved grid points and explicit handles |

Keep `examples/assets` beside the graphics study: its reference image and
[editable image source](../examples/assets/crop-source.pen) are included there.
See the [gallery](../examples/README.md) for complete screens and editing prompts.

## Rounded polygons

Set a polygon's `cornerRadius` to round its vertices. The radius must be a finite,
nonnegative number and can come from a variable, theme or instance override.
Zero keeps sharp corners. Large radii are capped at half the shortest edge.
The rounded shape is used consistently for fills, strokes, masks and effects.

## Text and outlines

A text node contains one string. Use separate text nodes when you need independently
styled spans. Set `underline` or `strikethrough` to `true` to decorate the text;
decorations follow wrapping and alignment and use the text's fills.

A text stroke follows the letters and their decorations, rather than the text box.
It supports inner, center and outer alignment and can use color, gradient, image
or shader paints. The line height controls layout; glyphs and decorations that
extend outside the line box still contribute to the exported area.

Color and bitmap fonts use the painted glyph silhouette for outlines. These
outlines approximate the rasterized shape, so their edges can differ from a
font's original vector curves. See [fonts](fonts.md) for family selection,
missing-character handling and reproducible font inputs.

## Borders and export bounds

Strokes support cap, join, miter and alignment settings. Rectangles, frames and
browser nodes also support independent top, right, bottom and left widths.
For other shapes, the `top` width supplies a uniform stroke; omitting it gives
zero width. Widths must be finite and nonnegative.

Strokes contribute to the exported bounds, including long miters at sharp corners.
Disabled or zero-width strokes add no area. Text boxes and other conservative
bounds can leave transparent space around the painted content.

[Icons](icons.md) support the same native fills, strokes and effects as shapes.
Image and shader paints inherited from a component resolve relative to the
component that supplied them.

## Image crop transforms

An image fill's six-number `transform` maps normalized source-image coordinates
into the normalized crop box. The default is the identity transform. For example,
`[2, 0, 0, 1, -1, 0]` selects the image's right half. Rotation and shear are supported.

| Mode | Result |
| --- | --- |
| `cover` | Fill the node while preserving the crop's aspect ratio |
| `contain` | Fit the crop inside the node, leaving space where necessary |
| `stretch` | Fit the crop to the node's dimensions |

Use these exact mode values in `.pen` files. When replacing a fill through an
edit operation, supply the complete paint object, including its `type`;
`update.set.fill` replaces the previous fill rather than merging individual fields.

Image sampling uses linear interpolation, including near crop boundaries.
A crop changes how the original image is displayed; it does not create a new
image file. Invalid or singular transforms produce an error.

## Shadow and paint blending

Inner shadows stay inside the object's painted alpha and do not expand export
bounds. Outer shadows extend beyond the object. Both support the eighteen blend
modes from the [`.pen` schema](https://docs.pencil.dev/for-developers/the-pen-format#typescript-schema).
Effects run in their stored array order, and parent opacity applies to the combined
content and effects.

Standard modes follow the [W3C compositing model](https://www.w3.org/TR/compositing-1/#blending).
`linearBurn` clamps the sum of the straight colors minus one; `linearDodge` clamps
their sum. Both use source-over alpha. Mode names retain their schema spelling,
including `light`. Disabled effects are skipped.

Run layout validation to find structural problems, then render the document to
check its paints and effects. These are separate checks: a valid structure can
still reference a missing image or contain an effect that cannot render.

## Mesh gradients

A `mesh_gradient` uses a row-major grid of colors. `columns` and `rows` default to
two and must be whole numbers from 2 to 64. Supply exactly `columns * rows` colors.
Omitted `points` form a regular grid. Explicit points must have the same count,
with normalized positions between 0 and 1.

A point can be a two-number position or an object with `position` and optional
left, right, top and bottom Bézier handles. Handles are finite two-number relative
offsets. An explicit handle replaces only that direction; omitted directions use
automatic tangents. Interior tangents use the next point minus the previous point,
divided by six; endpoints use the one-sided difference divided by three.

Cells follow their four boundary curves, with bilinear color interpolation in
premultiplied sRGB. Supply dimensions and handles explicitly when exchanging
precise meshes between editors. Folded meshes are drawn in row-major cell/triangle
order, so later overlapping triangles replace earlier ones.

Mesh fills also work on text, icons and borders. Excessive mesh curvature or
complexity produces `RESOURCE_LIMIT`; simplify the grid or handles if this occurs.

## Background blur and compositing

`background_blur` blurs the scene behind a node while keeping its foreground
content sharp. A later blur or shader can change that result, so effect order
matters. The blur radius maps to Gaussian sigma `radius / 2`; an omitted radius
is zero. Negative and nonfinite radii are rejected.

Shapes use their outlines as blur masks, text uses its glyphs, and groups use
their painted contents. Rotation, rounded corners, clipping and parent opacity
apply to the final composition. Background blur renders locally without Chromium.

Node and region exports are crops of the composed document. `--node` selects the
node's visual bounds and includes the surrounding or overlapping scene in that
area. To choose coordinates directly, use `--region X Y WIDTH HEIGHT`; negative
coordinates are allowed.

## Large exports

Working rasters are limited to 16,384 pixels per side and 32 megapixels. Filters
need extra pixels around the requested region, and meshes or intermediate layers
may require their full bounds even for a small crop. If a render exceeds its
limits, reduce the export scale, the effect radius or the design's complexity.

For supported properties and interchange limits, see [document semantics](pen-semantics.md).
Renderer contributors should start with [architecture](design.md) and the
[render fixture rules](development.md#test-fixtures-and-documentation).

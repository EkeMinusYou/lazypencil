# Native paints, effects, and outlines

Use effects to support hierarchy and separation. Keep labels and primary actions readable; avoid using blur as a substitute for contrast, spacing, or a clear surface color.

## Blur and shadows

Native effects include blur, outer and inner shadows with color blend modes, and background_blur. A background blur filters the scene behind the node while leaving its foreground content sharp. These native effects run on Skia CPU and do not need Chromium. Set effect: { type: background_blur, radius: 12 } and inspect the actual result over representative content. Transparent surfaces can add a restrained color tint; include a boundary where needed.

Shadows accept the same named color modes as paints. Use blendMode: multiply for a shadow that darkens the actual scene behind the node, or normal for ordinary source-over color. Inner shadows blend with the object's color and retain its alpha. Linear burn and linear dodge blend color before source-over composition; they do not add alpha channels. Query the advertised blend_modes for exact serialized names, including light rather than a guessed alias. Inspect the effect over opaque and translucent surfaces. Variables and instance themes can select blend modes without flattening their definitions.

## Order, transparency, and scene context

Effects execute in their declared array order. A foreground blur after a backdrop blur also blurs that intermediate result. A later shader can post-process the captured backdrop as part of the node. Changing the order can therefore change both colors and transparency. Do not reorder effects as an incidental cleanup.

Parent opacity applies after the parent's content and effects. Backdrop filtering retains the sampled transparency; it should not repeatedly increase alpha. Inspect rounded corners, clipped parents, overlapping translucent surfaces, and shadows when changing compositing.

Node previews are crops of the composited document using the node's visual bounds. They retain surrounding and overlapping scene context. A region preview has the same semantics, and background sampling can include pixels outside the requested output rectangle. Neither preview is an isolated transparent export of just one stored node.

## Mesh gradients

Mesh gradients are native paints for decorative surfaces, text, and strokes. Use type: mesh_gradient, columns and rows, and one color per vertex in row-major order. Omitted dimensions use a 2 by 2 grid; specify them explicitly for portable designs. A four-color example is { type: mesh_gradient, columns: 2, rows: 2, colors: ["#F59E9B", "#A8B4F8", "#FAD9A1", "#93D8CE"] }. Omitted points form a regular grid. Set points with normalized [x,y] positions within 0..1; keep boundary points at their defaults when the surface should cover the full box.

A point object can use position plus leftHandle, rightHandle, topHandle, and bottomHandle, each a relative [dx,dy] offset in the same normalized box. Omitted handles use neighboring points to generate smooth geometry. Set colors through existing variables and themes when available. Check actual text contrast across the complete surface. Inspect examples/mesh-study.pen for editable uses.

Mesh colors interpolate with alpha, and the paint supports opacity and the advertised blend modes. The grid supports 2..64 vertices along each axis; extreme curvature, scale, or density can exceed rendering limits. Reduce these when a resource diagnostic requests it. Specify dimensions, positions, and handles explicitly when exchanging precise artwork with another renderer.

## Filter resource limits

Filter support can require a larger working raster than the output image. Oversized support or scratch reservations fail explicitly; reduce region, scale, or effect radius when appropriate. No effect is silently dropped. Inspect diagnostics and the advertised capabilities when an effect cannot be rendered.

## Polygon corners and outlines

Polygons support a scalar cornerRadius in addition to polygonCount. Use a moderate radius for rounded decorative shapes and inspect the rendered result, including any stroke or shadow. The path uses circular fillets and caps the radius at half its shortest edge. Large fillets can cross on acute polygons, so increasing the radius does not always produce a smoother silhouette. Preserve an existing design's declared radius unless changing its shape is intentional.

Use a numeric strokeWidth for a uniform outline. Rectangle, frame and browser nodes also support independent top/right/bottom/left widths. Other shapes, including text and paths, use the top entry as a uniform width; an omitted top paints no stroke. Keep existing side declarations when preserving a design, and inspect get_layout and a preview after changing an outline because it can change the visual export bounds.

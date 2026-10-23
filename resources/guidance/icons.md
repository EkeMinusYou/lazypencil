# Vector icons

Call `list_icons` to discover names, aliases, versions, licenses, and installation status. Lucide is bundled and works offline. Feather, Phosphor Regular, and Material Symbols Outlined/Rounded/Sharp require the user's explicit setup. For example: `lazypencil icons setup phosphor` or `lazypencil icons setup 'Material Symbols Outlined'`. Rendering never downloads a missing pack. Check the exact library ID and installed status before promising an icon is available.

Use the serialized `.pen` 2.21 icon node:

```json
{"type":"icon","library":"lucide","icon":"search","width":24,"height":24,"fill":"#0F172A"}
```

Insert it through the normal editing batch and use the returned ID. `library`, `icon`, and `fill` can bind to document variables. Reuse icon-bearing components instead of duplicating unrelated shapes. Keep icon size, spacing, color roles, and visual density consistent with nearby text and controls. Use real labels where a symbol alone would be ambiguous.

Lucide, Feather, and Phosphor Regular have fixed outlines. Omit `weight`; do not synthesize a heavier glyph or map a number to a named Phosphor style. Phosphor includes original canonical names, PascalCase names and deprecated aliases; use discovery rather than inventing suffixes.

The three Material Symbols libraries use pinned original variable fonts. Their `weight` accepts numbers from 100 through 700, including fractional values, and defaults to 400. It can reference a numeric document variable, including themed values. For example: `{"type":"icon","library":"Material Symbols Rounded","icon":"settings","weight":350.5,"width":24,"height":24,"fill":"#0F766E"}`. The remaining font axes are fixed to `FILL=0`, `GRAD=0`, and `opsz=24`; the icon node's `fill` is its paint, not the font's FILL axis. Do not invent additional serialized axis properties. Calculation metadata reports the actual axes and pinned font hash.

Vectors are centered and fit inside the declared width and height with their aspect ratio preserved. A node's `fill` paints the icon ink, including gradient and image paints; transparent holes remain transparent. Opacity, transforms, and effects use the same rendering pipeline as other shapes. Terminal fonts do not supply these icons, and the managed icon fonts do not require system installation.

Keep a stored alias when editing an existing document unless the user requests a rename. Use a catalogued canonical name for a new design. Missing names, missing packs, modified installed data, and unsupported weights produce diagnostics. Do not replace them silently with a different icon or a placeholder. Check computed layout and rendered PNGs before reporting completion.

# Vector icons

The renderer supports `.pen` 2.21 `type: "icon"` nodes using pinned SVG geometry or original variable-font outlines. Icon rendering does not require a browser, AI service, terminal font, or system installation of an icon font.

## Packs and installation

| Library | Pinned source | Available names | Distribution | License |
| --- | --- | --- | --- | --- |
| `lucide` | `lucide-static` 1.57.0 | 1,890 canonical icons and 264 aliases | Compiled into the executable | ISC and MIT |
| `feather` | `feather-icons` 4.29.2 | 287 icons | Explicit installation | MIT |
| `phosphor` | `@phosphor-icons/core` 2.1.1, regular style | 1,512 canonical names and 1,548 original PascalCase/deprecated aliases | Explicit installation | MIT |
| `Material Symbols Outlined` | Google commit `10ecc14790612ca8fd164e3f4ffe9c16fc50d5b7` | 4,304 names | Explicit original TTF download | Apache-2.0 |
| `Material Symbols Rounded` | Same Google commit | 4,304 names | Explicit original TTF download | Apache-2.0 |
| `Material Symbols Sharp` | Same Google commit | 4,304 names | Explicit original TTF download | Apache-2.0 |

Lucide works offline immediately. Optional packs use the persistent data directory selected by `--data-dir` or `LAZYPENCIL_DATA_DIR`. Setup and rendering must use the same directory.

```sh
lazypencil icons list --pack lucide --name settings
lazypencil icons setup feather
lazypencil icons list --pack feather --name check
lazypencil icons license lucide
lazypencil icons license feather
lazypencil icons setup phosphor
lazypencil icons setup 'Material Symbols Outlined'
lazypencil icons setup 'Material Symbols Rounded'
lazypencil icons setup 'Material Symbols Sharp'
```

`icons license` returns the complete embedded upstream license, hash, source, and version without installing anything. Use `--json` for machine-readable output. Lucide's notice includes the MIT terms for its Feather-derived icons; its ISC portion alone is insufficient.

For offline Feather installation, obtain the exact archive named in [the catalogue](../resources/icons/catalogue.json):

```sh
lazypencil icons setup feather --archive /path/to/feather-icons-4.29.2.tgz
```

Phosphor similarly accepts its exact `phosphor-core-2.1.1.tgz` archive. Material packs download one original TTF each; their codepoint indices and complete license are embedded metadata. For offline setup, supply the exact pinned TTF with `--input /path/to/MaterialSymbolsOutlined[FILL,GRAD,opsz,wght].ttf`. `--input` is an alias of the existing `--archive` option; no archive extraction is performed on a font. Each installed font retains its original bytes and copyright/name tables.

Setup verifies the source SHA-256, converts only the expected geometry/index, checks normalized data and complete license hashes, and publishes a staged version directory under a per-pack lock. It reads archive members without extracting paths or executing package scripts. Concurrent installations converge on the same validated contents. Existing valid contents are reused; modified contents are rejected rather than overwritten. HTTPS retrieval has a 60 second global timeout. Cancellation is checked around retrieval and publication; a blocking HTTP call can take until its timeout to return.

Rendering and queries never install a missing pack. Missing optional data produces `ASSET_NOT_FOUND` with a correctly quoted setup command; corrupt installed data produces `ICON_INTEGRITY`. Unsupported libraries and weights produce explicit errors, and the stored document is preserved.

## Creating an icon

```json
{
  "id": "settings-icon",
  "type": "icon",
  "library": "lucide",
  "icon": "settings",
  "width": 24,
  "height": 24,
  "fill": "#2563EB"
}
```

`library` defaults to `lucide`. Names and paints are resolved after variables, themes, component inheritance, and overrides. Aliases such as Lucide `home` resolve to a canonical name (`house`); calculation metadata retains both names.

Lucide and Feather use round two-unit strokes in a 24×24 view box. The renderer converts strokes to filled geometry and unions overlapping coverage. Phosphor uses its original filled regular paths in a 256×256 view box. Material uses captured original fonts and codepoints, with its 960-unit design box. Its paths are extracted without font hinting and use the same native vector renderer. Holes remain transparent, and translucent intersections do not accumulate opacity. Non-square bounds fit and center the icon while preserving its aspect ratio. Native paints, gradients, transforms, and effects use the same geometry as ordinary shapes.

Omit `weight` for Lucide, Feather and Phosphor Regular; they do not provide continuous font weights. Phosphor's other named upstream styles are not substituted for regular and are not invented as numeric weight mappings.

Material `weight` supports the original continuous range 100..700, including fractions, with default 400. It can bind to a numeric document variable and participate in inherited component themes and overrides. `FILL=0`, `GRAD=0`, and `opsz=24` are fixed design defaults; `.pen` icon `fill` is the native paint, not the font's FILL axis. The font's actual axes/units are checked when first instantiated, and requested positions must be applied by Skia. No synthetic bold is used. Calculation metadata includes the resolved weight, axis positions and font hash. Explicit `null`, strings that are not numeric variables, non-finite/out-of-range values, and weights on a fixed pack are rejected.

The [editable icon study](../examples/icon-study.pen) demonstrates Japanese labels, gradients, non-square bounds, and translucent geometry with a shadow:

```sh
lazypencil render examples/icon-study.pen --output icon-study.png
```

After installing the five optional packs above, render [the icon families study](../examples/icon-families.pen) to compare all six libraries and three real Material weights in one editable design:

```sh
lazypencil render examples/icon-families.pen --output icon-families.png
```

## MCP and snapshots

`list_icons` is available with or without rendering. Arguments are `pack`, `name`, `limit` (1–500, default 100), and `cursor`. Search is case-insensitive and also searches available upstream tags. Results include aliases, weights, versions, hashes, licenses, and pack installation diagnostics. Cursors bind to the pack version and query. The English `icons` guide explains the format and workflow.

Successful layout records include resolved icon metadata. Discovery reports per-pack `variable_weight`, `weight_range`, and `default_weight`, rather than synthesizing a discrete list for a continuous font. Unavailable packs, names, and weights produce layout errors; rendering fails instead of substituting an icon or returning a placeholder. Installed manifests, geometry/index, licenses and original fonts become captured dependencies. Changes invalidate the snapshot. Packs first used by script-generated nodes become runtime dependencies. Installing a previously missing pack also invalidates the failed snapshot; a TUI refresh reloads the project.

Every load revalidates the original captured file bytes before sharing a pack. Live snapshots can share immutable geometry and font bytes, as well as lazy native typefaces; a modified file is still rejected before cache reuse. Native Material instances have an eight-position LRU per live pack. The cache uses weak pack references, so it does not retain every previously opened font after its snapshots are released.

Each snapshot also retains the requested installation paths and their resolved file targets independently of the shared pack. Retargeting an individual file or an ancestor directory invalidates the snapshot, even if the new target contains identical pinned bytes. Recalculation captures the new targets and dependency revision; returning a link to its original target restores the old snapshot's path condition. These checks also cover packs first loaded by generated content.

When a live pack already owns the original bytes, validation hashes the current geometry/font file through a 64 KiB buffer before reusing them. Project dependency checks use the same bounded streaming hash for all captured binary assets, with cancellation checks between reads. This avoids allocating another full font for each validation while retaining SHA-256 comparison, regular-file checks and input size limits. Native typefaces continue to own their Skia data; they do not borrow a snapshot's Rust byte slice.

## Provenance and reproduction

[The catalogue](../resources/icons/catalogue.json) pins upstream URLs, source SHA-256/SHA-512 integrity values, normalized geometry and license hashes, counts, and versions. npm archives retain their registry-published SHA-512 integrity; hashes for the immutable Google files are computed from the acquired originals, not presented as publisher-supplied checksums or signatures. The [resource notice](../resources/icons/README.md) describes redistribution; the [fixture notice](../tests/fixtures/icons/README.md) identifies the offline archives.

With both exact archives in an existing directory, run the Python standard-library-only reproducer:

```sh
python3 scripts/vendor_icons.py /path/to/archives --output /tmp/lazypencil-icon-resources
```

It verifies the archives, checks every canonical Lucide primitive against the original SVG, resolves aliases by matching geometry, and writes the catalogue and notices. Lucide geometry goes under the output directory; normalized Feather data goes beside the input archives for comparison with the Rust installer. It makes no network requests and executes no package scripts. Compare regenerated resources with `resources/icons` before accepting an update. Updating pins requires reviewing licenses and rendering evidence.

For Phosphor and Material, [vendor_optional_icons.py](../scripts/vendor_optional_icons.py) accepts a directory containing the original Phosphor archive, all three pinned TTF and `.codepoints` files, and `Material-LICENSE`. It verifies their pinned hashes before deriving indices and normalized-data hashes. Run `python3 scripts/vendor_optional_icons.py SOURCES --output resources/icons` after reproducing the existing base catalogue. Per-pack `provenance.json` records source paths/commits, hashes, copyright, and normalization; the original Material codepoint/license bytes are preserved. This developer reproducer performs no network requests and never evaluates package JavaScript.

## Verification and remaining work

`tests/icons.rs` covers pagination, aliases, no implicit installation, pinned licenses, CLI discovery/license/offline setup, concurrent installation, tamper detection, snapshot invalidation, all 2,441 advertised names' native geometry, native paint pixels, and script-generated Feather dependencies. `tests/mcp.rs` exercises variable and component resolution through a real MCP server and verifies PNG pixels.

An opt-in test compares representative native icons with original Lucide SVGs rendered by Chromium:

```sh
LAZYPENCIL_TEST_CHROMIUM=/path/to/chrome-headless-shell \
LAZYPENCIL_TEST_LUCIDE_ARCHIVE=/path/to/lucide-static-1.57.0.tgz \
cargo test --test icons -- --ignored
```

Additional opt-in checks use `LAZYPENCIL_TEST_MATERIAL_DIR` containing the original source TTFs and `.codepoints` files:

```sh
cargo test --test optional_icons official_material_fonts -- --ignored --nocapture
cargo test --test mcp real_mcp_material -- --ignored --nocapture
# Also set LAZYPENCIL_TEST_CHROMIUM for the independently rendered source comparison.
cargo test --test optional_icons optional_icon_paths_match -- --ignored --nocapture
```

The source comparison uses seven original regular Phosphor SVGs and all three Material fonts at weights 100, 350.5 and 700, served only from a temporary loopback server. SVG text uses geometric precision to compare unhinted outlines, with explicit font axes and no font synthesis. Native extraction also disables font hinting. The same mean absolute channel-difference bound of 2/255 applies to every case; successful macOS arm64 runs measured below 0.5/255 for Material and below 0.01/255 for Phosphor. Material tests separately resolve every one of the 12,912 advertised names, compare changing real weights, exercise cache eviction, themes/components, and reject modified font files. A real MCP check edits the numeric weight, receives actual PNG content, and rejects stale calculations after document/font changes.

Hosted CI is configured to run both native Material opt-ins on all five target environments. [The source preparer](../scripts/ci/prepare_material_fonts.py) uses the catalogue's exact Google commit, URL and SHA-256, streams each original TTF through a 64 KiB buffer with a 16 MiB limit, and publishes it only after checksum verification. Cached originals are revalidated. Bad downloads, redirects outside the expected HTTPS host, mutable pins and oversized data fail explicitly; temporary downloads are removed. Fonts are stored only in the hosted runner's temporary directory. The native tests need the three original TTFs; the separate Chromium comparison additionally needs its source indices and browser. Per-platform logs distinguish these opt-ins from the normal test configurations, and configuration alone is not proof that they have passed on a platform.

The Lucide comparison and explicit HTTPS Feather installation have passed on macOS arm64, as have real Codex/Claude subscription workflows that discover icons, create native icon nodes, and return PNG previews. These checks do not establish full Pencil compatibility, product performance, or other-platform operation. Broader catalogue configuration, platform compatibility, and release packaging remain required work.

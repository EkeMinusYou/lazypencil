# Icons

Use an `icon` node to add a vector icon to a design. Lucide is bundled; install
another pack when a document uses it. Icons support the same fills, transforms
and effects as other shapes.

## Creating an icon

```json
{
  "id":"settings-icon",
  "type":"icon",
  "library":"lucide",
  "icon":"settings",
  "width":24,
  "height":24,
  "fill":"#2563EB"
}
```

`library` defaults to `lucide`. Find names with:

```sh
lazypencil icons list --pack lucide --name settings
```

Names and paints can use variables, themes and component overrides. Recognized
aliases resolve to canonical names; for example Lucide `home` resolves to `house`.
Non-square bounds center the icon while preserving its aspect ratio.

## Packs and installation

| Library | Installation | Style | License |
| --- | --- | --- | --- |
| `lucide` | Bundled | Outline | ISC and MIT |
| `feather` | `lazypencil icons setup feather` | Outline | MIT |
| `phosphor` | `lazypencil icons setup phosphor` | Regular | MIT |
| `Material Symbols Outlined` | `lazypencil icons setup 'Material Symbols Outlined'` | Variable weight | Apache-2.0 |
| `Material Symbols Rounded` | `lazypencil icons setup 'Material Symbols Rounded'` | Variable weight | Apache-2.0 |
| `Material Symbols Sharp` | `lazypencil icons setup 'Material Symbols Sharp'` | Variable weight | Apache-2.0 |

For example:

```sh
lazypencil icons setup feather
lazypencil icons list --pack feather --name check
```

Packs are installed in the selected data directory. Use the same `--data-dir` or
`LAZYPENCIL_DATA_DIR` for installation and rendering. Setup reuses valid contents
and reports modified files rather than replacing them.

| Error | What to do |
| --- | --- |
| `ASSET_NOT_FOUND` for a pack | Run the setup command in the diagnostic |
| Unknown icon name | Search the pack's available names with `icons list` |
| Invalid weight | Use a supported Material weight or omit weight for a fixed pack |
| `ICON_INTEGRITY` | Inspect the reported pack installation before repairing its files |

### Offline installation

Use the exact archive or font from the [pack catalogue](../resources/icons/catalogue.json):

```sh
lazypencil icons setup feather --archive /path/to/feather-icons-4.29.2.tgz
lazypencil icons setup phosphor --archive /path/to/phosphor-core-2.1.1.tgz
```

Material packs accept their pinned original TTF with `--input`, an alias of
`--archive`. Setup checks the expected source hashes in either case.

## Weights and appearance

Omit `weight` for Lucide, Feather and Phosphor Regular. These packs provide fixed
geometry; other Phosphor styles are separate from the supported regular pack.

Material `weight` accepts 100–700, including fractions, with default 400. A numeric
variable can supply it. Other axes are fixed at `FILL=0`, `GRAD=0` and `opsz=24`.
The node's `fill` specifies paint, separately from the font's FILL axis.

## Examples

From a repository checkout:

```sh
lazypencil render examples/icon-study.pen --output icon-study.png
```

The [icon study](../examples/icon-study.pen) includes Japanese labels, gradients,
non-square bounds and translucent icons with shadows.

After installing the five optional packs, compare all six libraries and several
Material weights with the [icon families study](../examples/icon-families.pen):

```sh
lazypencil render examples/icon-families.pen --output icon-families.png
```

## MCP and snapshots

`list_icons` accepts `pack`, `name`, `limit` (1–500, default 100) and `cursor`.
Case-insensitive search includes available names, aliases and tags. Results include
installation status, weights, versions, hashes and licenses. Repeat the query with
its cursor to page the same catalogue.

Layout records report the resolved icon and weight. Packs expose
`variable_weight`, `weight_range` and `default_weight` for continuous weights.
Changed installed data invalidates old calculations. After installing a missing
pack, refresh the TUI or obtain a fresh MCP calculation.

## Provenance and reproduction

`lazypencil icons license PACK` prints the complete notice and source information.
Retain each pack's original terms when redistributing it. Lucide's notice includes
MIT terms for its Feather-derived icons as well as ISC terms. See the
[resource notice](../resources/icons/README.md).

The [catalogue](../resources/icons/catalogue.json) pins versions, original download
locations, source hashes and normalized-data hashes. Contributors updating packs
can reproduce the resources from those exact originals:

```sh
cargo xtask icons base /path/to/archives --output /tmp/lazypencil-icon-resources
cargo xtask icons optional /path/to/optional-sources --output /tmp/lazypencil-icon-resources
```

Base inputs are the pinned Lucide and Feather archives. Optional inputs include
the Phosphor archive, three Material TTF and `.codepoints` files, and
`Material-LICENSE`. Compare the generated catalogue, geometry and notices before
updating checked-in resources. Source requirements are maintained in
[icons.rs](../xtask/src/icons.rs).

## Development checks

Use `cargo xtask test --test icons --test optional_icons` for catalogue,
installation and rendering changes. Upstream comparisons are opt-in and require
the pinned originals through `LAZYPENCIL_TEST_LUCIDE_ARCHIVE` or
`LAZYPENCIL_TEST_MATERIAL_DIR`, and Chromium where the selected test uses it.
See [development](development.md#focused-development-checks) and
[fixture provenance](../tests/fixtures/icons/README.md).

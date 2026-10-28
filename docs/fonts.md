# Font files, faces, and fallback

The current SkParagraph path uses the same shaped Paragraph for text measurements and PNG painting. Noto Sans JP is bundled at the revision and hashes in [the manifest](../resources/fonts/manifest.json), under the complete [OFL 1.1 notice](../resources/fonts/OFL.txt). Other fonts can be supplied as files without installing them in the OS.

Paragraph glyphs use `FontHinting::None` for both measurement and painting. Skia's glyph-path extraction uses unhinted geometry; the shared policy keeps inner outlines within the corresponding glyph ink on FreeType as well as CoreText. Text metrics expose `hinting: "none"` as rendering metadata, not a saved `.pen` property. Raster antialiasing and native backend differences still require platform-specific comparison; this setting does not promise byte-identical PNGs across operating systems. Browser-node fonts retain the page's Chromium/CSS behavior.

## Adding and inspecting fonts

```sh
lazypencil fonts list
lazypencil fonts list --font /path/to/family.ttc --font /path/to/other.otf
lazypencil fonts list --file design.pen
lazypencil fonts license
lazypencil render design.pen --font /path/to/family.ttc --output preview.png
lazypencil mcp --file /absolute/path/design.pen --font /path/to/family.ttc
lazypencil tui design.pen --font /path/to/family.ttc
```

The `--font` option is repeatable on the existing document commands and AI/TUI entry points. Use the same files and order when computing, editing from measurements, and rendering. [User and project configuration](configuration.md) persists files, search directories, a default family, fallbacks, and locale. `config init --user` creates a shared `config.toml`; project `lazypencil.toml` overrides each explicitly supplied field. Arrays replace, and an empty array stops inheritance. Paths stay relative to the physical configuration file supplying that field. CLI, MCP, and TUI use the same captured inputs. Explicit CLI files precede project files/search results, then inherited user files/search results. `fonts list` without `--file` uses user settings only. Normal rendering does not download fonts, install them into the OS, or copy system fonts into the project. Fully lazy font search remains unfinished.

`fonts list` and MCP `list_fonts` report each supplied face, including all faces in a collection, plus the bundled default unless explicitly overridden. They include family, PostScript name, weight, width, slant, source hash, original collection index, normalized face hash, variation axes/ranges/current values, and synthetic-style flags. This is an inventory of supplied fonts; it is not a catalogue of every installed system font.

`fonts license` reads the complete embedded Noto Sans JP license and pinned source manifest without scanning system fonts or creating a data directory. Supplied fonts retain their own licenses; lazypencil does not infer redistribution permission from a filename or family name.

## Formats and immutable data

Supported containers are TrueType `.ttf`, CFF `.otf`, and OpenType collections `.ttc` / `.otc`. Container contents determine parsing; changing an extension does not create a new format. Collections can contain multiple families/styles and different outline formats, as defined in [the OpenType specification](https://learn.microsoft.com/en-us/typography/opentype/spec/otff#font-collections).

The current macOS Skia CoreText path does not accept nonzero collection indexes when making a typeface from bytes. Lazypencil therefore rebuilds each selected collection face as an in-memory standalone SFNT. It validates directory/table ranges, relocates table offsets, recomputes checksums and the head adjustment, and removes a signature that described the original container. Glyph and shaping table bodies remain unchanged. Source files are never modified. Standalone inputs borrow their captured bytes without an extra normalization buffer.

`hash` identifies the original source bytes, and `face_index` identifies its original collection entry. `face_hash` identifies the normalized standalone data supplied to Skia. The original collection remains a captured file dependency, so even a change to trailing source bytes invalidates a retained calculation. Normalization does not hide a changed input.

Supplied fonts are read from captured project bytes for calculation and painting. Direct file loading checks regular-file descriptors and bounded reads; a FIFO is rejected rather than waited on. Named system families are resolved on demand for the requested style, captured as font data, and reconstructed from those bytes. A system data hash identifies the bytes returned by Skia, which may reconstruct font tables; it is not a promise to identify an OS file path. Automatic OS-font-change monitoring, font lockfiles, and a full platform reproducibility audit remain unfinished. Explicit refresh/recalculation is required to resolve installed fonts again.

## Selection and text coverage

File fonts take precedence over same-named installed families. Family matching is case-insensitive. The bundled Noto Sans JP takes precedence over a system installation, while an explicitly supplied same-named family overrides the bundle. Multiple supplied styles of a family participate in Skia's style matching; the engine no longer assumes that the first face describes every weight or slant.

The default glyph fallback chain is the requested family, Noto Sans JP, then other explicitly supplied families in file/collection order. Duplicate family names are omitted. Skia's arbitrary system fallback is disabled. A missing requested family remains an error even if another font could draw the text.

Coverage is checked after shaping through Paragraph's unresolved-glyph/codepoint result. This handles the actual selected style and supplementary characters, rather than relying on a different face's per-character lookup. Missing text produces `ASSET_NOT_FOUND`, with up to 64 distinct missing characters and the full distinct count. It does not rewrite `.pen` font names or silently export unresolved text.

The previous ordinary fill path lost intrinsic color-glyph palettes by using the paragraph as an alpha mask. The replacement captures the exact glyphs, positions, fonts and run origins from the measured Paragraph when a selected face contains color tables. Native glyph painting reuses the existing fill paints, image placement/clips and mesh raster reservation; ordinary monochrome paragraphs keep their existing path. Color paragraphs retain at most 1,048,576 positioned glyphs, with cancellation checks during capture and every 64 painted glyphs. Decorations use the same native geometry adapter as outline strokes.

Original Rust-generated diagnostic fonts exercise COLR/CPAL color layers, required ZWJ/modifier ligatures and cmap variation-selector mappings. Tests require native shaping and palette pixels, both document schemas, mixed Japanese/monochrome/color text, decorations, gradient/image/mesh fills, translucent compositing and exact CLI/library PNG agreement. The initial hosted reproduction confirmed palette loss; its font also needed its right-hand layer's side bearing corrected. All four replacement tests passed in the Linux focus before Chromium setup, then on all three release platforms in the successful full gate for [e137ff1](https://github.com/EkeMinusYou/lazypencil/actions/runs/40242040657). Their table layouts follow the public [COLR](https://learn.microsoft.com/en-us/typography/opentype/spec/colr), [CPAL](https://learn.microsoft.com/en-us/typography/opentype/spec/cpal), [GSUB](https://learn.microsoft.com/en-us/typography/opentype/spec/gsub), and [cmap](https://learn.microsoft.com/en-us/typography/opentype/spec/cmap) specifications; no third-party font outlines are copied.

Real-font checks acquire the unmodified 2D Noto Emoji CBDT/CBLC and COLRv1 files at the official revision and SHA-256 pins in [emoji-upstream.json](../tests/fixtures/fonts/emoji-upstream.json). The Rust `cargo xtask fixtures emoji` task runs only for hosted acquisition, retains the original OFL, and rechecks cached files. These fonts are not included in the application distribution. Standard native GHA explicitly runs both opt-in checks: seven emoji/Japanese cases per format, composed glyph counts, captured source hashes, intrinsic color or grayscale palettes, and exact native Paragraph/ordinary-render pixels in both document schemas. PNGs and reports bind the upstream font and application source identities. The successful [three-platform gate at `e249f07`](https://github.com/EkeMinusYou/lazypencil/actions/runs/40595368880) retained these checks, the original sbix fixtures and their stroke regressions; its images were independently compared with the previous verified source. This establishes the named fixtures, not every font or emoji sequence.

On macOS, embedded FreeType and a lazily created file-only `FontMgr::custom_empty` handle CBDT and COLRv1. Bounded font-directory inspection selects these formats before loading. Ordinary faces, COLRv0 and sbix retain CoreText, including system-family lookup; captured system bytes use the same file loader. Other targets keep their existing backend. The current layout engine identity is `layout-7`, matching the native alpha-silhouette stroke implementation. The verified `7fe854b` macOS candidate included all 12 required FreeType original notices (178,787 bytes); complete release approval remains separate. Earlier implementation and verification history is recorded in [implementation status](implementation-status.md).

`get_layout` text metrics collect the faces actually visited in positioned glyph runs. A font present in the fallback list but unused in the shaped text is not reported as used. Variable `wght` coordinates are recorded from the selected instance; actual synthetic bold/italic flags are included. Color/bitmap text strokes now recover a bounded native-alpha silhouette when required; [the graphics notes](graphics.md#text-and-outlines) describe precision, limits and current verification. Hosted real-font checks retain their original native-fill comparisons and separately exercise strokes on CBDT/COLRv1 smile, ZWJ and mixed Japanese text. The real-font stroke checks and original bitmap-hole/island/seam regression passed the same three-platform gate; retained PNG comparisons were independently checked. Arbitrary variation-axis controls, all variable italic/named-instance cases and complete emoji-sequence compatibility remain unverified. A zero unresolved-glyph count does not prove that every requested emoji sequence has the intended composition or appearance.

Library callers can replace the fallback list with `Fonts::set_fallback_families`. User/project `[fonts].fallbacks` provides the same policy to CLI, MCP, and TUI. Omit it in both scopes for the default chain; an empty array leaves only the requested family. Effective `default_family` applies when a node omits `fontFamily`, and `locale` appears alongside the requested family in text metrics. Explicit stored families are preserved.

Cancellation can be supplied to `Fonts::layout_with_cancellation`. Common calculations use the same request token and, for script-generated layout, its absolute calculation deadline. Checks surround native shaping and run at every positioned run and at most 64 glyphs while collecting outlines. An interrupted walk returns its cancellation/deadline error without a partial text block; registered fonts can be reused by a later request. An individual Skia call remains cooperative. The glyph-walk cancellation/deadline and bitmap-tracing cancellation cases passed the same full three-platform gate; an individual native call remains cooperative.

## Limits and verification

Current limits are 64 MiB per source or extracted face, 128 MiB of supplied source data, 128 MiB of retained face data, 256 files/faces, 64 variation axes per face, and 4,095 tables per extracted face. Family names are limited to 256 UTF-8 bytes and locale strings to 128 bytes; invalid names produce bounded diagnostics. These are engine input limits, not a measured bound on process RSS. Collection face data is shared with native typefaces where possible, but full lazy loading and configurable cache budgets remain unfinished.

`tests/font_sources.rs` checks the [original tiny diagnostic fonts](../tests/fixtures/fonts/README.md): every collection index, static normal/bold/italic selection, CFF and mixed-outline collections, normalized hashes and standalone-equivalent PNGs, Japanese/supplementary fallback, variable weight metadata, bounded missing-text diagnostics, aggregate input limits, malformed containers, CLI inventory/license, and FIFO rejection. The original assets and generator require no third-party font library and reproduce byte-for-byte.

`tests/mcp.rs` checks actual MCP inventory, resolved mixed-script/style records, PNG image content, and rejection of changed font dependencies without saving the document. `tests/text.rs` retains Japanese wrapping, variable-weight pixel comparisons, missing families/glyphs, and pinned bundled data. An opt-in test also passed with the installed Arial normal/bold/italic faces on macOS arm64, comparing captured source data and selected PostScript names:

```sh
LAZYPENCIL_TEST_FONT_FAMILY=Arial \
cargo test --test font_sources installed_system_styles -- --ignored
```

These checks do not establish complete font compatibility or operation on every OS. The [font/platform design](fonts-and-platforms.md) and [implementation checklist](implementation-status.md) retain the remaining scope.

The three bundled files (`NotoSansJP.ttf`, `OFL.txt`, `METADATA.pb`) were independently compared with their original Google Fonts bytes at the pinned revision; hashes and sizes matched. At `e249f07`, 19 named loading, shaping, fallback, diagnostic, immutable-input, CLI/MCP and configuration regressions passed on all three initial release platforms. The corresponding font source and tests remain unchanged in the current worktree. These checks do not certify arbitrary external fonts or whole-product performance.

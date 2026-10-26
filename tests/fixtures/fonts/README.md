# Original diagnostic fonts

The seven font binaries in this directory are original lazypencil test assets, covered by the project's [MIT license](../../../LICENSE). No third-party glyph outlines or font-building libraries are included in these binaries. Their rectangular glyphs exercise font selection; they are not intended for real design work.

| File | Contents |
| --- | --- |
| `regular.ttf`, `bold.ttf`, `italic.ttf` | Three distinct static styles of `LP Test Sans`; bold intentionally uses a lowercase family spelling to test grouping. Only the letter A is mapped |
| `supplement.ttf` | `LP Test Supplement`, with a diagnostic glyph at U+F0000 |
| `faces.ttc` | All four TrueType faces, with original collection indexes 0–3 |
| `cff.otf` | `LP Test CFF`, with original Type 2 CFF outlines |
| `cff.otc` | A mixed CFF/TrueType collection containing the CFF and supplementary faces |

[manifest.json](manifest.json) records their sizes and SHA-256 digests. Regenerate them using the independent Rust task:

```sh
cargo xtask fixtures fonts --output /tmp/lazypencil-font-fixtures
```

The generator fixes timestamps and names so that all seven binaries and the manifest reproduce byte-for-byte. Compare the temporary output with this directory before accepting changes. Collection extraction tests compare derived face hashes and PNG pixels with the standalone originals.

The native-independent Rust test helper also generates original `LP Test Color` (COLR/CPAL) and `LP Test Bitmap` (sbix) faces at test time. They share the same rectangular outlines, Unicode variation mappings and required ligatures. The sbix variant embeds two original 64 × 70 RGBA PNGs and duplicate-glyph records, following the public [OpenType sbix specification](https://learn.microsoft.com/en-us/typography/opentype/spec/sbix). PNG compression and CRC use the existing flate2 dependency; no font-building library or third-party imagery is added. Independent xtask tests check the SFNT checksum, strike offsets, duplicate records, PNG chunks/CRCs, decompression and every source pixel. Native tests require shaping, intrinsic red/green palettes, three font sizes and both document schemas, with exact Paragraph/ordinary-render pixel equality and retained source-bound PNG reports. The macOS 15 representative gate at `6d6c105` passed all 36 schema/size/case comparisons. Independent decoding checked all 54 PNGs, 3,744,000 compared pixels, the intrinsic palette counts and reversed modifier orientation. The generated font hash is checked in the native test; the font bytes themselves were not separately retained for independent hashing. The final three-platform gate remains pending.

`emoji-upstream.json` separately pins two unmodified, third-party Noto Emoji fonts (CBDT/CBLC and COLRv1) from the official Google Fonts repository. Their SIL OFL 1.1 license and embedded Google copyright remain upstream terms, distinct from the original MIT fixtures. `cargo xtask fixtures emoji --output ...` acquires the exact files and original license for hosted compatibility tests only; it verifies the revision, byte lengths and SHA-256 checksums. These font binaries are neither committed here nor added to the normal application distribution. The standard native GHA checks explicitly run the opt-in real-font test before Chromium setup and retain the rendered PNGs and source-bound report.

# Original diagnostic fonts

All files in this directory are original lazypencil test assets, covered by the project's [MIT license](../../../LICENSE). No third-party glyph outlines, font names, or font-building libraries are included. Their rectangular glyphs exercise font selection; they are not intended for real design work.

| File | Contents |
| --- | --- |
| `regular.ttf`, `bold.ttf`, `italic.ttf` | Three distinct static styles of `LP Test Sans`; bold intentionally uses a lowercase family spelling to test grouping. Only the letter A is mapped |
| `supplement.ttf` | `LP Test Supplement`, with a diagnostic glyph at U+F0000 |
| `faces.ttc` | All four TrueType faces, with original collection indexes 0–3 |
| `cff.otf` | `LP Test CFF`, with original Type 2 CFF outlines |
| `cff.otc` | A mixed CFF/TrueType collection containing the CFF and supplementary faces |

[manifest.json](manifest.json) records their sizes and SHA-256 digests. Regenerate them using only Python's standard library:

```sh
python3 scripts/make_font_fixtures.py --output /tmp/lazypencil-font-fixtures
```

The generator fixes timestamps and names so that all seven binaries and the manifest reproduce byte-for-byte. Compare the temporary output with this directory before accepting changes. Collection extraction tests compare derived face hashes and PNG pixels with the standalone originals.

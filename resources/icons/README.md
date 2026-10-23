# Third-party icon resources

These assets retain their upstream licenses. They are not relicensed by lazypencil's MIT license.

| Files | Source | License notice |
| --- | --- | --- |
| `lucide/icons.json` (normalized SVG primitives, names, aliases, tags) | `lucide-static` 1.57.0, [Lucide](https://github.com/lucide-icons/lucide) | [Complete upstream ISC and Feather-derived MIT notice](lucide/LICENSE) |
| `feather/LICENSE` and optional runtime-installed geometry | `feather-icons` 4.29.2, [Feather](https://github.com/feathericons/feather) | [Complete upstream MIT notice](feather/LICENSE) |
| `phosphor/index.json`, license, provenance, and optional regular geometry | `@phosphor-icons/core` 2.1.1, [Phosphor](https://github.com/phosphor-icons/core) | [Original MIT notice, including its original CRLF bytes](phosphor/LICENSE) |
| `Material Symbols */codepoints.txt`, license, provenance, and separately installed original TTFs | Google Material Symbols at commit `10ecc14790612ca8fd164e3f4ffe9c16fc50d5b7` | [Original Apache-2.0 license](<Material Symbols Outlined/LICENSE>); each original font retains `Copyright 2026 Google LLC.  All Rights Reserved.` in its name table |

Exact archives, digests, and generated data hashes are pinned in [catalogue.json](catalogue.json). The reproducer is [vendor_icons.py](../../scripts/vendor_icons.py). No upstream executable code or icon font is embedded by these resources. The source Feather archive is retained separately as an offline fixture with its own [notice](../../tests/fixtures/icons/README.md).

[vendor_optional_icons.py](../../scripts/vendor_optional_icons.py) reproduces the optional indices, descriptors and provenance. Phosphor's original regular SVG paths are normalized without executing package JavaScript; its canonical names, PascalCase names, deprecated aliases and tags come from the pinned catalogue data literal. Other named Phosphor styles are not substituted for the regular style. The complete original archive is retained as a separate offline test fixture.

Material codepoint files and license texts are unchanged upstream bytes. Setup downloads each original TTF separately and verifies its exact SHA-256 before installation, preserving the complete font, including its copyright/name tables. No font is bundled into the executable. Generated metadata records the source commit, font/index/license URLs and hashes, copyright, and normalization. The derived glyph lookup fixes `FILL=0`, `GRAD=0`, `opsz=24`, and uses the original continuous `wght` axis. These choices do not modify the installed font. Retain the codepoints' Apache license and the original copyright attribution when redistributing derived metadata.

Redistributions containing Lucide geometry must retain its complete notice, including the Feather-derived MIT portion. Redistributions containing Feather geometry or the fixture must retain the Feather MIT notice. Both complete texts are compiled into lazypencil and readable with `lazypencil icons license PACK`; release packages must also retain the relevant notice files. A full audit of executable dependencies and release artifacts remains separate release work.

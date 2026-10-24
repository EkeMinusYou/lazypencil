# Original script compatibility corpus

These scripts and the document were written for lazypencil. The PNG was rendered by the public interactive Export operation of `@pen.dev/cli` 0.3.9, from document version 2.19 at scale 2. No vendor program, skill, source code or credentials are included.

The four script nodes use headers 2.11, 2.19, 2.21 and 2.19. The last node has different dimensions and input values. Each script samples the first 32 `Math.random()` calls and calls at zero-based indices 63, 255, 1023, 4095 and 65535. Each number is encoded as two 24-bit RGB panels. This preserves 48 fractional bits; all observed values have exactly 32 bits of precision. White gaps remain part of the image comparison.

`observations.json` records independently decoded native words, source hashes and the PNG hash. All 1,019,904 RGBA channels matched a separate integer implementation of Mulberry32 with seed 0. The corpus establishes these observations, rather than full compatibility with every script or native version. The algorithm's original CC0 dedication is retained in `resources/licenses/script-random/NOTICE.txt`.

To reproduce without a model request, open `random-sequence.pen` in the public interactive CLI with explicit input, output and preview paths. Call `execute` with `Export(["root"], "png", "/absolute/output/directory")`, then exit. Keep all generated files in a temporary directory. The original source and image must not be overwritten by lazypencil's test output.

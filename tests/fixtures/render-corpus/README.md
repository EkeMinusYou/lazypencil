# Original rendering contract corpus

The .pen, JavaScript and GLSL inputs are original lazypencil test designs. The triangle is freshly expressed with simple root/triangle IDs. All other retained input bytes predate this corpus relocation. They contain no vendor program, guide, skill or credentials.

Expected pixels are computed independently in `xtask/src/fixtures/oracles` using mathematical contours and scanline coverage, authored shader equations, typed-input contract values and the public CC0 Mulberry32 algorithm. These helpers do not invoke lazypencil, Skia, a GLSL metadata parser or Pencil. Lightweight xtask integration tests check the helpers against analytic area equations and contract boundaries. Application tests compare actual renders at their original pixel thresholds, dimensions, schemas and case counts. Expected images must never be generated from the renderer under test.

Pencil-exported PNGs (11 files), decoded observation JSON and the old export provenance record have been removed from the current tree because no individual written permission or agreement is available. This is not a claim of Pencil pixel parity or a legal determination about historical use. Prior Git history and CI artifacts remain unchanged; see [the recorded permission/history status](../../../docs/design-guidance.md).

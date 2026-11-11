# Original rendering contract corpus

The .pen, JavaScript and GLSL inputs are original lazypencil test designs. They use self-authored geometry, scripts and shaders. They contain no vendor program, guide, skill or credentials.

Expected pixels are computed independently in `xtask/src/fixtures/oracles` using mathematical contours and scanline coverage, authored shader equations, typed-input contract values and the public CC0 Mulberry32 algorithm. These helpers do not invoke lazypencil, Skia, a GLSL metadata parser or Pencil. Lightweight xtask integration tests check the helpers against analytic area equations and contract boundaries. Application tests compare actual renders at their original pixel thresholds, dimensions, schemas and case counts. Expected images must never be generated from the renderer under test.

Only original inputs and independent expected-value code belong in this corpus. Pencil-exported images and decoded vendor observations are not test oracles or distributable fixtures. See the [fixture contribution rules](../../../docs/development.md#test-fixtures-and-documentation).

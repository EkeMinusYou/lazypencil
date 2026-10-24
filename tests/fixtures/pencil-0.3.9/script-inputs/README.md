# Original typed input corpus

The document and all scripts were written for lazypencil. The public interactive Export operation of `@pen.dev/cli` 0.3.9 produced `input-controls.png` from the `root` frame of document version 2.19 at scale 2. No vendor implementation, skill text, credentials or unrelated output is included.

The 22 scripts encode the JavaScript type, value, input keys and an undeclared field as JSON. The first four-pixel cell stores the string length in RGB; subsequent cells store UTF-16 code units. This makes the result readable without relying on fonts or vendor internals. `observations.json` records the decoded values, source hashes and native PNG hash. A separate decoder checked all 1,081,344 RGBA channels, including the white gaps.

The cases cover implicit defaults for all six input types, numeric limits including a negative maximum, declared defaults, explicit zero and false, null resets, valid enum/ref selections, empty refs, and stale saved inputs including an undefined variable. Undeclared keys remain in the source document but do not enter `pencil.input`. The unrelated reusable `template` definition sits outside the exported root frame. Invalid type coercions and error-colored cells are not part of this corpus.

Use explicit input, output and preview paths with the public interactive CLI, then export `root` as PNG to a temporary directory. The test exports in lazypencil are separate files; they must not overwrite this native reference or rewrite the source document and scripts.

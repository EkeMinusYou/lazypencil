# Original typed-input contract

The 22 original scripts encode the JavaScript type, value, declared keys and an undeclared field as JSON. A four-pixel RGB cell stores UTF-16 length; subsequent cells store code units. Rust independently supplies the explicit contract values and UTF-16 encoding, without executing the application or reading vendor output.

Cases cover implicit defaults for six types, numeric limits including a negative maximum, declared defaults, explicit zero/false, omitted inputs with declared defaults, valid enum/ref selections, empty refs and stale inputs including an undefined variable. Stored keys remain untouched while undeclared keys do not enter pencil.input. The reusable template lies outside the rendered root.

Tests retain all cases, actual worker checks, byte-preserving input/script checks, no persisted generated nodes/history, and exact all-channel pixel comparison for the current 2.22 document schema at scale two. A separate small render checks all three supported document schemas. Vendor PNG/observation files are absent. See the parent corpus README for fixture contribution rules.

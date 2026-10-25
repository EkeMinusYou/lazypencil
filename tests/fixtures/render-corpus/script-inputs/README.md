# Original typed-input contract

The 22 original scripts encode the JavaScript type, value, declared keys and an undeclared field as JSON. A four-pixel RGB cell stores UTF-16 length; subsequent cells store code units. Rust independently supplies the explicit contract values and UTF-16 encoding, without executing the application or reading vendor output.

Cases cover implicit defaults for six types, numeric limits including a negative maximum, declared defaults, explicit zero/false, null resets, valid enum/ref selections, empty refs and stale inputs including an undefined variable. Stored keys remain untouched while undeclared keys do not enter pencil.input. The reusable template lies outside the rendered root.

Tests retain all cases, actual worker checks, byte-preserving input/script checks, no persisted generated nodes/history, and exact all-channel pixel comparison for 2.19/2.21 at scale two. Vendor PNG/observation files are absent. See the parent corpus README for permission/history status.

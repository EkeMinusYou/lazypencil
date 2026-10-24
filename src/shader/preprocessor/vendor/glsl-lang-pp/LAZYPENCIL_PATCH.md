# lazypencil patch

Based on the unmodified `glsl-lang-pp` 0.8.1 crate archive, SHA-256
`88677876fb27f116015d0ecebda6d77bdf65f3a6fa3f5dfeed10b8c8f04cb2c5`,
published from upstream revision `247fbba98cd1ba9e1c87f4d5a8e0568fbdc48cfe`.
Repository: https://github.com/alixinne/glsl-lang

`src/processor/definition.rs` prescans function macro arguments before the
invoked macro is suppressed for replacement rescanning. This fixes valid nested
calls such as `F(F(x))`, whose inner call otherwise remains unexpanded. Expanded
arguments are cached only when used as ordinary replacement tokens. Unused
arguments are ignored; token concatenation continues to use raw arguments.

The `__LINE__` builtin converts the internal zero-based location to the GLSL
line number. Wrapping addition also preserves an explicit `#line 0` without
changing the zero-based resolver used by source diagnostics.

The workspace-root BSD-3-Clause `LICENSE` from that exact upstream revision is
included because it is omitted from the published crate archive. Its SHA-256 is
`2676516154cc85237cf8e97bb58d93b9e15b80a51066d448ef6c19eca05d88b3`.
All other original archive files are retained. lazypencil exercises the patched
code inside a Wasm instance with a 64 MiB imported-memory maximum; it is not
used to expand untrusted macros in the native application process.

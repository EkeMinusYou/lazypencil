# skia-bindings compatibility patch

Source: crates.io skia-bindings 0.153.3.

Archive SHA-256: `eb325787c91b3f6c34a7c6f2b8a2297b20292d733ca53a08c233e5028aec49af`.

The only source change adds opaque declarations for `std::__rebind_alloc.*` and `std::__hash_table.*`, backported from [rust-skia PR 1335](https://github.com/rust-skia/rust-skia/pull/1335), upstream commit `16bc1d24ae3cc62474583f3388df6e3d883fafb3`. This fixes source builds on macOS 27 with Apple clang 21 without enabling a GPU backend.

The original MIT license is included in LICENSE. Remove this patch when an upstream crates.io release includes the fix.

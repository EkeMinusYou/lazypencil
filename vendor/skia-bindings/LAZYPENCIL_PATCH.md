# skia-bindings compatibility patch

Source: crates.io skia-bindings 0.153.3.

Archive SHA-256: `eb325787c91b3f6c34a7c6f2b8a2297b20292d733ca53a08c233e5028aec49af`.

Opaque declarations for `std::__rebind_alloc.*` and `std::__hash_table.*` are backported from [rust-skia PR 1335](https://github.com/rust-skia/rust-skia/pull/1335), upstream commit `16bc1d24ae3cc62474583f3388df6e3d883fafb3`. This fixes source builds on macOS 27 with Apple clang 21 without enabling a GPU backend.

Dependency source extraction also defers directory entries until descendants are unpacked, matching `tar::Archive::unpack` while retaining the existing root and `infra/` filters. On Unix, it additionally restores the directories' archived modification times with the standard library: tar 0.4.46 preserves file times but does not restore directory times. This prevents a newly extracted `skia/include` from appearing newer than the first Cargo build. Actual header edits remain Cargo inputs. Cargo fingerprints and cache timestamps are unchanged; Windows retains its existing extraction-time mtimes.

The original MIT license is included in LICENSE. Remove each patch when an upstream crates.io release includes its fix.

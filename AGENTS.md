# Agent instructions

- Respond in Japanese. Keep code comments, commit messages, and PR descriptions in English.
- Commit completed changes proactively in coherent, reviewable units after the relevant checks pass. Keep unrelated changes in separate commits.
- Limit local build parallelism to two jobs to balance CPU usage and build time. Do not run multiple builds concurrently.
- Cargo defaults to two jobs through `.cargo/config.toml`. The vendored Skia build forwards Cargo's `NUM_JOBS` to Ninja, so native builds use the same job limit.
- Use a release build (`cargo build --locked --release --bin lazypencil`) when replacing the user's local executable. Keep debug builds for development and tests.
- Choose validation from the changed behavior before editing. Ordinary fixes finish after relevant checks and a coherent commit; do not expand them into product-wide release work.
- Start with focused tests. Broaden validation for shared behavior, dependency/build changes, or evidence of a regression; a passing focused check is not a claim that the full suite passed.
- Reuse an existing CI run for the same commit and inputs. Do not dispatch full CI after passing targeted checks without a concrete reason. Watch the run with `cargo xtask ci watch --run ID --head SHA` instead of repeated model-driven status queries.
- Keep implementation, PR creation, merging, and installation as distinct completion conditions. Wait for the checks required by the requested outcome; do not make unrelated CI or distribution preparation part of every fix.
- When local installation is requested, finish the edits and relevant checks first, then build release once for that final state. Batch intermediate visual adjustments before replacing the installed executable.
- Use focused xtask tests to isolate user settings and prepare the native helper. Retain the two-job, one-build-at-a-time limit.
- Use `~/Library/Caches/lazypencil/target` as `CARGO_TARGET_DIR` for all builds and tests, including release builds and native helper builds. Do not create per-task, per-feature, or dated target directories; each one grows to tens of gigabytes and exhausted the disk. Builds run one at a time, so a shared directory does not cause lock contention.
- When a different toolchain is genuinely required (for example reproducing CI with Rust 1.94), use one fixed directory per toolchain such as `~/Library/Caches/lazypencil/target-rust194` and reuse it. Remove any other target directory you create once the task is done.
- Read the files and evidence relevant to the current change. Carry a short task summary into independent follow-up work instead of repeatedly reading the full implementation history.
- Keep repository documentation limited to maintained user and developer references. Do not commit task diaries, progress logs, audit reports, verification timelines, or one-off measurement reports; communicate these in the conversation or keep temporary artifacts outside the repository. Preserve release notes, reusable test fixtures, and required licensing and provenance materials.

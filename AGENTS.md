# Agent instructions

- Respond in Japanese. Keep code comments, commit messages, and PR descriptions in English.
- Limit local build parallelism to two jobs to balance CPU usage and build time. Do not run multiple builds concurrently.
- Cargo defaults to two jobs through `.cargo/config.toml`. For Skia source builds, also set `SKIA_NINJA_COMMAND="$PWD/scripts/ninja-limited.sh"`; Ninja does not inherit Cargo's job limit.
- Use a release build (`cargo build --locked --release --bin lazypencil`) when replacing the user's local executable. Keep debug builds for development and tests.

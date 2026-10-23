#!/bin/sh
set -eu

# Keep reference fixtures independent of the developer's real user settings.
lazypencil_test_python="${LAZYPENCIL_TEST_PYTHON:-python3}"
lazypencil_test_config="$("$lazypencil_test_python" -c 'import tempfile; print(tempfile.mkdtemp(prefix="lazypencil-test-config-"))')"
trap '"$lazypencil_test_python" -c "import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)" "$lazypencil_test_config"' EXIT
export LAZYPENCIL_CONFIG_DIR="$lazypencil_test_config/user"

rustc -vV
cargo test --locked --no-default-features -- --test-threads=2
cargo test --locked -- --test-threads=2
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --no-default-features --all-targets -- -D warnings
# Leave the runnable application with its default rendering/TUI features.
cargo build --locked

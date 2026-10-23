#!/bin/sh
set -eu

# Keep reference fixtures independent of the developer's real user settings.
lazypencil_test_python="${LAZYPENCIL_TEST_PYTHON:-python3}"
lazypencil_test_config="$("$lazypencil_test_python" -c 'import tempfile; print(tempfile.mkdtemp(prefix="lazypencil-test-config-"))')"
trap '"$lazypencil_test_python" -c "import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)" "$lazypencil_test_config"' EXIT
export LAZYPENCIL_CONFIG_DIR="$lazypencil_test_config/user"

rustc -vV
lazypencil_test_status=0
# Collect failures across test binaries and configurations so one platform
# failure does not hide the remaining checks. Preserve a failing exit status.
cargo test --locked --no-default-features --no-fail-fast -- --test-threads=2 || lazypencil_test_status=1
cargo test --locked --no-fail-fast -- --test-threads=2 || lazypencil_test_status=1
cargo clippy --locked --all-targets -- -D warnings || lazypencil_test_status=1
cargo clippy --locked --no-default-features --all-targets -- -D warnings || lazypencil_test_status=1
# Leave the runnable application with its default rendering/TUI features.
cargo build --locked || lazypencil_test_status=1
exit "$lazypencil_test_status"

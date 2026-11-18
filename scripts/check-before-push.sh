#!/usr/bin/env bash
set -euo pipefail

if (( $# > 1 )); then
    echo 'Usage: bash scripts/check-before-push.sh [COMMIT]' >&2
    exit 2
fi

cd "$(git rev-parse --show-toplevel)"
check_tree=$PWD
if (( $# == 1 )); then
    revision=$(git rev-parse --verify --end-of-options "$1^{commit}")
    check_tree=$(mktemp -d)
    trap 'rm -rf -- "$check_tree"' EXIT
    git archive "$revision" | tar -xf - -C "$check_tree"
    printf 'Checking %s before push\n' "$revision"
fi

failed=0
cargo fmt --all --manifest-path "$check_tree/Cargo.toml" -- --check || failed=1
cargo fmt --all --manifest-path "$check_tree/xtask/Cargo.toml" -- --check || failed=1
cargo xtask notices rust verify --root "$check_tree" || failed=1
cargo xtask notices rust verify --root "$check_tree/xtask" --bundle "$check_tree/xtask/notices" || failed=1
cargo xtask notices skia-wrapper --root "$check_tree" || failed=1
exit "$failed"

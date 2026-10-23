#!/bin/sh
set -eu

# CI caches dependencies, not application binaries or old render evidence.
# This also prevents archive timestamps from making old application output
# appear current after a dependency cache is restored.
: "${CARGO_TARGET_DIR:?Set an explicit hosted-runner target directory}"
"${LAZYPENCIL_TEST_PYTHON:-python3}" - <<'PY'
import os
from pathlib import Path

directory = Path(os.environ["CARGO_TARGET_DIR"])
if not directory.is_absolute() or directory.resolve().parent == directory.resolve():
    raise SystemExit("Refusing an ambiguous/root compilation cache directory")
for name in ("release-smoke", "release-dashboard", "release-graphics"):
    for extension in ("png", "json"):
        path = directory / f"{name}.{extension}"
        if path.is_file() or path.is_symlink():
            path.unlink()
PY
cargo clean --locked --package lazypencil --profile dev
cargo clean --locked --package lazypencil --profile release

#!/bin/sh
set -eu

lazypencil_target="${CARGO_TARGET_DIR:-target}"
lazypencil_python="${LAZYPENCIL_TEST_PYTHON:-python3}"
lazypencil_release_config="$("$lazypencil_python" -c 'import tempfile; print(tempfile.mkdtemp(prefix="lazypencil-release-config-"))')"
trap '"$lazypencil_python" -c "import shutil,sys; shutil.rmtree(sys.argv[1], ignore_errors=True)" "$lazypencil_release_config"' EXIT
export LAZYPENCIL_CONFIG_DIR="$lazypencil_release_config/user"
cargo build --locked --release
cargo run --locked --release -- --version
cargo run --locked --release -- --json render examples/mesh-study.pen --output "$lazypencil_target/release-smoke.png" > "$lazypencil_target/release-smoke.json"
"$lazypencil_python" - "$lazypencil_target" <<'PY'
import json
import hashlib
from pathlib import Path
import struct
import shutil
import subprocess
import sys
import tempfile

directory = Path(sys.argv[1])
report = json.loads((directory / "release-smoke.json").read_text(encoding="utf-8"))
assert (report["width"], report["height"], report["node_count"]) == (1040, 900, 26), report
assert report["diagnostics"] == [], report
assert report["assets"] == [] and report["dynamic_artifacts"] == [], report
png = (directory / "release-smoke.png").read_bytes()
assert png[:8] == b"\x89PNG\r\n\x1a\n" and struct.unpack(">II", png[16:24]) == (1040, 900)
assert hashlib.sha256(png).hexdigest() == report["png_hash"]
print(json.dumps({"release_render": "ok", "width": 1040, "height": 900, "diagnostics": 0}))

# Exercise the distributed generator and actual component/text/image renderer.
with tempfile.TemporaryDirectory(prefix="lazypencil-release-", dir=directory) as temporary:
    fixture = Path(temporary) / "input"
    subprocess.run([sys.executable, "scripts/make_dashboard_performance_fixture.py", "--output-dir", str(fixture)], check=True)
    result = subprocess.run([
        "cargo", "run", "--quiet", "--locked", "--release", "--", "--json", "render",
        str(fixture / "dashboard.pen"), "--output", str(directory / "release-dashboard.png"),
    ], check=True, stdout=subprocess.PIPE, text=True, encoding="utf-8")
    report = json.loads(result.stdout)
    assert (report["width"], report["height"], report["node_count"]) == (1920, 1080, 1000), report
    assert report["diagnostics"] == [] and len(report["assets"]) == 1, report
    assert report["dynamic_artifacts"] == [] and report["script_executions"] == [], report
    png = (directory / "release-dashboard.png").read_bytes()
    assert png[:8] == b"\x89PNG\r\n\x1a\n" and struct.unpack(">II", png[16:24]) == (1920, 1080)
    assert hashlib.sha256(png).hexdigest() == report["png_hash"]
    (directory / "release-dashboard.json").write_text(result.stdout, encoding="utf-8")
    print(json.dumps({"release_dashboard": "ok", "expanded_nodes": 1000, "diagnostics": 0}))

# Exercise text decoration contours in the actual optimized renderer. A fixed
# artboard keeps export bounds identical, so missing decoration ink cannot pass
# merely because an empty margin changed the PNG dimensions.
with tempfile.TemporaryDirectory(prefix="lazypencil-release-graphics-", dir=directory) as temporary:
    fixture = Path(temporary)
    shutil.copytree("examples/assets", fixture / "assets")
    design = json.loads(Path("examples/graphics-study.pen").read_text(encoding="utf-8"))
    result = subprocess.run([
        "cargo", "run", "--quiet", "--locked", "--release", "--", "--json", "render",
        "examples/graphics-study.pen", "--output", str(directory / "release-graphics.png"),
    ], check=True, stdout=subprocess.PIPE, text=True, encoding="utf-8")
    report = json.loads(result.stdout)
    assert (report["width"], report["height"]) == (1000, 760), report
    assert report["diagnostics"] == [] and len(report["assets"]) == 1, report
    assert report["dynamic_artifacts"] == [] and report["script_executions"] == [], report
    png = (directory / "release-graphics.png").read_bytes()
    assert png[:8] == b"\x89PNG\r\n\x1a\n" and struct.unpack(">II", png[16:24]) == (1000, 760)
    assert hashlib.sha256(png).hexdigest() == report["png_hash"]
    (directory / "release-graphics.json").write_text(result.stdout, encoding="utf-8")
    def find(node):
        if node.get("id") == "outline":
            return node
        for child in node.get("children", []):
            result = find(child)
            if result is not None:
                return result
    outline = next(found for node in design["children"] if (found := find(node)) is not None)
    assert outline["underline"] is True
    outline["fill"] = []
    hashes = []
    for enabled in [False, True]:
        outline["underline"] = enabled
        document = fixture / "strokes.pen"
        document.write_text(json.dumps(design, ensure_ascii=False), encoding="utf-8")
        result = subprocess.run([
            "cargo", "run", "--quiet", "--locked", "--release", "--", "--json", "render",
            str(document), "--output", str(fixture / "strokes.png"),
        ], check=True, stdout=subprocess.PIPE, text=True, encoding="utf-8")
        rendered = json.loads(result.stdout)
        assert (rendered["width"], rendered["height"]) == (1000, 760), rendered
        assert rendered["diagnostics"] == [], rendered
        digest = hashlib.sha256((fixture / "strokes.png").read_bytes()).hexdigest()
        assert digest == rendered["png_hash"]
        hashes.append(digest)
    assert hashes[0] != hashes[1], "Decoration-only stroke changes must produce visible output"
    print(json.dumps({"release_graphics": "ok", "decoration_stroke_ink": True, "diagnostics": 0}))
PY

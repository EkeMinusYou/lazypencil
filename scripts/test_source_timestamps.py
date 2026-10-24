import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("source_timestamps", Path(__file__).parent / "ci/source_timestamps.py")
timestamps = importlib.util.module_from_spec(spec)
spec.loader.exec_module(timestamps)


class SourceTimestampTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.source = Path(self.temporary.name) / "source"
        self.dist = timestamps.DISTRIBUTION
        self.write_source()

    def write_source(self):
        if self.source.exists():
            shutil.rmtree(self.source)
        self.source.mkdir()
        self.files = {"Cargo.toml": b"manifest", "Cargo.lock": b"lock", "LICENSE": b"license",
                      "README.md": b"readme", "src/lib.rs": b"pub fn example() {}",
                      "vendor/skia-bindings/build.rs": b"fn main() {}"}
        entries = []
        for name, data in self.files.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            path.chmod(0o644)
            entries.append({"path": name, "sha256": self.dist["digest"](data), "mode": 0o644, "bytes": len(data)})
        self.manifest = self.source / "SOURCE-MANIFEST.json"
        self.manifest.write_text(json.dumps({"files": entries}))

    def test_reextracting_identical_inputs_keeps_them_older_than_cached_fingerprints(self):
        # Reproduce extraction after an earlier Cargo build without compiling.
        anchor = timestamps.SOURCE_TIME_NS + 60_000_000_000
        for name in self.files:
            os.utime(self.source / name, ns=(anchor + 1, anchor + 1))
        before = {name: (self.source / name).read_bytes() for name in self.files}
        report = timestamps.prepare(self.source)
        self.assertEqual(report["files"], len(before))
        for name, data in before.items():
            path = self.source / name
            self.assertLess(path.stat().st_mtime_ns, anchor)
            self.assertEqual(path.read_bytes(), data)
            self.assertEqual(path.stat().st_mode & 0o777, 0o644)
        self.assertEqual((self.source / "vendor/skia-bindings").stat().st_mtime_ns, timestamps.SOURCE_TIME_NS)
        self.assertEqual(timestamps.prepare(self.source), report)

    def test_changed_missing_extra_and_executable_inputs_fail_before_any_timestamp_changes(self):
        for change in ("bytes", "missing", "extra", "mode", "duplicate", "escape"):
            with self.subTest(change=change):
                self.write_source()
                sentinel = self.source / "Cargo.lock"
                before = sentinel.stat().st_mtime_ns
                path = self.source / "src/lib.rs"
                if change == "bytes":
                    path.write_bytes(b"pub fn changed() {}")
                elif change == "missing":
                    path.unlink()
                elif change == "extra":
                    (self.source / "src/extra.rs").write_bytes(b"extra")
                elif change == "mode":
                    path.chmod(0o755)
                else:
                    manifest = json.loads(self.manifest.read_text())
                    if change == "duplicate":
                        manifest["files"][-1] = manifest["files"][0]
                    else:
                        manifest["files"][-1]["path"] = "../outside"
                    self.manifest.write_text(json.dumps(manifest))
                with self.assertRaises((ValueError, self.dist["DistributionError"])):
                    timestamps.prepare(self.source)
                self.assertEqual(sentinel.stat().st_mtime_ns, before)

    def test_checkout_and_symlink_inputs_are_rejected_without_touching_external_files(self):
        (self.source / ".git").mkdir()
        with self.assertRaises(ValueError):
            timestamps.prepare(self.source)
        (self.source / ".git").rmdir()
        outside = self.source.parent / "outside"
        outside.write_bytes(b"outside")
        before = outside.stat().st_mtime_ns
        path = self.source / "src/lib.rs"
        path.unlink()
        path.symlink_to(outside)
        with self.assertRaises(self.dist["DistributionError"]):
            timestamps.prepare(self.source)
        self.assertEqual(outside.stat().st_mtime_ns, before)
        self.assertEqual(outside.read_bytes(), b"outside")


if __name__ == "__main__":
    unittest.main()

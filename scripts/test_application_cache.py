import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("application_cache", Path(__file__).parent / "ci/application_cache.py")
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class ApplicationCacheTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.target = self.root / "target"
        self.directory = self.root / "cache"
        self.source.mkdir()
        self.files = {"Cargo.toml": '[package]\nname="lazypencil"\nversion="0.1.0"\n',
                      "src/lib.rs": "pub fn example() {}\n", "docs/guide.md": "Embedded guide\n",
                      ".github/workflows/ci.yml": "Original workflow\n"}
        self.manifest()
        self.fingerprint = "1234567890abcdef"
        self.marker = self.target / f"debug/.fingerprint/lazypencil-{self.fingerprint}/lib-lazypencil.json"
        self.marker.parent.mkdir(parents=True)
        self.marker.write_text("{}")
        self.library = self.target / f"debug/deps/liblazypencil-{self.fingerprint}.rlib"
        self.library.parent.mkdir()
        self.library.write_bytes(b"compiled fixture library")
        self.dep_info = self.target / f"debug/deps/lazypencil-{self.fingerprint}.d"
        self.dep_info.write_text(f"library: {self.source}/src/lib.rs {self.source}/docs/guide.md\n")

    def manifest(self, revision="1" * 40):
        entries = []
        for name, content in self.files.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
            entries.append({"path": name, "sha256": cache.digest(content.encode()), "mode": 0o644, "bytes": len(content.encode())})
        (self.source / "SOURCE-MANIFEST.json").write_text(json.dumps({"git": {"revision": revision}, "files": entries}))

    def identity(self, native="native-v2-ubuntu-24.04-fixture"):
        return cache.identity(self.source, self.target, native)

    def stage(self):
        return cache.stage(self.source, self.target, self.directory, self.identity())

    def test_same_inputs_reuse_outputs_but_never_cached_images_or_dependencies(self):
        (self.target / "release-smoke.png").write_bytes(b"old evidence")
        (self.library.parent / "libdependency-1234567890abcdef.rlib").write_bytes(b"dependency")
        report = self.stage()
        self.assertEqual(report["files"], 3)
        shutil.rmtree(self.target)
        restored = cache.restore(self.source, self.target, self.directory, self.identity())
        self.assertTrue(restored["reused"])
        self.assertEqual(self.library.read_bytes(), b"compiled fixture library")
        self.assertFalse((self.target / "release-smoke.png").exists())
        self.assertFalse((self.library.parent / "libdependency-1234567890abcdef.rlib").exists())

    def test_source_build_environment_and_paths_bind_identity(self):
        original = self.identity()["fingerprint"]
        self.assertNotEqual(self.identity("native-v2-ubuntu-24.04-changed-compiler-image-flags")["fingerprint"], original)
        other = cache.identity(self.source, self.root / "another-target", "native-v2-ubuntu-24.04-fixture")
        self.assertNotEqual(other["fingerprint"], original)
        for name in ("src/lib.rs", "Cargo.toml", "docs/guide.md"):
            old = self.files[name]
            self.files[name] += "\n"
            self.manifest()
            self.assertNotEqual(self.identity()["fingerprint"], original)
            self.files[name] = old
        self.manifest()

    def test_public_source_hash_matches_identity_and_ignores_manifest_order(self):
        manifest = json.loads((self.source / "SOURCE-MANIFEST.json").read_text())
        expected = cache.source_fingerprint(manifest)
        original = self.identity()
        self.assertIn(f"-{expected}-", original["primary"])
        manifest["files"].reverse()
        (self.source / "SOURCE-MANIFEST.json").write_text(json.dumps(manifest))
        self.assertEqual(cache.source_fingerprint(manifest), expected)
        self.assertEqual(self.identity(), original)
        other = self.identity("native-v2-macos-15-another-image")
        self.assertIn(f"-{expected}-", other["primary"])
        self.assertNotEqual(other["fingerprint"], original["fingerprint"])

    def test_workflow_changes_reuse_only_when_not_compiler_inputs(self):
        self.stage()
        original = self.identity()["fingerprint"]
        self.files[".github/workflows/ci.yml"] = "Changed workflow\n"
        self.manifest("2" * 40)
        self.assertEqual(self.identity()["fingerprint"], original)
        shutil.rmtree(self.target)
        report = cache.restore(self.source, self.target, self.directory, self.identity())
        self.assertEqual(report["producer"], "1" * 40)
        self.assertEqual(report["source"], "2" * 40)
        self.dep_info.write_text(f"library: {self.source}/.github/workflows/ci.yml\n")
        with self.assertRaises(cache.CacheError):
            self.stage()

    def test_ci_only_changes_reuse_but_embedded_ci_files_disable_reuse(self):
        excluded = ("scripts/ci/focus.py", "scripts/ci/nested/helper.py",
                    "scripts/test_application_cache.py")
        for name in excluded:
            self.files[name] = "Original CI helper\n"
        self.manifest()
        self.stage()
        original = self.identity()["fingerprint"]
        for name in excluded:
            with self.subTest(name=name):
                self.files[name] = "Changed CI helper\n"
                self.manifest("2" * 40)
                self.assertEqual(self.identity()["fingerprint"], original)
                shutil.rmtree(self.target)
                self.assertTrue(cache.restore(self.source, self.target, self.directory, self.identity())["reused"])
                self.dep_info.write_text(f"library: {self.source}/{name}\n")
                with self.assertRaises(cache.CacheError):
                    self.stage()
                with self.assertRaises(cache.CacheError):
                    cache.restore(self.source, self.target, self.directory, self.identity())
                self.dep_info.write_text(f"library: {self.source}/src/lib.rs {self.source}/docs/guide.md\n")
                self.stage()

    def test_other_scripts_and_similar_path_names_remain_bound(self):
        original = self.identity()["fingerprint"]
        for name in ("scripts/dist.py", "scripts/ci_helper.py", "scripts/test_input.json",
                     "scripts/test_data/fixture.py", "tests/support/test_fixture.py"):
            with self.subTest(name=name):
                self.files[name] = "Changed build or runtime input\n"
                self.manifest()
                self.assertNotEqual(self.identity()["fingerprint"], original)
                del self.files[name]
                self.manifest()

    def test_same_mtime_source_changes_cannot_reuse_application_outputs(self):
        self.stage()
        path = self.source / "src/lib.rs"
        before = path.stat()
        path.write_text("pub fn changed() {}\n")
        os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns))
        with self.assertRaises(cache.CacheError):
            cache.restore(self.source, self.target, self.directory, self.identity())
        self.files["src/lib.rs"] = path.read_text()
        self.manifest()
        with self.assertRaises(cache.CacheError):
            cache.restore(self.source, self.target, self.directory, self.identity())

    def test_corrupt_outputs_are_rejected_before_target_changes(self):
        self.stage()
        cached_library = self.directory / "files" / self.library.relative_to(self.target)
        cached_library.write_bytes(b"corrupt")
        shutil.rmtree(self.target)
        with self.assertRaises(cache.CacheError):
            cache.restore(self.source, self.target, self.directory, self.identity())
        self.assertFalse(self.target.exists())

    def test_changed_compile_time_environment_is_rejected(self):
        with self.dep_info.open("a") as stream:
            stream.write("# env-dep:EXAMPLE_BUILD_SETTING=one\n")
        with patch.dict(os.environ, {"EXAMPLE_BUILD_SETTING": "one"}):
            self.stage()
        with patch.dict(os.environ, {"EXAMPLE_BUILD_SETTING": "two"}), self.assertRaises(cache.CacheError):
            cache.restore(self.source, self.target, self.directory, self.identity())

    def test_unknown_generated_inputs_and_custom_build_scripts_disable_reuse(self):
        self.dep_info.write_text(f"library: {self.source}/SOURCE-MANIFEST.json\n")
        with self.assertRaises(cache.CacheError):
            self.stage()
        (self.source / "build.rs").write_text("fn main() {}")
        with self.assertRaises(cache.CacheError):
            self.identity()

    def test_escaped_spaces_in_compiler_inputs_are_checked(self):
        self.files["docs/a guide.md"] = "Embedded space\n"
        self.manifest()
        self.dep_info.write_text(f"library: {self.source}/docs/a\\ guide.md\n")
        self.stage()
        shutil.rmtree(self.target)
        self.assertTrue(cache.restore(self.source, self.target, self.directory, self.identity())["reused"])

    def test_unsafe_paths_and_symlinks_are_rejected(self):
        for name in ("../outside", "/tmp/outside", "debug/deps/../../outside", "debug/build/anything", "release-smoke.png"):
            self.assertFalse(cache.allowed(name))
        self.stage()
        cached_library = self.directory / "files" / self.library.relative_to(self.target)
        cached_library.unlink()
        cached_library.symlink_to(self.library)
        with self.assertRaises(cache.CacheError):
            cache.restore(self.source, self.target, self.directory, self.identity())


if __name__ == "__main__":
    unittest.main()

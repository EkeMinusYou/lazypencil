"""Integrity and completeness checks for the actual source archive interface."""

import gzip
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import tomllib
import unittest

import dist


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / "project"
        self.root.mkdir()
        (self.root / "src").mkdir()
        (self.root / "src/main.rs").write_text("fn main() {}\n")
        (self.root / "Cargo.toml").write_text('[package]\nname="fixture"\nversion="0.1.0"\nrust-version="1.94"\n[patch.crates-io]\ndep={path="vendor/skia-bindings"}\n')
        (self.root / "Cargo.lock").write_text("version=4\n")
        (self.root / "LICENSE").write_text("MIT License\nCopyright (c) Fixture\n")
        (self.root / "README.md").write_text("Fixture 日本語\n", encoding="utf-8")
        (self.root / "vendor/skia-bindings/src").mkdir(parents=True)
        (self.root / "vendor/skia-bindings/Cargo.toml").write_text('[package]\nname="dep"\nversion="0.1.0"\n')
        (self.root / "vendor/skia-bindings/src/lib.rs").write_text("// Patched implementation\n")
        self.archive = self.root.parent / "source.tar.gz"

    def create(self, path=None):
        return dist.source(self.root, path or self.archive, allow_dirty=True)

    def members(self):
        with tarfile.open(self.archive, "r:gz") as archive:
            return [(member.name, archive.extractfile(member).read()) for member in archive]

    def replace_archive(self, members):
        with tarfile.open(self.archive, "w:gz") as archive:
            for name, data in members:
                member = tarfile.TarInfo(name)
                member.size = len(data)
                member.mode = 0o644
                archive.addfile(member, io.BytesIO(data))

    def test_repeatable_source_keeps_local_patches_and_omits_checkout_and_cache(self):
        for name in ["vendor/skia-bindings/skia/large-cache", "scripts/__pycache__/cache.pyc", "target/debug/secret"]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("not a distributable file")
        (self.root / "scripts/tool.py").write_text("print('test')\n")
        first = self.create()
        second = self.create(self.root.parent / "second.tar.gz")
        self.assertEqual(first["sha256"], second["sha256"])
        output = self.root.parent / "extracted"
        verified = dist.verify(self.archive, output)
        self.assertEqual(verified["sha256"], first["sha256"])
        self.assertEqual((output / "vendor/skia-bindings/src/lib.rs").read_bytes(), (self.root / "vendor/skia-bindings/src/lib.rs").read_bytes())
        self.assertEqual((output / "README.md").read_text(encoding="utf-8"), "Fixture 日本語\n")
        self.assertFalse((output / "vendor/skia-bindings/skia").exists())
        self.assertFalse((output / "scripts/__pycache__").exists())
        self.assertFalse((output / "target").exists())

    def test_missing_patched_dependency_never_produces_a_distribution(self):
        (self.root / "vendor/skia-bindings/Cargo.toml").unlink()
        with self.assertRaisesRegex(dist.DistributionError, "patched dependency"):
            self.create()
        self.assertFalse(self.archive.exists())

    def test_release_source_requires_clean_git_and_preserves_provenance(self):
        # This disposable test repository never changes the project's Git state.
        hooks = self.root.parent / "empty-hooks"
        hooks.mkdir()

        def git(*args):
            return subprocess.run([
                "git", "-C", str(self.root), "-c", "user.name=Fixture",
                "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false",
                "-c", "core.autocrlf=false", "-c", f"core.hooksPath={hooks}", *args,
            ], check=True, capture_output=True, text=True).stdout.strip()

        git("init", "--quiet", f"--template={hooks}")
        git("add", ".")
        git("commit", "--quiet", "-m", "Add distribution test fixture")
        (self.root / ".git/info").mkdir(exist_ok=True)
        with (self.root / ".git/info/exclude").open("a", encoding="utf-8") as ignored:
            ignored.write("\nsrc/ignored-local.rs\n")
        (self.root / "src/ignored-local.rs").write_text("// Private local data\n")
        self.assertEqual(git("status", "--porcelain=v1"), "")
        expected = {"revision": git("rev-parse", "HEAD"), "dirty": False}
        report = dist.source(self.root, self.archive)
        self.assertEqual(report["git"], expected)
        self.assertFalse(any(name.endswith("/src/ignored-local.rs") for name, _ in self.members()))
        manifest = json.loads(self.members()[0][1])
        self.assertEqual(manifest["git"], expected)

        readme = self.root / "README.md"
        original = readme.read_bytes()
        readme.write_bytes(original + b"Changed\n")
        refused = self.root.parent / "dirty.tar.gz"
        with self.assertRaisesRegex(dist.DistributionError, "dirty checkout"):
            dist.source(self.root, refused)
        self.assertFalse(refused.exists())
        self.assertTrue(dist.source(self.root, refused, allow_dirty=True)["git"]["dirty"])
        with tarfile.open(refused, "r:gz") as archive:
            self.assertFalse(any(name.endswith("/src/ignored-local.rs") for name in archive.getnames()))

        readme.write_bytes(original)
        (self.root / "src/untracked.rs").write_text("// Untracked\n")
        with self.assertRaisesRegex(dist.DistributionError, "dirty checkout"):
            dist.source(self.root, self.root.parent / "untracked.tar.gz")
        snapshot = self.root.parent / "development.tar.gz"
        dist.source(self.root, snapshot, allow_dirty=True)
        with tarfile.open(snapshot, "r:gz") as archive:
            self.assertIn("fixture-0.1.0/src/untracked.rs", archive.getnames())
            self.assertNotIn("fixture-0.1.0/src/ignored-local.rs", archive.getnames())

    def test_symlinks_are_rejected_instead_of_reading_outside_source(self):
        outside = self.root.parent / "private.txt"
        outside.write_text("private")
        path = self.root / "src/linked"
        try:
            path.symlink_to(outside)
        except OSError:
            self.skipTest("This environment cannot create file symlinks")
        with self.assertRaisesRegex(dist.DistributionError, "symlinks"):
            self.create()
        self.assertFalse(self.archive.exists())

    def test_corrupted_content_is_rejected_before_creating_extraction_destination(self):
        self.create()
        members = [(name, b"tampered" if name.endswith("/src/main.rs") else data) for name, data in self.members()]
        self.replace_archive(members)
        output = self.root.parent / "extracted"
        with self.assertRaisesRegex(dist.DistributionError, "integrity mismatch"):
            dist.verify(self.archive, output)
        self.assertFalse(output.exists())

    def test_unlisted_duplicate_and_escaping_paths_are_rejected(self):
        self.create()
        original = self.members()
        for extra in [original[-1], ("fixture-0.1.0/extra.txt", b"unlisted"), ("../escape", b"bad"), ("fixture-0.1.0/C:/escape", b"bad"), ("fixture-0.1.0/NUL.txt", b"bad")]:
            with self.subTest(extra=extra[0]):
                self.replace_archive(original + [extra])
                with self.assertRaises(dist.DistributionError):
                    dist.verify(self.archive)

    def test_existing_archive_and_extraction_directory_are_preserved(self):
        self.create()
        original = self.archive.read_bytes()
        with self.assertRaises(dist.DistributionError):
            self.create()
        self.assertEqual(self.archive.read_bytes(), original)
        output = self.root.parent / "existing"
        output.mkdir()
        (output / "keep.txt").write_text("keep")
        with self.assertRaises(dist.DistributionError):
            dist.verify(self.archive, output)
        self.assertEqual((output / "keep.txt").read_text(), "keep")

    def test_modified_manifest_cannot_remove_a_required_patch_or_change_identity(self):
        self.create()
        original = self.members()
        for remove_patch in [True, False]:
            manifest = json.loads(original[0][1])
            if remove_patch:
                omitted = "vendor/skia-bindings/Cargo.toml"
                manifest["files"] = [item for item in manifest["files"] if item["path"] != omitted]
                members = [(name, data) for name, data in original[1:] if name != "fixture-0.1.0/" + omitted]
            else:
                manifest["package"]["version"] = "9.9.9"
                members = original[1:]
            self.replace_archive([(original[0][0], json.dumps(manifest).encode())] + members)
            with self.assertRaises(dist.DistributionError):
                dist.verify(self.archive)

    def test_archive_links_and_excessive_payload_are_not_extracted(self):
        self.create()
        original = self.members()
        with tarfile.open(self.archive, "w:gz") as archive:
            link = tarfile.TarInfo("fixture-0.1.0/link")
            link.type = tarfile.SYMTYPE
            link.linkname = "../../private"
            archive.addfile(link)
        with self.assertRaisesRegex(dist.DistributionError, "regular files"):
            dist.verify(self.archive)
        self.replace_archive(original)
        previous = dist.MAX_BYTES
        try:
            dist.MAX_BYTES = 8
            with self.assertRaisesRegex(dist.DistributionError, "byte budget"):
                dist.verify(self.archive)
        finally:
            dist.MAX_BYTES = previous

    def test_decompression_budget_applies_before_parsing_a_compression_bomb(self):
        with gzip.open(self.archive, "wb") as stream:
            stream.write(b"\0" * 100000)
        previous = dist.MAX_BYTES, dist.MAX_MANIFEST, dist.MAX_FILES
        try:
            dist.MAX_BYTES, dist.MAX_MANIFEST, dist.MAX_FILES = 16384, 512, 1
            self.assertLess(self.archive.stat().st_size, dist.MAX_BYTES)
            with self.assertRaisesRegex(dist.DistributionError, "Expanded distribution"):
                dist.verify(self.archive)
        finally:
            dist.MAX_BYTES, dist.MAX_MANIFEST, dist.MAX_FILES = previous

    def test_case_and_unicode_aliases_cannot_describe_two_output_files(self):
        self.create()
        original = self.members()
        for names in [("NAME", "name"), ("café", "cafe\u0301")]:
            self.replace_archive(original + [("fixture-0.1.0/" + name, b"alias") for name in names])
            with self.assertRaisesRegex(dist.DistributionError, "colliding paths"):
                dist.verify(self.archive)

    def test_bundled_corresponding_source_is_the_exact_locked_registry_package(self):
        project = Path(__file__).resolve().parents[1]
        directory = project / "resources/licenses"
        inventory = json.loads((directory / "source-provenance.json").read_text(encoding="utf-8"))
        lock = tomllib.loads((project / "Cargo.lock").read_text(encoding="utf-8"))
        policy = tomllib.loads((project / "deny.toml").read_text(encoding="utf-8"))
        self.assertEqual(inventory["format_version"], 1)
        self.assertTrue(inventory["sources"])
        required = {item["crate"] for item in policy["licenses"]["exceptions"] if "MPL-2.0" in item["allow"]}
        provided = {item["name"] + "@" + item["version"] for item in inventory["sources"] if item["license"] == "MPL-2.0"}
        self.assertEqual(provided, required)
        for item in inventory["sources"]:
            with self.subTest(package=item["name"], version=item["version"]):
                matches = [p for p in lock["package"] if (p["name"], p["version"], p.get("source")) == (item["name"], item["version"], item["registry"])]
                self.assertEqual(len(matches), 1)
                path = directory / dist.safe_path(item["archive"])
                self.assertEqual(dist.digest(path.read_bytes()), matches[0]["checksum"])
                self.assertEqual(item["sha256"], matches[0]["checksum"])
                self.assertIs(item["modified"], False)
                with tarfile.open(path, "r:gz") as archive:
                    prefix = item["name"] + "-" + item["version"] + "/"
                    self.assertTrue({prefix + p for p in ["src/lib.rs", "Cargo.toml", "LICENSE.txt"]} <= set(archive.getnames()))
                    manifest = tomllib.loads(archive.extractfile(prefix + "Cargo.toml").read().decode("utf-8"))
                    self.assertEqual(manifest["package"]["license"], item["license"])
                    original = archive.extractfile(prefix + "LICENSE.txt").read()
                    self.assertEqual((directory / dist.safe_path(item["license_file"])).read_bytes(), original)
                    self.assertEqual(dist.digest(original), item["license_sha256"])


if __name__ == "__main__":
    unittest.main()

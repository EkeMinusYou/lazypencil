"""Exercise source provenance and publication checks with original tiny crates."""

import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

import license_notices as notices


class NoticesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.cargo = self.root / "cargo"
        self.cache = self.cargo / "registry/cache/index.crates.io-test"
        self.cache.mkdir(parents=True)
        (self.root / "Cargo.toml").write_text('[package]\nname="fixture"\nversion="0.1.0"\nlicense="MIT"\n')
        (self.root / "LICENSE").write_text("Original fixture license; Copyright 2026 Fixture contributors.\n")
        self.upstream = self.root / "inputs/upstream.json"
        self.upstream.parent.mkdir()
        self.policy = {"format_version": 1, "packages": [], "license_references": []}
        self.members = {
            "Cargo.toml": b'[package]\nname="dependency"\nversion="1.0.0"\nlicense="MIT"\nauthors=["Published Author"]\nrepository="https://github.com/example/dependency"\n',
            "LICENSE": b"Original dependency grant. Copyright 2025 Original Holder.\n",
            "vendor/NOTICE": b"Original subcomponent notice.\n",
            "src/copying.rs": b'// Copyright 2025 Source Holder\nconst FALSE_NOTICE: &str = "Copyright 1900 Not a notice";\nfn code_body() {}\n',
        }
        self.archive = self.cache / "dependency-1.0.0.crate"
        self.prepare()

    def prepare(self, extra=None):
        with tarfile.open(self.archive, "w:gz") as stream:
            for path, data in self.members.items():
                entry = tarfile.TarInfo("dependency-1.0.0/" + path)
                entry.size = len(data)
                stream.addfile(entry, io.BytesIO(data))
            if extra:
                stream.addfile(extra)
        self.checksum = hashlib.sha256(self.archive.read_bytes()).hexdigest()
        (self.root / "Cargo.lock").write_text('version=4\n[[package]]\nname="fixture"\nversion="0.1.0"\n[[package]]\nname="dependency"\nversion="1.0.0"\nsource="registry+https://github.com/rust-lang/crates.io-index"\nchecksum="' + self.checksum + '"\n')
        self.upstream.write_text(json.dumps(self.policy))

    def collect(self, name="bundle"):
        path = self.root / name
        notices.collect(self.root, self.cargo, self.upstream, path)
        return path

    def test_original_notices_authors_and_source_comments_are_preserved_without_code(self):
        bundle = self.collect()
        result = notices.verify(self.root, bundle, self.cargo)
        self.assertTrue(result["source_archives_verified"])
        self.assertEqual(result["rust_packages"], 2)
        index = json.loads((bundle / "index.json").read_text())
        package = next(p for p in index["packages"] if p["name"] == "dependency")
        self.assertEqual({f["path"] for f in package["files"]}, {"LICENSE", "vendor/NOTICE"})
        text = (bundle / "RUST-NOTICES.txt").read_text()
        for value in ["Original Holder", "Original subcomponent", "Published Author", "Source Holder"]:
            self.assertIn(value, text)
        self.assertNotIn("code_body", text)
        self.assertNotIn("Not a notice", text)
        self.assertNotIn(str(self.root), text)

    def test_collection_is_reproducible_and_never_overwrites_existing_output(self):
        a, b = self.collect("a"), self.collect("b")
        self.assertEqual({str(p.relative_to(a)): p.read_bytes() for p in a.rglob("*") if p.is_file()},
                         {str(p.relative_to(b)): p.read_bytes() for p in b.rglob("*") if p.is_file()})
        with self.assertRaisesRegex(notices.NoticeError, "already exist"):
            notices.collect(self.root, self.cargo, self.upstream, a)

    def test_original_crlf_notice_bytes_survive_readable_text_normalization(self):
        original = b"Original grant.  \r\nCopyright 2025 Original Holder.\r\n"
        self.members["LICENSE"] = original
        self.prepare()
        bundle = self.collect()
        self.assertEqual((bundle / "texts" / (notices.digest(original) + ".txt")).read_bytes(), original)
        readable = (bundle / "RUST-NOTICES.txt").read_bytes()
        self.assertNotIn(b"\r", readable)
        self.assertIn(b"Original grant.\nCopyright 2025 Original Holder.", readable)

    def test_custom_declared_license_file_is_captured(self):
        del self.members["LICENSE"]
        self.members["Cargo.toml"] += b'license-file="legal/TERMS"\n'
        self.members["legal/TERMS"] = b"The declared original terms.\n"
        self.prepare()
        bundle = self.collect()
        self.assertIn("The declared original terms", (bundle / "RUST-NOTICES.txt").read_text())

    def test_missing_package_notice_does_not_become_a_generic_template(self):
        del self.members["LICENSE"]
        self.prepare()
        with self.assertRaisesRegex(notices.NoticeError, "Review omitted upstream"):
            self.collect()
        self.assertFalse((self.root / "bundle").exists())

    def test_changed_archive_is_rejected_before_reading_its_contents(self):
        self.archive.write_bytes(self.archive.read_bytes() + b"changed")
        with self.assertRaisesRegex(notices.NoticeError, "locked crate archive"):
            self.collect()

    def test_escaping_and_linked_notice_members_are_rejected(self):
        for name, kind in [("dependency-1.0.0/../../LICENSE", tarfile.REGTYPE),
                           ("dependency-1.0.0/NOTICE", tarfile.SYMTYPE)]:
            entry = tarfile.TarInfo(name)
            entry.type = kind
            entry.linkname = "../../LICENSE"
            self.prepare(entry)
            with self.assertRaises(notices.NoticeError):
                self.collect()

    def test_changed_lock_or_local_notice_invalidates_inventory(self):
        bundle = self.collect()
        lock = self.root / "Cargo.lock"
        original_lock = lock.read_bytes()
        lock.write_bytes(original_lock + b"# changed\n")
        with self.assertRaisesRegex(notices.NoticeError, "stale"):
            notices.verify(self.root, bundle)
        # Restore the exact input: recreating gzip changes its timestamp and
        # therefore the locked archive checksum when the clock crosses a second.
        lock.write_bytes(original_lock)
        (self.root / "LICENSE").write_text("Different local notice\n")
        with self.assertRaisesRegex(notices.NoticeError, "Workspace notice changed"):
            notices.verify(self.root, bundle)

    def test_modified_missing_and_unlisted_bundle_files_are_rejected(self):
        for case in ["modified", "missing", "unlisted"]:
            bundle = self.collect(case)
            if case == "unlisted":
                (bundle / "unexpected.txt").write_text("Unexpected")
            else:
                path = next((bundle / "texts").iterdir())
                if case == "missing":
                    path.unlink()
                else:
                    path.write_bytes(path.read_bytes() + b"tampered")
            with self.assertRaises(notices.NoticeError):
                notices.verify(self.root, bundle)

    def test_source_check_rejects_self_consistent_forged_package_metadata(self):
        bundle = self.collect()
        index = json.loads((bundle / "index.json").read_text())
        next(p for p in index["packages"] if p["name"] == "dependency")["authors"] = ["Not the published author"]
        (bundle / "index.json").write_bytes(notices.json_bytes(index))
        texts = {p.stem: p.read_bytes() for p in (bundle / "texts").iterdir()}
        (bundle / "RUST-NOTICES.txt").write_bytes(notices.render(index, texts))
        manifest = json.loads((bundle / "manifest.json").read_text())
        for entry in manifest["files"]:
            data = (bundle / entry["path"]).read_bytes()
            entry.update({"bytes": len(data), "sha256": notices.digest(data)})
        (bundle / "manifest.json").write_bytes(notices.json_bytes(manifest))
        notices.verify(self.root, bundle)
        with self.assertRaisesRegex(notices.NoticeError, "provenance differs"):
            notices.verify(self.root, bundle, self.cargo)

    def test_pinned_upstream_notice_is_bound_to_archive_revision_and_git_blob(self):
        del self.members["LICENSE"]
        revision = "1" * 40
        self.members[".cargo_vcs_info.json"] = json.dumps({"git": {"sha1": revision}}).encode()
        data = b"Original upstream grant. Copyright 2025 Upstream Holder.\n"
        value = notices.digest(data)
        (self.upstream.parent / "texts").mkdir()
        (self.upstream.parent / "texts" / (value + ".txt")).write_bytes(data)
        self.prepare()
        extra = {"name": "dependency", "version": "1.0.0", "archive_sha256": self.checksum,
                 "files": [{"path": "LICENSE", "url": "https://raw.githubusercontent.com/example/dependency/" + revision + "/LICENSE",
                            "revision": revision, "git_blob_sha1": hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest(),
                            "sha256": value, "blob": "texts/" + value + ".txt"}]}
        self.policy["packages"] = [extra]
        self.upstream.write_text(json.dumps(self.policy))
        bundle = self.collect()
        self.assertIn("Upstream Holder", (bundle / "RUST-NOTICES.txt").read_text())
        notices.verify(self.root, bundle, self.cargo)
        extra["files"][0]["revision"] = "2" * 40
        self.upstream.write_text(json.dumps(self.policy))
        with self.assertRaisesRegex(notices.NoticeError, "revision"):
            self.collect("wrong-revision")
        extra["files"][0]["revision"] = revision
        extra["files"][0]["git_blob_sha1"] = "0" * 40
        self.upstream.write_text(json.dumps(self.policy))
        with self.assertRaisesRegex(notices.NoticeError, "Git blob"):
            self.collect("wrong-blob")

    @unittest.skipUnless(hasattr(__import__("os"), "symlink"), "Requires symlinks")
    def test_notice_directory_symlink_cannot_redirect_verification(self):
        bundle = self.collect()
        texts = bundle / "texts"
        moved = bundle / "original-texts"
        texts.rename(moved)
        try:
            texts.symlink_to(moved, target_is_directory=True)
        except OSError as error:
            self.skipTest(str(error))
        with self.assertRaisesRegex(notices.NoticeError, "symlink"):
            notices.verify(self.root, bundle)


if __name__ == "__main__":
    unittest.main()

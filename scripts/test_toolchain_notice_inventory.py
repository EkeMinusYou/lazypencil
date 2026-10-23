"""Exercise standard-library provenance independently of the host Rust install."""

import hashlib
import io
import json
from pathlib import Path
import shutil
import tempfile
import tarfile
import unittest

import license_notices as notices
import toolchain_notice_inventory as toolchain


class ToolchainInventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source, self.legal, self.docs, self.libs = [self.root / name for name in ("source", "legal", "docs", "libs")]
        for path in (self.source / "library/vendor/dependency-1.0.0", self.legal, self.docs / "licenses", self.libs):
            path.mkdir(parents=True)
        self.library = self.source / "library"
        self.package = self.library / "vendor/dependency-1.0.0"
        (self.library / "Cargo.toml").write_text('[workspace]\nmembers=[]\n')
        (self.library / "Cargo.lock").write_text('version=4\n[[package]]\nname="dependency"\nversion="1.0.0"\nsource="registry+https://github.com/rust-lang/crates.io-index"\nchecksum="' + "a" * 64 + '"\n')
        (self.package / "Cargo.toml").write_text('[package]\nname="dependency"\nversion="1.0.0"\nlicense="MIT"\nauthors=["Declared Publisher"]\n')
        (self.package / "LICENSE").write_bytes(b"Original dependency license. Copyright 2025 Original Holder.\r\n")
        (self.library / "core.rs").write_text('// Copyright 2026 Source Holder\nfn original_code() {}\n')
        for name in ("COPYRIGHT", "LICENSE-MIT", "LICENSE-APACHE"):
            (self.legal / name).write_text("Original distribution " + name)
        (self.docs / "COPYRIGHT-library.html").write_text('<b>License:</b> MIT\n<b>License:</b> Unicode-3.0\n')
        for name in ("MIT", "Unicode-3.0"):
            (self.docs / "licenses" / (name + ".txt")).write_text("Original reference terms for " + name)
        (self.libs / "libstd-target.rlib").write_bytes(b"Original installed std library")
        self.identity = {"release": "1.99.1", "target": "fixture-target", "commit_hash": "b" * 40}
        self.checksums()

    def checksums(self):
        data = {"package": getattr(self, "archive_checksum", "a" * 64), "files": {p.relative_to(self.package).as_posix(): notices.digest(p.read_bytes())
                for p in self.package.rglob("*") if p.is_file() and p.name != ".cargo-checksum.json"}}
        (self.package / ".cargo-checksum.json").write_bytes(notices.json_bytes(data))

    def inventory(self):
        return toolchain.build_inventory(self.source, self.legal, self.docs, self.libs, self.identity)

    def files(self):
        return toolchain.bundle_files(*self.inventory())

    def prepare_registry(self):
        self.cargo = self.root / "cargo"
        self.archive = self.cargo / "registry/cache/test/dependency-1.0.0.crate"
        self.archive.parent.mkdir(parents=True, exist_ok=True)
        with tarfile.open(self.archive, "w:gz") as stream:
            for path in sorted(self.package.rglob("*")):
                if not path.is_file() or path.name == ".cargo-checksum.json":
                    continue
                data = path.read_bytes()
                entry = tarfile.TarInfo("dependency-1.0.0/" + path.relative_to(self.package).as_posix())
                entry.size = len(data)
                stream.addfile(entry, io.BytesIO(data))
        old = getattr(self, "archive_checksum", "a" * 64)
        self.archive_checksum = notices.digest(self.archive.read_bytes())
        lock = self.library / "Cargo.lock"
        lock.write_text(lock.read_text().replace(old, self.archive_checksum))
        self.checksums()

    def registry_inventory(self, upstream=None):
        return toolchain.build_inventory(self.source, self.legal, self.docs, self.libs, self.identity, self.cargo, upstream)

    def test_registry_archives_support_a_source_tree_without_vendor(self):
        self.prepare_registry()
        shutil.rmtree(self.library / "vendor")
        index, texts = self.registry_inventory()
        self.assertTrue(index["registry_archives_verified"])
        self.assertFalse(index["vendor_file_checksums_verified"])
        self.assertEqual(index["registry_packages"][0]["original_source_hashes"]["LICENSE"], notices.digest(b"Original dependency license. Copyright 2025 Original Holder.\r\n"))
        self.assertEqual(index["review_findings"], [])
        self.assertIn("Original Holder", toolchain.bundle_files(index, texts)["TOOLCHAIN-NOTICES.txt"].decode())
        with self.assertRaisesRegex(notices.NoticeError, "provide --cargo-home"):
            self.inventory()

    def test_original_archive_checksums_and_missing_cache_fail_without_fallback(self):
        self.prepare_registry()
        self.archive.write_bytes(b"Changed archive")
        with self.assertRaisesRegex(notices.NoticeError, "locked crate archive"):
            self.registry_inventory()
        self.archive.unlink()
        with self.assertRaisesRegex(notices.NoticeError, "locked crate archive"):
            self.registry_inventory()

    def test_vendor_differences_cannot_masquerade_as_original_registry_sources(self):
        self.prepare_registry()
        (self.package / "LICENSE").write_text("Changed installed vendor notice")
        self.checksums()
        index, texts = self.registry_inventory()
        comparison = index["registry_packages"][0]["vendor_registry_comparison"]
        self.assertFalse(comparison["matches"])
        self.assertEqual(comparison["changed_files"], ["LICENSE"])
        self.assertTrue(index["registry_archives_verified"])
        self.assertTrue(any("differ from" in f for f in index["review_findings"]))
        text = toolchain.bundle_files(index, texts)["TOOLCHAIN-NOTICES.txt"].decode()
        self.assertIn("Original Holder", text)
        self.assertIn("Changed installed vendor notice", text)

    def test_partial_vendor_tree_is_not_silently_replaced_with_registry_content(self):
        self.prepare_registry()
        (self.package / ".cargo-checksum.json").unlink()
        with self.assertRaisesRegex(notices.NoticeError, "Missing standard-library vendor"):
            self.registry_inventory()

    def test_mpl_corresponding_source_is_the_complete_original_archive(self):
        manifest = self.package / "Cargo.toml"
        manifest.write_text(manifest.read_text().replace('license="MIT"', 'license="MPL-2.0"'))
        self.prepare_registry()
        index, texts = self.registry_inventory()
        files = toolchain.bundle_files(index, texts)
        source = index["corresponding_sources"][0]
        self.assertEqual(files[source["path"]], self.archive.read_bytes())
        self.assertEqual(source["sha256"], self.archive_checksum)
        self.assertIn(source["path"], files["TOOLCHAIN-NOTICES.txt"].decode())
        self.assertNotIn(b"\x1f\x8b", files["TOOLCHAIN-NOTICES.txt"])
        output = self.root / "with-source"
        toolchain.publish(files, output)
        toolchain.verify(files, output)

    def test_registry_workspace_terms_bind_original_archive_and_dirty_vcs_metadata(self):
        manifest = self.package / "Cargo.toml"
        manifest.write_text(manifest.read_text() + 'repository="https://github.com/example/upstream"\n')
        (self.package / "LICENSE").unlink()
        revision = "c" * 40
        (self.package / ".cargo_vcs_info.json").write_bytes(notices.json_bytes({"git": {"sha1": revision, "dirty": True}}))
        self.prepare_registry()
        root = self.root / "upstream"
        (root / "texts").mkdir(parents=True)
        data = b"Original workspace license. Copyright 2025 Workspace Holder.\n"
        sha = notices.digest(data)
        (root / "texts" / (sha + ".txt")).write_bytes(data)
        item = {"path": "LICENSE", "url": "https://raw.githubusercontent.com/example/upstream/" + revision + "/LICENSE",
                "revision": revision, "sha256": sha, "blob": "texts/" + sha + ".txt",
                "git_blob_sha1": hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()}
        policy = {"format_version": 1, "packages": [{"name": "dependency", "version": "1.0.0", "archive_sha256": self.archive_checksum, "files": [item]}]}
        path = root / "upstream.json"
        path.write_bytes(notices.json_bytes(policy))
        index, texts = self.registry_inventory(path)
        self.assertTrue(any("dirty=true" in f for f in index["review_findings"]))
        self.assertFalse(any("No standalone" in f for f in index["review_findings"]))
        self.assertIn("Workspace Holder", toolchain.bundle_files(index, texts)["TOOLCHAIN-NOTICES.txt"].decode())
        policy["packages"][0]["archive_sha256"] = "0" * 64
        path.write_bytes(notices.json_bytes(policy))
        with self.assertRaisesRegex(notices.NoticeError, "another archive"):
            self.registry_inventory(path)

    def test_upstream_terms_require_original_registry_applicability_checks(self):
        with self.assertRaisesRegex(notices.NoticeError, "requires --cargo-home"):
            toolchain.build_inventory(self.source, self.legal, self.docs, self.libs, self.identity, upstream=self.root / "unused")

    def canonical_reference(self):
        root = self.root / "canonical"
        (root / "texts").mkdir(parents=True)
        data = b"Canonical Unicode reference terms; Copyright <year> <holder>.\r\n"
        sha, revision = notices.digest(data), "d" * 40
        (root / "texts" / (sha + ".txt")).write_bytes(data)
        item = {"id": "Unicode-3.0", "kind": "reference_terms_only", "repository": "https://github.com/spdx/license-list-data",
                "revision": revision, "path": "text/Unicode-3.0.txt", "sha256": sha, "blob": "texts/" + sha + ".txt",
                "url": "https://raw.githubusercontent.com/spdx/license-list-data/" + revision + "/text/Unicode-3.0.txt",
                "git_blob_sha1": hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()}
        policy = root / "upstream.json"
        policy.write_bytes(notices.json_bytes({"format_version": 1, "packages": [], "license_references": [item]}))
        return policy, item, data

    def test_missing_distribution_terms_can_use_pinned_reference_without_inventing_copyright(self):
        self.prepare_registry()
        (self.docs / "licenses/Unicode-3.0.txt").unlink()
        policy, reference, data = self.canonical_reference()
        index, texts = self.registry_inventory(policy)
        item = next(p for p in index["notice_files"] if p["path"].endswith("licenses/Unicode-3.0.txt"))
        self.assertEqual(item["kind"], "referenced_terms_upstream")
        self.assertEqual(item["provenance"], reference)
        self.assertEqual(texts[item["sha256"]], data)
        rendered = toolchain.bundle_files(index, texts)["TOOLCHAIN-NOTICES.txt"].decode()
        self.assertIn("Copyright <year> <holder>", rendered)
        self.assertIn("Original Holder", rendered)
        self.assertIn("Reference terms only; original package/source copyright notices are retained separately.", rendered)

    def test_reference_terms_require_the_exact_identifier_repository_and_git_blob(self):
        policy, _, _ = self.canonical_reference()
        self.assertEqual(toolchain.upstream_policy(policy), {})
        data = json.loads(policy.read_text())
        data["license_references"][0]["git_blob_sha1"] = "0" * 40
        policy.write_bytes(notices.json_bytes(data))
        with self.assertRaisesRegex(notices.NoticeError, "Git blob"):
            toolchain.reference_terms(policy)
        data["license_references"][0]["repository"] = "https://example.invalid/licenses"
        policy.write_bytes(notices.json_bytes(data))
        with self.assertRaisesRegex(notices.NoticeError, "provenance mismatch"):
            toolchain.reference_terms(policy)

    def test_registry_source_links_are_rejected_before_corresponding_source_publication(self):
        self.prepare_registry()
        with tarfile.open(self.archive, "r:gz") as stream:
            members = [(member, stream.extractfile(member).read()) for member in stream if member.isfile()]
        with tarfile.open(self.archive, "w:gz") as stream:
            for member, data in members:
                stream.addfile(member, io.BytesIO(data))
            linked = tarfile.TarInfo("dependency-1.0.0/src/linked.rs")
            linked.type, linked.linkname = tarfile.SYMTYPE, "../../outside"
            stream.addfile(linked)
        original, self.archive_checksum = self.archive_checksum, notices.digest(self.archive.read_bytes())
        lock = self.library / "Cargo.lock"
        lock.write_text(lock.read_text().replace(original, self.archive_checksum))
        self.checksums()
        with self.assertRaisesRegex(notices.NoticeError, "Linked source member"):
            self.registry_inventory()

    def test_original_notices_and_source_attributions_are_distinct_from_declared_authors(self):
        index, texts = self.inventory()
        self.assertTrue(index["vendor_file_checksums_verified"])
        self.assertFalse(index["registry_archives_verified"])
        self.assertEqual(index["vendor_packages"][0]["authors"], ["Declared Publisher"])
        self.assertEqual(index["review_findings"], [])
        text = toolchain.bundle_files(index, texts)["TOOLCHAIN-NOTICES.txt"].decode()
        for value in ("Original Holder", "Source Holder", "Original reference terms for Unicode-3.0"):
            self.assertIn(value, text)
        self.assertNotIn("fn original_code", text)
        self.assertNotIn(str(self.root), text)
        self.assertIn(b"Original dependency license. Copyright 2025 Original Holder.\r\n", texts.values())

    def test_missing_or_modified_vendor_files_cannot_pass_with_a_lock_checksum_alone(self):
        path = self.package / "LICENSE"
        path.write_text("Changed source license")
        with self.assertRaisesRegex(notices.NoticeError, "Changed standard-library vendor file"):
            self.inventory()
        path.unlink()
        with self.assertRaisesRegex(notices.NoticeError, "Missing or extra"):
            self.inventory()

    def test_manifest_identity_and_archive_declaration_are_verified(self):
        path = self.package / "Cargo.toml"
        original = path.read_text()
        path.write_text(original.replace('version="1.0.0"', 'version="2.0.0"'))
        self.checksums()
        with self.assertRaisesRegex(notices.NoticeError, "identity mismatch"):
            self.inventory()
        path.write_text(original)
        self.checksums()
        checksum = self.package / ".cargo-checksum.json"
        data = json.loads(checksum.read_text());data["package"] = "0" * 64
        checksum.write_bytes(notices.json_bytes(data))
        with self.assertRaisesRegex(notices.NoticeError, "differs from"):
            self.inventory()

    def test_malformed_vendor_file_metadata_is_a_validation_error(self):
        path = self.package / ".cargo-checksum.json"
        data = json.loads(path.read_text())
        data["files"] = ["LICENSE"]
        path.write_bytes(notices.json_bytes(data))
        with self.assertRaisesRegex(notices.NoticeError, "Invalid vendor checksum metadata"):
            self.inventory()

    def test_missing_notices_are_reported_without_inventing_a_copyright_holder(self):
        (self.package / "LICENSE").unlink()
        self.checksums()
        index, texts = self.inventory()
        self.assertIn("No standalone notice", index["review_findings"][0])
        self.assertIn("license=\"MIT\"", toolchain.bundle_files(index, texts)["TOOLCHAIN-NOTICES.txt"].decode())

    def test_missing_target_library_or_reference_terms_fail_explicitly(self):
        (self.libs / "libstd-target.rlib").unlink()
        with self.assertRaisesRegex(notices.NoticeError, "No installed target"):
            self.inventory()
        (self.libs / "libstd-target.rlib").write_bytes(b"Installed std")
        (self.docs / "licenses/Unicode-3.0.txt").unlink()
        with self.assertRaisesRegex(notices.NoticeError, "regular notice input"):
            self.inventory()

    def test_bundle_reproducibility_no_overwrite_and_live_source_reverification(self):
        first, second = self.files(), self.files()
        self.assertEqual(first, second)
        output = self.root / "output"
        toolchain.publish(first, output)
        toolchain.verify(second, output)
        with self.assertRaisesRegex(notices.NoticeError, "already exist"):
            toolchain.publish(second, output)
        (self.library / "core.rs").write_text("// Copyright Changed Source\n")
        with self.assertRaisesRegex(notices.NoticeError, "Changed or stale"):
            toolchain.verify(self.files(), output)

    def test_target_library_change_and_extra_bundle_file_are_detected(self):
        output = self.root / "output"
        toolchain.publish(self.files(), output)
        (self.libs / "libstd-target.rlib").write_bytes(b"A different std library")
        with self.assertRaisesRegex(notices.NoticeError, "Changed or stale"):
            toolchain.verify(self.files(), output)
        extra = output / "extra"
        extra.write_bytes(b"Unexpected")
        with self.assertRaisesRegex(notices.NoticeError, "Missing or extra"):
            toolchain.verify(self.files(), output)

    def test_source_link_cannot_substitute_external_notice_bytes(self):
        path = self.library / "redirect.rs"
        try:
            path.symlink_to(self.legal / "LICENSE-MIT")
        except OSError as error:
            self.skipTest(str(error))
        with self.assertRaisesRegex(notices.NoticeError, "must not be a link"):
            self.inventory()


if __name__ == "__main__":
    unittest.main()

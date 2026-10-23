"""Reject stale, altered or foreign-compiler standard-library attachments."""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import license_notices as notices
import toolchain_build_binding as binding


class ToolchainBindingTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root, self.output, self.libraries = (self.base / name for name in ("input", "output", "libraries"))
        (self.root / "inventory").mkdir(parents=True)
        self.libraries.mkdir()
        (self.libraries / "libstd-original.rlib").write_bytes(b"actual installed standard library fixture")
        self.compiler = "rustc pinned fixture\nhost: x86_64-unknown-linux-gnu\n"
        self.identity = {"release": binding.RELEASE, "commit_hash": binding.COMMIT,
                         "target": "x86_64-unknown-linux-gnu", "verbose": self.compiler.strip()}
        self.index = {"format_version": 1, "observed_toolchain": self.identity,
                      "target_libraries": binding.toolchain.library_fingerprints(self.libraries),
                      "registry_archives_verified": True, "registry_packages": [{"name": "original"}],
                      "review_findings": ["Preserve original publisher finding"]}
        self.distribution = {"format_version": 1, "release_ready": False, "observed_toolchain": self.identity,
                             "source_url": binding.SOURCE_URL, "source_sha256": binding.SOURCE_SHA}
        self.refresh()
        self.observer = patch.object(binding.toolchain, "observe", return_value=(self.identity, self.base, self.libraries))
        self.observer.start()
        self.addCleanup(self.observer.stop)

    def refresh(self):
        (self.root / "source-distribution.json").write_bytes(notices.json_bytes(self.distribution))
        files = {"inventory.json": notices.json_bytes(self.index), "TOOLCHAIN-NOTICES.txt": b"Original notices\r\n"}
        for name, data in files.items():
            (self.root / "inventory" / name).write_bytes(data)
        self.manifest = {"format_version": 1, "files": [{"path": name, "bytes": len(data), "sha256": notices.digest(data)}
                                                        for name, data in sorted(files.items())]}
        (self.root / "inventory/manifest.json").write_bytes(notices.json_bytes(self.manifest))

    def validate(self):
        return binding.validate(self.root, self.compiler)

    def test_matching_compiler_library_bytes_and_original_findings_are_retained(self):
        result = binding.attach(self.root, self.output, self.compiler)
        self.assertEqual(result["target_libraries"], 1)
        self.assertEqual(result["review_findings"], self.index["review_findings"])
        self.assertEqual((self.output / "inventory/TOOLCHAIN-NOTICES.txt").read_bytes(), b"Original notices\r\n")
        self.assertEqual(binding.validate(self.output, self.compiler), result)

    def test_wrong_compiler_and_target_are_rejected(self):
        with self.assertRaises(notices.NoticeError):
            binding.validate(self.root, self.compiler + "foreign compiler")
        self.index["observed_toolchain"] = {**self.identity, "target": "aarch64-apple-darwin"}
        self.refresh()
        with self.assertRaises(notices.NoticeError):
            self.validate()

    def test_same_version_but_changed_installed_library_bytes_are_rejected(self):
        (self.libraries / "libstd-original.rlib").write_bytes(b"different installed library")
        with self.assertRaisesRegex(notices.NoticeError, "Installed target libraries"):
            self.validate()

    def test_foreign_distribution_and_unverified_archives_are_rejected(self):
        self.distribution["source_sha256"] = "0" * 64
        self.refresh()
        with self.assertRaises(notices.NoticeError):
            self.validate()
        self.distribution["source_sha256"] = binding.SOURCE_SHA
        self.index["registry_archives_verified"] = False
        self.refresh()
        with self.assertRaises(notices.NoticeError):
            self.validate()

    def test_missing_altered_and_extra_notice_members_are_rejected(self):
        path = self.root / "inventory/TOOLCHAIN-NOTICES.txt"
        path.write_bytes(b"substitute")
        with self.assertRaises(notices.NoticeError):
            self.validate()
        self.refresh()
        path.unlink()
        self.manifest["files"] = [v for v in self.manifest["files"] if v["path"] != "TOOLCHAIN-NOTICES.txt"]
        (self.root / "inventory/manifest.json").write_bytes(notices.json_bytes(self.manifest))
        with self.assertRaises(notices.NoticeError):
            self.validate()
        self.refresh()
        (self.root / "inventory/unlisted").write_bytes(b"extra")
        with self.assertRaises(notices.NoticeError):
            self.validate()

    def test_existing_attachment_is_not_overwritten(self):
        self.output.mkdir()
        (self.output / "keep").write_bytes(b"preserved")
        with self.assertRaises(notices.NoticeError):
            binding.attach(self.root, self.output, self.compiler)
        self.assertEqual((self.output / "keep").read_bytes(), b"preserved")


if __name__ == "__main__":
    unittest.main()

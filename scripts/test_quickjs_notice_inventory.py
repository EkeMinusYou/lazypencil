"""Bind copied native inputs and notice candidates to original archive bytes."""

import io
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import license_notices as notices
import quickjs_notice_inventory as quick


class QuickJSInventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.source, self.out, self.cargo = (self.root / name for name in ("source", "out", "cargo"))
        self.source.mkdir()
        self.out.mkdir()
        self.target = "aarch64-apple-darwin"
        self.files = {"Cargo.toml": b"[package]\nname='rquickjs-sys'\nversion='0.14.0'\nlicense='MIT'\nrepository='https://example.test/source'\n",
                      "build.rs": b"reviewed native-copy build program\n",
                      "quickjs/LICENSE": b"Original MIT grant. Copyright Original QuickJS Holder.\n",
                      "src/bindings/" + self.target + ".rs": b"// Copyright original binding holder\n"}
        for original, copied in quick.COPIED:
            self.files[original] = ("/* Copyright original holder for " + copied + " */\nint source;\n").encode()
        self.files["quickjs/quickjs.c"] += b" " * 70000 + b"\n/* Copyright Late Holder. License: retained in full. */\n"
        for name, data in self.files.items():
            path = self.source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        for original, copied in quick.COPIED:
            (self.out / copied).write_bytes(self.files[original])
        self.generated = self.out / "bindings.rs"
        self.generated.write_text('macro_rules! bindings_env { ("TARGET") => { "' + self.target + '" }; }')
        self.archive = self.cargo / "registry/cache/pinned/rquickjs-sys-0.14.0.crate"
        self.archive.parent.mkdir(parents=True)
        with tarfile.open(self.archive, "w:gz") as stream:
            for name, data in self.files.items():
                member = tarfile.TarInfo("rquickjs-sys-0.14.0/" + name)
                member.size = len(data)
                stream.addfile(member, io.BytesIO(data))
        self.sha = notices.digest(self.archive.read_bytes())
        self.build_sha = notices.digest(self.files["build.rs"])
        (self.root / "Cargo.lock").write_text('version=4\n[[package]]\nname="rquickjs-sys"\nversion="0.14.0"\nsource="registry+https://github.com/rust-lang/crates.io-index"\nchecksum="' + self.sha + '"\n')
        self.output = self.root / "inventory"
        self.features = ["std"]
        self.addCleanup(patch.stopall)
        patch.object(quick, "ARCHIVE_SHA", self.sha).start()
        patch.object(quick, "BUILD_SHA", self.build_sha).start()

    def arguments(self):
        return (self.root, self.source / "Cargo.toml", self.out, self.features, self.cargo, self.target)

    def inventory(self):
        return quick.build_inventory(*self.arguments())

    def test_archive_binding_retains_original_grant_and_late_source_comment(self):
        index, texts = self.inventory()
        self.assertEqual(len(index["inputs"]), 19)
        self.assertEqual(sum(v["role"] == "compiled_source" for v in index["inputs"]), 4)
        self.assertEqual(index["package"]["archive_sha256"], self.sha)
        data = quick.bundle_files(index, texts)["QUICKJS-NOTICES.txt"]
        self.assertIn(b"Original MIT grant.", data)
        self.assertIn(b"Copyright Late Holder. License: retained in full.", data)
        self.assertNotIn(b"int source;", data)

    def test_copied_source_original_source_manifest_and_build_script_changes_are_rejected(self):
        for path in (self.out / "quickjs.h", self.source / "quickjs/dtoa.c",
                     self.source / "Cargo.toml", self.source / "build.rs"):
            with self.subTest(path=path):
                data = path.read_bytes()
                path.write_bytes(data + b"modified")
                with self.assertRaises(notices.NoticeError):
                    quick.collect(*self.arguments(), self.output)
                self.assertFalse(self.output.exists())
                path.write_bytes(data)

    def test_corrupt_registry_archive_is_rejected(self):
        self.archive.write_bytes(self.archive.read_bytes() + b"changed")
        with self.assertRaises(notices.NoticeError):
            self.inventory()

    def test_unreviewed_archive_build_script_features_and_target_are_rejected(self):
        with patch.object(quick, "ARCHIVE_SHA", "0" * 64), self.assertRaises(notices.NoticeError):
            self.inventory()
        with patch.object(quick, "BUILD_SHA", "0" * 64), self.assertRaises(notices.NoticeError):
            self.inventory()
        for features in (["std", "bindgen"], ["std", "update-bindings"]):
            self.features = features
            with self.assertRaises(notices.NoticeError):
                self.inventory()
        self.features = ["std"]
        self.target = "wasm32-unknown-unknown"
        with self.assertRaises(notices.NoticeError):
            self.inventory()

    def test_generated_target_mismatch_or_extra_generated_code_is_rejected(self):
        for text in ('macro_rules! bindings_env { ("TARGET") => { "x86_64-apple-darwin" }; }',
                     self.generated.read_text() + " fn unexpected() {}"):
            self.generated.write_text(text)
            with self.assertRaises(notices.NoticeError):
                self.inventory()

    def test_reconstruction_detects_modified_notice_and_existing_output_is_preserved(self):
        result = quick.collect(*self.arguments(), self.output)
        self.assertTrue(result["registry_archive_verified"])
        quick.verify(*self.arguments(), self.output)
        with self.assertRaises(notices.NoticeError):
            quick.collect(*self.arguments(), self.output)
        (self.output / "QUICKJS-NOTICES.txt").write_bytes(b"a substituted license")
        with self.assertRaises(notices.NoticeError):
            quick.verify(*self.arguments(), self.output)


if __name__ == "__main__":
    unittest.main()

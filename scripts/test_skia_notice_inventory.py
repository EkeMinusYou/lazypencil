"""Check native source evidence and deliberately incomplete license inventories."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import license_notices as notices
import skia_notice_inventory as native


class SkiaInventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "source"
        self.build = self.root / "build"
        self.output = self.root / "inventory"
        self.source.mkdir()
        self.build.mkdir()
        for name in ("LICENSE", "AUTHORS", "DEPS"):
            (self.source / name).write_text("Original Skia " + name + "\n")
        self.c = self.source / "src/native.cc"
        self.c.parent.mkdir()
        self.c.write_text("/* Copyright 2026 Original Skia Holder. */\nint native_code;\n")
        self.external = self.source / "third_party/externals/zlib"
        self.external.mkdir(parents=True)
        (self.external / "LICENSE").write_bytes(b"Original zlib grant.\r\n")
        self.h = self.external / "native.h"
        self.h.write_text("// Copyright 2026 Header Holder\n// Chromium source repository LICENSE file.\nint declaration;\n")
        for name in ("args.gn", "build.ninja", "toolchain.ninja"):
            (self.build / name).write_text("Original build evidence " + name)
        (self.build / "libnative.a").write_bytes(b"Original static library")
        self.graph = "../source/src/native.cc\nlibnative.a\n"
        # The header is absent from the explicit source graph: its license must
        # still be found through the recorded dependency evidence.
        self.headers = "obj/native.o: #deps 2\n    ../source/third_party/externals/zlib/native.h\n    ../system.h\n\n"

    def query(self, build, arguments, ninja):
        return self.graph if arguments[0] == "inputs" else self.headers

    def inventory(self):
        return native.build_inventory(self.source, self.build, query_fn=self.query)

    def reference(self):
        directory = self.root / "reference"
        directory.mkdir(exist_ok=True)
        data = b"Original Chromium reference terms. Copyright 2015 Chromium Authors.\r\n"
        sha = notices.digest(data)
        (directory / "texts").mkdir(exist_ok=True)
        (directory / "texts" / (sha + ".txt")).write_bytes(data)
        item = {"id": "chromium-bsd", "repository": "https://chromium.googlesource.com/chromium/src",
                "revision": "a" * 40, "path": "LICENSE", "sha256": sha, "blob": "texts/" + sha + ".txt",
                "url": "https://chromium.googlesource.com/chromium/src/+/" + "a" * 40 + "/LICENSE?format=TEXT",
                "git_blob_sha1": hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest(),
                "applies_to": {"skia_deps_sha256": notices.digest((self.source / "DEPS").read_bytes()),
                               "source_sha256": {self.h.relative_to(self.source).as_posix(): notices.digest(self.h.read_bytes())}}}
        policy = directory / "upstream.json"
        policy.write_bytes(notices.json_bytes({"format_version": 1, "references": [item]}))
        return policy

    def collect(self):
        original = native.build_inventory
        with patch.object(native, "build_inventory", side_effect=lambda s, b, n: original(s, b, n, self.query)):
            return native.collect(self.source, self.build, self.output)

    def verify(self):
        original = native.build_inventory
        with patch.object(native, "build_inventory", side_effect=lambda s, b, n: original(s, b, n, self.query)):
            return native.verify(self.source, self.build, self.output)

    def test_recorded_header_finds_original_notice_and_unresolved_reference(self):
        index, texts = self.inventory()
        self.assertEqual(index["components"], ["skia", "zlib"])
        self.assertEqual(len(index["inputs"]), 2)
        self.assertEqual(index["excluded_inputs"], {"system_or_external_headers": 1})
        self.assertEqual(len(index["review_findings"]), 1)
        record = next(f for f in index["notice_files"] if f["path"].endswith("zlib/LICENSE"))
        self.assertEqual(texts[record["sha256"]], b"Original zlib grant.\r\n")
        text = native.rendered(index, texts).decode()
        self.assertIn("Header Holder", text)
        self.assertNotIn("int native_code", text)
        self.assertNotIn("int declaration", text)
        self.assertNotIn(str(self.root), text)

    def test_pinned_reference_retains_original_source_attributions_separately(self):
        index, texts = self.inventory()
        native.apply_references(index, texts, self.reference())
        self.assertEqual(index["review_findings"], [])
        self.assertEqual(index["license_references"][0]["observed_source_paths"], [self.h.relative_to(self.source).as_posix()])
        text = native.rendered(index, texts).decode()
        self.assertIn("Copyright 2015 Chromium Authors.", text)
        self.assertIn("Copyright 2026 Header Holder", text)
        self.assertIn("Original zlib grant", text)

    def test_reference_does_not_apply_to_changed_deps_or_source(self):
        policy = self.reference()
        self.h.write_text(self.h.read_text() + "// Modified source\n")
        index, texts = self.inventory()
        with self.assertRaisesRegex(notices.NoticeError, "changed source"):
            native.apply_references(index, texts, policy)
        policy = self.reference()
        (self.source / "DEPS").write_text("A new Skia revision")
        index, texts = self.inventory()
        with self.assertRaisesRegex(notices.NoticeError, "another Skia DEPS"):
            native.apply_references(index, texts, policy)

    def test_reference_requires_the_original_bytes_and_git_blob(self):
        policy = self.reference()
        item = json.loads(policy.read_text())["references"][0]
        (policy.parent / item["blob"]).write_bytes(b"Modified terms")
        index, texts = self.inventory()
        with self.assertRaisesRegex(notices.NoticeError, "checksum"):
            native.apply_references(index, texts, policy)
        policy = self.reference()
        contents = json.loads(policy.read_text())
        contents["references"][0]["git_blob_sha1"] = "0" * 40
        policy.write_bytes(notices.json_bytes(contents))
        with self.assertRaisesRegex(notices.NoticeError, "Git blob"):
            native.apply_references(index, texts, policy)

    def test_reference_cannot_change_repository_or_commit_url(self):
        policy = self.reference()
        contents = json.loads(policy.read_text())
        contents["references"][0]["url"] = "https://example.invalid/terms"
        policy.write_bytes(notices.json_bytes(contents))
        index, texts = self.inventory()
        with self.assertRaisesRegex(notices.NoticeError, "repository/revision"):
            native.apply_references(index, texts, policy)

    def test_reference_source_bindings_cannot_escape_or_omit_their_checksums(self):
        policy = self.reference()
        contents = json.loads(policy.read_text())
        bindings = contents["references"][0]["applies_to"]["source_sha256"]
        bindings["../outside"] = "a" * 64
        policy.write_bytes(notices.json_bytes(contents))
        with self.assertRaisesRegex(notices.NoticeError, "Escaping archive path"):
            native.load_reference(policy)
        del bindings["../outside"]
        bindings[next(iter(bindings))] = "invalid-checksum"
        policy.write_bytes(notices.json_bytes(contents))
        with self.assertRaisesRegex(notices.NoticeError, "source binding"):
            native.load_reference(policy)

    def test_standalone_reference_check_verifies_bytes_without_a_native_build(self):
        policy = self.reference()
        reference, data = native.load_reference(policy)
        self.assertEqual(notices.digest(data), reference["sha256"])
        self.assertEqual(len(reference["applies_to"]["source_sha256"]), 1)

    def test_reference_mismatch_reports_observed_bytes_without_accepting_line_ending_changes(self):
        # write_text uses host newlines; pin the LF fixture bytes before making
        # a deliberate CRLF variant on every platform.
        original = self.h.read_bytes().replace(b"\r\n", b"\n")
        self.h.write_bytes(original)
        policy = self.reference()
        self.h.write_bytes(original.replace(b"\n", b"\r\n"))
        index, texts = self.inventory()
        with self.assertRaises(notices.NoticeError) as caught:
            native.apply_references(index, texts, policy)
        message = str(caught.exception)
        self.assertIn(self.h.relative_to(self.source).as_posix(), message)
        self.assertIn(notices.digest(original), message)
        self.assertIn(notices.digest(self.h.read_bytes()), message)
        self.assertIn('"crlf_pairs": 3', message)
        self.assertIn('"bare_cr": 0', message)

    def variant_reference(self):
        original = self.h.read_bytes().replace(b"\r\n", b"\n")
        self.h.write_bytes(original)
        revision = "a" * 40
        (self.source / "DEPS").write_bytes(('"third_party/externals/zlib": "' + native.ZLIB_SOURCE
                                          + "@" + revision + '",\n').encode())
        policy = self.reference()
        value = json.loads(policy.read_bytes())
        applies = value["references"][0]["applies_to"]
        path = self.h.relative_to(self.source).as_posix()
        actual = original.replace(b"\n", b"\r\n")
        variant = {"transform": "lf_to_crlf", "sha256": notices.digest(actual),
                   "source_git_blob_sha1": hashlib.sha1(b"blob " + str(len(original)).encode() + b"\0" + original).hexdigest(),
                   "url": native.ZLIB_SOURCE + "/+/" + revision + "/native.h?format=TEXT"}
        applies["source_origin"] = {"repository": native.ZLIB_SOURCE, "revision": revision}
        applies["working_tree_variants"] = {path: variant}
        self.h.write_bytes(actual)
        policy.write_bytes(notices.json_bytes(value))
        return policy, value, original, variant, path

    def test_reviewed_crlf_variant_records_actual_bytes_and_retains_original_pin(self):
        policy, value, original, variant, path = self.variant_reference()
        index, texts = self.inventory()
        native.apply_references(index, texts, policy)
        applied = index["license_references"][0]
        self.assertEqual(applied["observed_source_forms"][path],
                         {"form": "reviewed_lf_to_crlf", "sha256": notices.digest(self.h.read_bytes())})
        self.assertEqual(applied["applies_to"]["source_sha256"][path], notices.digest(original))
        self.assertEqual(index["review_findings"], [])
        self.h.write_bytes(original)
        index, texts = self.inventory()
        native.apply_references(index, texts, policy)
        self.assertEqual(index["license_references"][0]["observed_source_forms"][path]["form"], "original_bytes")

    def test_declared_variant_cannot_hide_changed_content_mixed_newlines_or_bare_cr(self):
        policy, value, original, variant, path = self.variant_reference()
        for data in (original.replace(b"Header Holder", b"Substitute Holder").replace(b"\n", b"\r\n"),
                     original.replace(b"\n", b"\r\n", 1), original.replace(b"\n", b"\r\r\n")):
            with self.subTest(data=data[:30]):
                variant["sha256"] = notices.digest(data)
                self.h.write_bytes(data)
                policy.write_bytes(notices.json_bytes(value))
                index, texts = self.inventory()
                with self.assertRaisesRegex(notices.NoticeError, "changed source"):
                    native.apply_references(index, texts, policy)

    def test_variant_requires_exact_working_tree_hash_and_original_git_blob(self):
        policy, value, original, variant, path = self.variant_reference()
        for field, bad in (("sha256", "0" * 64), ("source_git_blob_sha1", "0" * 40)):
            original_field = variant[field]
            variant[field] = bad
            policy.write_bytes(notices.json_bytes(value))
            index, texts = self.inventory()
            with self.assertRaisesRegex(notices.NoticeError, "changed source"):
                native.apply_references(index, texts, policy)
            variant[field] = original_field

    def test_variant_origin_must_match_actual_skia_dependency(self):
        policy, value, original, variant, path = self.variant_reference()
        value["references"][0]["applies_to"]["source_origin"]["revision"] = "b" * 40
        variant["url"] = variant["url"].replace("a" * 40, "b" * 40)
        policy.write_bytes(notices.json_bytes(value))
        index, texts = self.inventory()
        with self.assertRaisesRegex(notices.NoticeError, "observed Skia dependency"):
            native.apply_references(index, texts, policy)

    def test_variant_provenance_and_unreviewed_paths_are_rejected(self):
        policy, value, original, variant, path = self.variant_reference()
        for field, bad in (("transform", "arbitrary-normalization"), ("sha256", "invalid"),
                           ("source_git_blob_sha1", "invalid"), ("url", "https://example.invalid/source")):
            original_field = variant[field]
            variant[field] = bad
            policy.write_bytes(notices.json_bytes(value))
            with self.assertRaises(notices.NoticeError):
                native.load_reference(policy)
            variant[field] = original_field
        variants = value["references"][0]["applies_to"]["working_tree_variants"]
        variants["third_party/externals/zlib/unreviewed.h"] = variants.pop(path)
        policy.write_bytes(notices.json_bytes(value))
        with self.assertRaises(notices.NoticeError):
            native.load_reference(policy)

    def test_malformed_reference_root_is_a_validation_error(self):
        policy = self.reference()
        policy.write_text("[]")
        with self.assertRaisesRegex(notices.NoticeError, "reference format"):
            native.load_reference(policy)

    def test_unknown_native_component_is_rejected_even_if_only_a_header(self):
        self.headers += "    ../source/third_party/externals/unreviewed/header.h\n"
        with self.assertRaisesRegex(notices.NoticeError, "newly used native component"):
            self.inventory()

    def test_external_compiled_input_cannot_be_silently_excluded_as_system(self):
        self.graph += "../unreviewed/native.c\n"
        with self.assertRaisesRegex(notices.NoticeError, "escapes source/build roots"):
            self.inventory()

    def test_missing_recorded_header_requires_cleaning_or_rebuilding_evidence(self):
        self.headers += "    ../source/src/deleted.h\n"
        with self.assertRaisesRegex(notices.NoticeError, "Missing regular native input"):
            self.inventory()

    def test_unbuilt_or_incomplete_native_outputs_are_rejected(self):
        (self.build / "libnative.a").unlink()
        with self.assertRaisesRegex(notices.NoticeError, "Missing generated input or native library"):
            self.inventory()
        (self.build / "libnative.a").write_bytes(b"Native library")
        self.headers = ""
        with self.assertRaisesRegex(notices.NoticeError, "No recorded headers"):
            self.inventory()

    def test_system_header_links_are_excluded_but_source_links_are_rejected(self):
        system = self.root / "system.h"
        try:
            system.symlink_to(self.root / "sdk.h")
        except OSError as error:
            self.skipTest(str(error))
        self.assertEqual(self.inventory()[0]["excluded_inputs"], {"system_or_external_headers": 1})
        self.h.unlink()
        self.h.symlink_to(system)
        with self.assertRaisesRegex(notices.NoticeError, "Native input link"):
            self.inventory()

    def test_notice_symlink_is_rejected(self):
        license = self.external / "LICENSE"
        license.unlink()
        try:
            license.symlink_to(self.source / "LICENSE")
        except OSError as error:
            self.skipTest(str(error))
        with self.assertRaisesRegex(notices.NoticeError, "regular notice input"):
            self.inventory()

    def test_byte_reproducibility_no_overwrite_and_full_source_reverification(self):
        self.collect()
        first = {p.relative_to(self.output): p.read_bytes() for p in self.output.rglob("*") if p.is_file()}
        self.assertTrue(self.verify()["verified_files"])
        with self.assertRaisesRegex(notices.NoticeError, "already exist"):
            self.collect()
        self.output = self.root / "second"
        self.collect()
        second = {p.relative_to(self.output): p.read_bytes() for p in self.output.rglob("*") if p.is_file()}
        self.assertEqual(first, second)
        self.c.write_text("// Copyright 2026 Changed Source Holder\n")
        with self.assertRaisesRegex(notices.NoticeError, "Stale or modified"):
            self.verify()

    def test_static_library_changes_invalidate_build_receipt(self):
        self.collect()
        (self.build / "libnative.a").write_bytes(b"Different native binary")
        with self.assertRaisesRegex(notices.NoticeError, "Stale or modified"):
            self.verify()

    def test_extra_files_or_self_consistent_forged_metadata_do_not_verify(self):
        self.collect()
        extra = self.output / "extra"
        extra.write_bytes(b"Extra")
        with self.assertRaisesRegex(notices.NoticeError, "extra files"):
            self.verify()
        extra.unlink()
        path = self.output / "inventory.json"
        index = json.loads(path.read_text())
        index["components"] = ["invented"]
        path.write_bytes(notices.json_bytes(index))
        manifest_path = self.output / "manifest.json"
        manifest = json.loads(manifest_path.read_text())
        for file in manifest["files"]:
            if file["path"] == "inventory.json":
                file.update({"bytes": path.stat().st_size, "sha256": notices.digest(path.read_bytes())})
        manifest_path.write_bytes(notices.json_bytes(manifest))
        with self.assertRaisesRegex(notices.NoticeError, "Stale or modified"):
            self.verify()


if __name__ == "__main__":
    unittest.main()

"""Reject mismatched source/build identities before publishing review evidence."""

import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import license_notices as notices
import release_evidence as evidence


class ReleaseEvidenceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root, self.target = self.base / "source", self.base / "target"
        self.root.mkdir()
        self.target.mkdir()
        for name, content in {"Cargo.toml": b"[package]\nname='lazypencil'\nversion='0.1.0'\n",
                              "Cargo.lock": b"version=4\n", "LICENSE": b"MIT original\n",
                              "src/main.rs": b"fn main() {}\n"}.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        self.revision = "a" * 40
        self.manifest = {"format_version": 1, "git": {"revision": self.revision, "dirty": False},
                         "package": {"name": "lazypencil", "version": "0.1.0", "rust_version": "1.94"},
                         "files": [{**evidence.fingerprint(p, p.relative_to(self.root).as_posix()), "mode": 0o644}
                                   for p in sorted(self.root.rglob("*")) if p.is_file()]}
        self.write_manifest()
        self.binary = self.target / "release/lazypencil"
        self.binary.parent.mkdir()
        self.binary.write_bytes(b"new release binary")
        self.events = self.base / "cargo.jsonl"
        self.records = [{"reason": "compiler-artifact", "package_id": "root#0.1.0",
                         "target": {"name": "lazypencil", "kind": ["bin"]},
                         "manifest_path": str(self.root / "Cargo.toml"),
                         "profile": {"test": False, "opt_level": "3"}, "fresh": False,
                         "executable": str(self.binary)}]
        for name, libraries in {"skia_bindings": [*evidence.native.TARGETS, "skia-bindings"],
                                "rquickjs_sys": ["quickjs"]}.items():
            out = self.target / "release/build" / (name + "-selected") / "out"
            directory = out / "skia" if name == "skia_bindings" else out
            directory.mkdir(parents=True)
            for library in libraries:
                (directory / ("lib" + library + ".a")).write_bytes(b"native archive " + library.encode())
            self.records += [
                {"reason": "compiler-artifact", "package_id": name + "#1.0.0",
                 "target": {"name": name, "kind": ["lib"]}, "manifest_path": str(self.root / "Cargo.toml"),
                 "profile": {"test": False}, "features": ["default"]},
                {"reason": "build-script-executed", "package_id": name + "#1.0.0", "out_dir": str(out),
                 "linked_libs": ["static=" + library for library in libraries], "linked_paths": [str(directory)],
                 "env": [["SECRET_FIXTURE", "must not be copied"]]},
            ]
        self.records.append({"reason": "build-finished", "success": True})
        self.write_events()

    def write_manifest(self):
        (self.root / "SOURCE-MANIFEST.json").write_bytes(notices.json_bytes(self.manifest))

    def write_events(self):
        self.events.write_text("a non-JSON build-tool line\n" + "\n".join(json.dumps(r) for r in self.records) + "\n")

    def observations(self):
        return evidence.cargo_observations(self.events, self.root, self.target)

    def test_selected_builds_pair_by_package_identity_and_omit_arbitrary_environment(self):
        # An unrelated older build directory must not be discovered by a glob.
        (self.target / "release/build/skia_bindings-newer-but-unselected").mkdir()
        binary, selected, ignored = self.observations()
        self.assertEqual(binary, self.binary)
        self.assertEqual(ignored, 1)
        self.assertNotIn("must not be copied", json.dumps(selected))
        self.assertEqual(len(evidence.native_archives(self.target, selected)["skia_bindings"]), 6)

    def test_completion_failure_missing_or_duplicate_is_rejected(self):
        original = list(self.records)
        for tail in ([], [{"reason": "build-finished", "success": False}],
                     [original[-1], original[-1]], [original[-1], original[0]]):
            with self.subTest(tail=tail):
                self.records = original[:-1] + tail
                self.write_events()
                with self.assertRaises(notices.NoticeError):
                    self.observations()

    def test_wrong_binary_manifest_profile_or_cached_application_is_rejected(self):
        original = dict(self.records[0])
        for field, value in (("fresh", True), ("manifest_path", str(self.root / "LICENSE")),
                             ("profile", {"test": False, "opt_level": "0"}),
                             ("executable", str(self.root / "LICENSE"))):
            with self.subTest(field=field):
                self.records[0] = {**original, field: value}
                self.write_events()
                with self.assertRaises(notices.NoticeError):
                    self.observations()

    def test_duplicate_or_unpaired_native_build_is_rejected(self):
        original = list(self.records)
        for records in (original[:-1] + [original[2], original[-1]],
                        [r for r in original if not (r.get("reason") == "build-script-executed"
                                                    and r["package_id"].startswith("skia_bindings"))]):
            self.records = records
            self.write_events()
            with self.assertRaises(notices.NoticeError):
                self.observations()

    def test_unrelated_package_may_have_multiple_build_script_contexts(self):
        self.records[-1:-1] = [
            {"reason": "build-script-executed", "package_id": "unrelated#1.0.0", "out_dir": "host context"},
            {"reason": "build-script-executed", "package_id": "unrelated#1.0.0", "out_dir": "target context"},
        ]
        self.write_events()
        self.assertEqual(self.observations()[0], self.binary)

    def test_native_build_escape_and_missing_archive_are_rejected(self):
        self.records[2]["out_dir"] = str(self.root)
        self.write_events()
        with self.assertRaises(notices.NoticeError):
            self.observations()
        self.records[2]["out_dir"] = str(self.target / "release/build/skia_bindings-selected/out")
        self.write_events()
        _, selected, _ = self.observations()
        (self.target / selected["rquickjs_sys"]["out_dir"] / "libquickjs.a").unlink()
        with self.assertRaises(notices.NoticeError):
            evidence.native_archives(self.target, selected)

    def test_windows_archive_names_and_native_link_search_kinds(self):
        for record in self.records:
            if record.get("reason") != "build-script-executed":
                continue
            record["linked_libs"] = [v.removeprefix("static=") for v in record["linked_libs"]]
            record["linked_paths"] = ["native=" + v for v in record["linked_paths"]]
        self.write_events()
        for path in self.target.rglob("lib*.a"):
            path.rename(path.with_name(path.name[3:-2] + ".lib"))
        _, selected, _ = self.observations()
        self.assertEqual(len(evidence.native_archives(self.target, selected)["rquickjs_sys"]), 1)

    def test_cc_archive_aliases_must_have_identical_bytes(self):
        _, selected, _ = self.observations()
        directory = self.target / selected["rquickjs_sys"]["out_dir"]
        alias = directory / "quickjs.lib"
        alias.write_bytes((directory / "libquickjs.a").read_bytes())
        self.assertEqual(len(evidence.native_archives(self.target, selected)["rquickjs_sys"]), 2)
        alias.write_bytes(b"stale MSVC archive")
        with self.assertRaises(notices.NoticeError):
            evidence.native_archives(self.target, selected)

    def test_source_revision_dirty_content_and_unlisted_files_are_rejected(self):
        self.assertEqual(evidence.source_identity(self.root, self.revision)["files"], 4)
        for change in ("revision", "dirty", "content", "unlisted"):
            with self.subTest(change=change):
                if change == "revision":
                    self.manifest["git"]["revision"] = "b" * 40
                elif change == "dirty":
                    self.manifest["git"]["dirty"] = True
                elif change == "content":
                    (self.root / "LICENSE").write_bytes(b"changed")
                else:
                    (self.root / "build.rs").write_text("unexpected build input")
                self.write_manifest()
                with self.assertRaises(notices.NoticeError):
                    evidence.source_identity(self.root, self.revision)
                self.manifest["git"] = {"revision": self.revision, "dirty": False}
                (self.root / "LICENSE").write_bytes(b"MIT original\n")

    def test_source_duplicates_and_path_escape_are_rejected(self):
        self.manifest["files"].append(dict(self.manifest["files"][0]))
        self.write_manifest()
        with self.assertRaises(notices.NoticeError):
            evidence.source_identity(self.root, self.revision)
        self.manifest["files"][-1]["path"] = "../outside"
        self.write_manifest()
        with self.assertRaises(evidence.dist.DistributionError):
            evidence.source_identity(self.root, self.revision)

    def test_linked_input_is_rejected(self):
        if not hasattr(__import__("os"), "symlink"):
            self.skipTest("No symlink support")
        external = self.base / "external"
        external.write_bytes(self.binary.read_bytes())
        self.binary.unlink()
        try:
            self.binary.symlink_to(external)
        except OSError:
            self.skipTest("Symlinks require privileges on this host")
        with self.assertRaises(notices.NoticeError):
            self.observations()

    def prepare_collection(self):
        (self.root / "vendor/skia-bindings/skia").mkdir(parents=True)
        (self.root / "resources/licenses/rust").mkdir(parents=True)
        # Empty directories do not add source inputs.
        self.output = self.base / "evidence"

    def collect(self, native_effect=None):
        def fake_native(source, build, output, ninja, upstream):
            output.mkdir()
            (output / "SKIA-NOTICES.txt").write_text("original notices")
            if native_effect is not None:
                native_effect()
            return {"components": ["skia"], "review_findings": []}

        with patch.object(evidence.notices, "verify", return_value={"source_archives_verified": False}), \
                patch.object(evidence.native, "collect", side_effect=fake_native), \
                patch.object(evidence.native, "verify"), \
                patch.object(evidence.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"rustc pinned\n")):
            return evidence.collect(self.root, self.target, self.events, self.revision, self.output)

    def test_collection_is_explicitly_incomplete_and_output_cannot_be_replaced(self):
        self.prepare_collection()
        result = self.collect()
        self.assertFalse(result["release_ready"])
        report = json.loads((self.output / "build.json").read_bytes())
        self.assertEqual(report["binary"]["sha256"], notices.digest(self.binary.read_bytes()))
        self.assertGreaterEqual(len(report["remaining_review"]), 5)
        self.assertNotIn("must not be copied", (self.output / "build.json").read_text())
        manifest = json.loads((self.output / "manifest.json").read_bytes())
        for item in manifest["files"]:
            self.assertEqual(evidence.fingerprint(self.output / item["path"], item["path"]), item)
        with self.assertRaises(notices.NoticeError):
            self.collect()

    def test_changed_binary_source_or_cargo_records_aborts_without_publishing(self):
        self.prepare_collection()
        for path in (self.binary, self.root / "src/main.rs", self.events):
            with self.subTest(path=path):
                original = path.read_bytes()
                with self.assertRaises(notices.NoticeError):
                    self.collect(lambda: path.write_bytes(b"modified during collection"))
                self.assertFalse(self.output.exists())
                path.write_bytes(original)


if __name__ == "__main__":
    unittest.main()

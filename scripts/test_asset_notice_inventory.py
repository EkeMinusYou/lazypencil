"""Keep original asset notices attached to reviewed source-level inclusions."""

from pathlib import Path
import json
import os
import tempfile
import unittest

import asset_notice_inventory as assets
import license_notices as notices


class AssetNoticeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.output = self.root / "output"
        self.font = {"family": "Noto Sans JP", "license": "OFL-1.1", "files": []}
        for name, data in {"NotoSansJP.ttf": b"original font fixture", "OFL.txt": b"Original OFL grant. Copyright font holder.\r\n",
                           "METADATA.pb": b"Original publisher metadata\n"}.items():
            self.write("resources/fonts/" + name, data)
            self.font["files"].append({"file": name, "bytes": len(data), "sha256": notices.digest(data)})
        self.write_json("resources/fonts/manifest.json", self.font)
        self.catalogue = []
        for name in sorted(assets.PACKS):
            directory = "resources/icons/" + name + "/"
            legal = ("Original " + name + " copyright and license.\r\n").encode()
            self.write(directory + "LICENSE", legal)
            entry = {"id": name, "version": "1.0.0", "source_url": "https://example.test/source",
                     "license": "MIT", "license_sha256": notices.digest(legal), "bundled": name == "lucide"}
            data = ("Original input for " + name).encode()
            if name == "lucide":
                self.write(directory + "icons.json", data)
                entry["data_sha256"] = notices.digest(data)
            elif name != "feather":
                self.write(directory + ("index.json" if name == "phosphor" else "codepoints.txt"), data)
                entry["index_sha256"] = notices.digest(data)
                self.write_json(directory + "provenance.json", {"descriptor": entry, "normalization": "fixture"})
            self.catalogue.append(entry)
        self.write_json("resources/icons/catalogue.json", self.catalogue)
        for i, (source, target) in enumerate(sorted(assets.EXPECTED)):
            path = self.root / source
            path.parent.mkdir(parents=True, exist_ok=True)
            relative = os.path.relpath(self.root / target, path.parent).replace(os.sep, "/")
            with path.open("a", encoding="utf-8", newline="\n") as stream:
                stream.write(f'const DATA_{i}: &[u8] = include_bytes!("{relative}");\n')

    def write(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def write_json(self, name, data):
        self.write(name, notices.json_bytes(data))

    def test_original_notices_and_provenance_are_retained_without_payload_duplication(self):
        result = assets.collect(self.root, self.output)
        self.assertEqual(result, {"embedded_declarations": 15, "asset_inputs": 20, "icon_packs": 6, "original_files": 14})
        assets.verify(self.root, self.output)
        self.assertFalse((self.output / "resources/fonts/NotoSansJP.ttf").exists())
        self.assertFalse((self.output / "resources/icons/lucide/icons.json").exists())
        self.assertEqual((self.output / "resources/fonts/OFL.txt").read_bytes(), (self.root / "resources/fonts/OFL.txt").read_bytes())
        self.assertEqual((self.output / "resources/icons/phosphor/LICENSE").read_bytes(),
                         (self.root / "resources/icons/phosphor/LICENSE").read_bytes())
        self.assertIn(b"Original OFL grant.", (self.output / "ASSET-NOTICES.txt").read_bytes())

    def test_payload_license_and_index_changes_fail_before_output_is_created(self):
        for name in ["resources/fonts/NotoSansJP.ttf", "resources/fonts/OFL.txt", "resources/icons/lucide/icons.json",
                     "resources/icons/Material Symbols Sharp/codepoints.txt", "resources/icons/phosphor/LICENSE"]:
            path = self.root / name
            original = path.read_bytes()
            path.write_bytes(original + b"changed")
            with self.subTest(name=name), self.assertRaises(notices.NoticeError):
                assets.collect(self.root, self.output)
            self.assertFalse(self.output.exists())
            path.write_bytes(original)

    def test_added_missing_and_computed_declarations_require_review(self):
        path = self.root / "src/text.rs"
        original = path.read_bytes()
        for data in [b"", original + original.replace(b"DATA_", b"OTHER_"),
                     original + b'const EXTRA: &[u8]=include_bytes!("../resources/fonts/OFL.txt");',
                     original + b'const OTHER: &str=include_str!(concat!("a", "b"));']:
            path.write_bytes(data)
            with self.assertRaises(notices.NoticeError):
                assets.build_inventory(self.root)
        path.write_bytes(original)

    def test_optional_geometry_and_provenance_cannot_silently_change_scope(self):
        self.catalogue[0]["bundled"] = True
        self.write_json("resources/icons/catalogue.json", self.catalogue)
        with self.assertRaises(notices.NoticeError):
            assets.build_inventory(self.root)
        self.catalogue[0]["bundled"] = False
        self.write_json("resources/icons/catalogue.json", self.catalogue)
        self.write_json("resources/icons/phosphor/provenance.json", {"descriptor": {"id": "stale"}})
        with self.assertRaises(notices.NoticeError):
            assets.build_inventory(self.root)

    def test_reconstruction_detects_notice_changes_missing_and_extra_files(self):
        assets.collect(self.root, self.output)
        path = self.output / "resources/fonts/OFL.txt"
        original = path.read_bytes()
        path.write_bytes(b"replacement terms")
        with self.assertRaises(notices.NoticeError):
            assets.verify(self.root, self.output)
        path.write_bytes(original)
        extra = self.output / "unlisted"
        extra.write_bytes(b"extra")
        with self.assertRaises(notices.NoticeError):
            assets.verify(self.root, self.output)
        extra.unlink()
        path.unlink()
        with self.assertRaises(notices.NoticeError):
            assets.verify(self.root, self.output)

    def test_existing_output_is_not_overwritten(self):
        self.output.mkdir()
        sentinel = self.output / "keep"
        sentinel.write_bytes(b"preserve")
        with self.assertRaises(notices.NoticeError):
            assets.collect(self.root, self.output)
        self.assertEqual(sentinel.read_bytes(), b"preserve")

    def test_linked_asset_is_rejected_even_if_bytes_match(self):
        path = self.root / "resources/fonts/OFL.txt"
        other = self.root / "same-bytes"
        other.write_bytes(path.read_bytes())
        path.unlink()
        try:
            path.symlink_to(other)
        except OSError:
            self.skipTest("Symlinks require privileges on this host")
        with self.assertRaises(notices.NoticeError):
            assets.build_inventory(self.root)


if __name__ == "__main__":
    unittest.main()

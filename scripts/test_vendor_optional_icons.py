"""Independent checks for source-only icon normalization and catalogue parsing."""

import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest

import vendor_optional_icons as icons


class OptionalIconSources(unittest.TestCase):
    def test_catalogue_literal_reads_data_and_preserves_escaped_strings(self):
        text = 'const n = [{name:"demo",pascal_name:"Demo",tags:["a\\\"b","日本語"],categories:[e.SYSTEM],figma_category:a.SYSTEM,codepoint:123,published_in:2.1,alias:{name:"old",pascal_name:"Old"}}];\nexport { n as icons };'
        value = icons.catalogue_literal(text)
        self.assertEqual(value[0]["tags"], ['a"b', "日本語"])
        self.assertEqual(value[0]["alias"], {"name": "old", "pascal_name": "Old"})
        self.assertEqual(value[0]["codepoint"], 123)

    def test_executable_expressions_and_duplicate_fields_are_rejected(self):
        for expression in ['process.exit(1)', '(()=>[])()', 'import("x")', 'e.SYSTEM + run()', '{name:"x",name:"y"}']:
            with self.subTest(expression=expression):
                with self.assertRaises(ValueError):
                    icons.catalogue_literal('const n = [' + expression + '];\nexport { n as icons };')

    def test_catalogue_boundary_must_be_the_pinned_declaration_and_export(self):
        for text in ['const other = [];', 'const n = [];\nrun();', 'const n = [];\nexport { n }; const n = [];']:
            with self.subTest(text=text):
                with self.assertRaises(ValueError):
                    icons.catalogue_literal(text)

    def test_wrong_source_is_rejected_before_derived_resources_are_created(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "phosphor-core-2.1.1.tgz").write_bytes(b"not the original archive")
            with self.assertRaisesRegex(ValueError, "Source differs"):
                icons.phosphor(root, root / "output")
            self.assertFalse((root / "output").exists())

    def test_original_phosphor_fixture_reproduces_index_license_and_geometry_bytes(self):
        project = Path(__file__).resolve().parent.parent
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shutil.copyfile(project / "tests/fixtures/icons/phosphor-core-2.1.1.tgz", root / "phosphor-core-2.1.1.tgz")
            descriptor, provenance = icons.phosphor(root, root / "output")
            expected = next(d for d in json.loads((project / "resources/icons/catalogue.json").read_text()) if d["id"] == "phosphor")
            self.assertEqual(descriptor, expected)
            for name in ["LICENSE", "index.json"]:
                self.assertEqual((root / "output/phosphor" / name).read_bytes(), (project / "resources/icons/phosphor" / name).read_bytes())
            self.assertEqual(hashlib.sha256((root / "phosphor.icons.json").read_bytes()).hexdigest(), expected["data_sha256"])
            self.assertEqual(provenance["catalogue_member"], "package/dist/index.mjs")
            self.assertEqual((descriptor["icon_count"], descriptor["alias_count"]), (1512, 1548))


if __name__ == "__main__":
    unittest.main()

"""Validate hosted font-source preparation without downloading or compiling."""

import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.parse

SPEC = importlib.util.spec_from_file_location(
    "prepare_material_fonts", Path(__file__).parent / "ci/prepare_material_fonts.py"
)
fonts = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fonts)


class Response(io.BytesIO):
    def __init__(self, data, url):
        super().__init__(data)
        self.url = url

    def geturl(self):
        return self.url


class MaterialSources(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="material-font-ci-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.catalogue = self.root / "catalogue.json"
        self.output = self.root / "fonts"
        self.records = []
        self.payloads = {}
        for style in fonts.STYLES:
            filename = f"MaterialSymbols{style}[FILL,GRAD,opsz,wght].ttf"
            url = ("https://raw.githubusercontent.com/google/material-design-icons/"
                   + "a" * 40 + "/variablefont/" + urllib.parse.quote(filename, safe=""))
            data = ("pinned-" + style).encode()
            self.records.append({"id": "Material Symbols " + style, "version": "a" * 40,
                                 "source_url": url, "source_format": "ttf",
                                 "source_sha256": hashlib.sha256(data).hexdigest()})
            self.payloads[url] = data
        self.save()

    def save(self):
        self.catalogue.write_text(json.dumps(self.records), encoding="utf-8")

    def open(self, request):
        return Response(self.payloads[request.full_url], request.full_url)

    def test_original_bytes_are_verified_and_reused_without_network(self):
        first = fonts.prepare(self.catalogue, self.output, self.open)
        self.assertEqual(len(first), 3)
        for item in first:
            self.assertFalse(item["reused"])
            record = next(r for r in self.records if r["id"] == item["pack"])
            self.assertEqual((self.output / item["filename"]).read_bytes(),
                             self.payloads[record["source_url"]])
        def fail(_):
            self.fail("A verified cached font must not make a network request")
        self.assertTrue(all(item["reused"] for item in fonts.prepare(self.catalogue, self.output, fail)))
        (self.output / first[0]["filename"]).write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            fonts.prepare(self.catalogue, self.output, fail)

    def test_bad_downloads_and_oversized_sources_leave_no_font_or_temporary_file(self):
        for oversized in [False, True]:
            with self.subTest(oversized=oversized), patch.object(fonts, "MAX_BYTES", 32):
                def broken(request):
                    return Response(b"x" * (33 if oversized else 8), request.full_url)
                with self.assertRaisesRegex(ValueError, "exceeds|checksum mismatch"):
                    fonts.prepare(self.catalogue, self.output, broken)
                self.assertEqual(list(self.output.iterdir()), [])

    def test_mutable_sources_and_unexpected_redirects_are_rejected(self):
        self.records[0]["version"] = "main"
        self.save()
        with self.assertRaisesRegex(ValueError, "immutable source pin"):
            fonts.prepare(self.catalogue, self.output, self.open)
        self.records[0]["version"] = "a" * 40
        self.save()
        def redirect(request):
            return Response(self.payloads[request.full_url], "http://example.invalid/font.ttf")
        with self.assertRaisesRegex(ValueError, "redirect"):
            fonts.prepare(self.catalogue, self.output, redirect)
        self.assertEqual(list(self.output.iterdir()), [])

    def test_exact_size_limit_is_accepted(self):
        for item in self.records:
            self.payloads[item["source_url"]] = b"a" * 32
            item["source_sha256"] = hashlib.sha256(b"a" * 32).hexdigest()
        self.save()
        with patch.object(fonts, "MAX_BYTES", 32):
            self.assertTrue(all(item["bytes"] == 32 for item in fonts.prepare(self.catalogue, self.output, self.open)))


if __name__ == "__main__":
    unittest.main()

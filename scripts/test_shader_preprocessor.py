"""Reject stale or unsafe compiled shader modules before rendering builds."""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import license_notices as notices
import shader_preprocessor as component


class ShaderPreprocessorTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(__file__).resolve().parent.parent

    def test_actual_shipped_module_binds_sources_notices_and_its_memory_limit(self):
        result = component.verify(self.root)
        self.assertEqual(result["memory_import"]["maximum_pages"], 1024)
        self.assertEqual(result["original_package_notices"], 69)
        self.assertEqual(result["target_library_notices"], 54)

    def test_stale_component_hash_or_module_cannot_pass(self):
        original = notices.read
        for field, value in [("wasm_sha256", "0" * 64), ("source_files", {}), ("memory_maximum_bytes", 128 * 1024 * 1024)]:
            def changed(path, maximum=notices.MAX_FILE):
                data = original(path, maximum)
                if path == self.root / "resources/shader-preprocessor/build.json":
                    report = json.loads(data)
                    report[field] = value
                    return json.dumps(report).encode()
                return data
            with self.subTest(field=field), patch.object(notices, "read", side_effect=changed):
                with self.assertRaises(notices.NoticeError):
                    component.verify(self.root)

    def test_wrong_rebuilt_binary_is_not_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "module.wasm"
            binary.write_bytes(b"not the rebuilt module")
            with self.assertRaises(notices.NoticeError):
                component.verify(self.root, binary)

    def test_host_function_import_and_larger_memory_are_rejected(self):
        def number(value):
            result = bytearray()
            while value >= 128:
                result.append((value & 127) | 128)
                value >>= 7
            result.append(value)
            return bytes(result)
        def module(kind, maximum):
            entry = b"\x03env\x06memory" + bytes([kind]) + b"\x01" + number(256) + number(maximum)
            section = b"\x01" + entry
            return b"\0asm\x01\0\0\0\x02" + number(len(section)) + section
        for binary in [module(0, 1024), module(2, 2048), b"\0asm\x01\0\0\0", module(2, 1024)[:-1]]:
            with self.subTest(binary=binary), self.assertRaises(notices.NoticeError):
                component.memory_import(binary)


if __name__ == "__main__":
    unittest.main()

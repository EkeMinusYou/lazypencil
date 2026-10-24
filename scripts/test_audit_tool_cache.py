import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("audit_tool_cache", Path(__file__).parent / "ci/audit_tool_cache.py")
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class AuditToolCacheTests(unittest.TestCase):
    def test_version_compiler_image_and_architecture_bind_identity(self):
        original = cache.identity("compiler-one", "image-one", "X64")
        for arguments in (("compiler-two", "image-one", "X64"), ("compiler-one", "image-two", "X64"), ("compiler-one", "image-one", "ARM64")):
            self.assertNotEqual(cache.identity(*arguments), original)
        with patch.object(cache, "VERSION", "other"):
            self.assertNotEqual(cache.identity("compiler-one", "image-one", "X64"), original)
        with self.assertRaises(ValueError):
            cache.identity("compiler", "unknown", "X64")

    def test_binary_hash_and_observed_version_are_required(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "bin/cargo-deny"
            binary.parent.mkdir()
            binary.write_bytes(b"fixture binary")
            result = subprocess.CompletedProcess([], 0, f"cargo-deny {cache.VERSION}\n", "")
            with patch.object(cache.subprocess, "run", return_value=result):
                original = cache.record(root, "compiler-image-fingerprint")
                binary.write_bytes(b"changed fixture binary")
                self.assertNotEqual(cache.record(root, "compiler-image-fingerprint"), original)
            with patch.object(cache.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "cargo-deny wrong\n", "")):
                with self.assertRaises(ValueError):
                    cache.record(root, "compiler-image-fingerprint")


if __name__ == "__main__":
    unittest.main()

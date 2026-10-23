"""Check native measurement attribution, failure cleanup and evidence integrity."""

import binascii
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib

import measure_native_performance as benchmark


def png():
    def chunk(kind, data):
        return (struct.pack(">I", len(data)) + kind + data
                + struct.pack(">I", binascii.crc32(kind + data)))
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", 1920, 1080, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress((b"\0" + bytes(1920 * 4)) * 1080))
            + chunk(b"IEND", b""))


class NativeMeasurement(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="native-measure-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def child(self, code, timeout=5):
        return benchmark.measure([sys.executable, "-c", code], dict(os.environ), self.root, timeout)

    def test_child_counters_are_not_the_parents_or_previous_childs_high_water_mark(self):
        high, _ = self.child("data=bytes([1])*(64*1024*1024); print(len(data))")
        low, output = self.child("import time; time.sleep(.025); print('measured')")
        self.assertEqual(output.strip(), b"measured")
        self.assertGreater(high["peak_rss_bytes"], low["peak_rss_bytes"] + 16 * 1024 * 1024)
        self.assertGreater(low["peak_rss_bytes"], 0)
        self.assertGreaterEqual(low["elapsed_ms"], 25)
        self.assertGreaterEqual(low["user_cpu_ms"], 0)
        self.assertGreaterEqual(low["system_cpu_ms"], 0)
        self.assertIn("resource_method", high)

    def test_failed_timed_out_and_oversized_children_do_not_become_samples(self):
        with self.assertRaisesRegex(ValueError, "failed \\(7\\)"):
            self.child("import sys; sys.stderr.write('fixture error'); sys.exit(7)")
        processes = []
        original = subprocess.Popen
        def record(*args, **kwargs):
            process = original(*args, **kwargs)
            processes.append(process)
            return process
        with patch.object(benchmark.subprocess, "Popen", side_effect=record):
            with self.assertRaises(TimeoutError):
                self.child("import time; time.sleep(5)", timeout=.2)
        self.assertIsNotNone(processes[0].returncode)
        if os.name != "nt":
            with self.assertRaises(ChildProcessError):
                os.waitpid(processes[0].pid, os.WNOHANG)
        with self.assertRaisesRegex(ValueError, "log limit"):
            self.child("import sys; sys.stdout.write('x'*(5*1024*1024))")
        for invalid in (0, -1, float("nan"), float("inf")):
            with self.assertRaises(ValueError):
                self.child("pass", timeout=invalid)

    def test_render_validation_binds_geometry_execution_scope_and_exact_png(self):
        output = self.root / "render.png"
        data = png()
        output.write_bytes(data)
        report = {"width": 1920, "height": 1080, "node_count": 1000,
                  "diagnostics": [], "assets": [], "dynamic_artifacts": [],
                  "script_executions": [], "png_hash": hashlib.sha256(data).hexdigest()}
        self.assertEqual(benchmark.validate_render(json.dumps(report), output, 0), report["png_hash"])
        for key, value in [("width", 100), ("node_count", 999), ("diagnostics", [{}]),
                           ("dynamic_artifacts", [{}]), ("script_executions", [{}]),
                           ("assets", [{}]), ("assets", None), ("png_hash", "forged")]:
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                benchmark.validate_render(json.dumps(dict(report, **{key: value})), output, 0)
        output.write_bytes(data + b"changed")
        with self.assertRaisesRegex(ValueError, "hash"):
            benchmark.validate_render(json.dumps(report), output, 0)
        output.write_bytes(b"not a png" * 4)
        with self.assertRaisesRegex(ValueError, "geometry"):
            benchmark.validate_render(json.dumps(report), output, 0)
        with output.open("wb") as stream:
            stream.truncate(benchmark.MAX_PNG_BYTES + 1)
        with self.assertRaisesRegex(ValueError, "oversized"):
            benchmark.validate_render(json.dumps(report), output, 0)

    def test_summary_uses_observed_nearest_rank_p95(self):
        samples = [{key: value for key in ("elapsed_ms", "peak_rss_bytes", "user_cpu_ms", "system_cpu_ms")}
                   for value in range(1, 21)]
        result = benchmark.summarize(samples)["elapsed_ms"]
        self.assertEqual(result, {"min": 1, "median": 10.5, "p95_nearest_rank": 19, "max": 20})
        with self.assertRaises(ValueError):
            benchmark.summarize([])

    def test_driver_uses_fixed_fixtures_isolated_data_and_excludes_first_invocations(self):
        binary = self.root / "binary"
        binary.write_bytes(b"original fixture executable")
        data = png()
        calls, directories = [], []
        def measured(command, environment, cwd, timeout):
            self.assertEqual(command[0], str(binary.resolve()))
            self.assertIn("--data-dir", command)
            self.assertNotEqual(environment["LAZYPENCIL_CONFIG_DIR"], "ambient")
            self.assertEqual((cwd / "lazypencil.toml").read_bytes(), b"version=1\n")
            output = Path(command[command.index("--output") + 1])
            self.assertFalse(output.exists())
            output.write_bytes(data)
            directories.append(cwd)
            calls.append(command)
            result = {"width": 1920, "height": 1080, "node_count": 1000,
                      "diagnostics": [], "assets": [{}] if cwd.name == "dashboard" else [],
                      "dynamic_artifacts": [], "script_executions": [],
                      "png_hash": hashlib.sha256(data).hexdigest()}
            metrics = {key: len(calls) * 10 for key in ("elapsed_ms", "peak_rss_bytes", "user_cpu_ms", "system_cpu_ms")}
            return dict(metrics, resource_method="fixture"), json.dumps(result)
        with patch.dict(os.environ, {"LAZYPENCIL_CONFIG_DIR": "ambient"}):
            with patch.object(benchmark, "measure", side_effect=measured):
                report = benchmark.benchmark(binary, samples=2)
            self.assertEqual(os.environ["LAZYPENCIL_CONFIG_DIR"], "ambient")
        self.assertEqual(len(calls), 6)
        shadow, dashboard = report["scenes"]
        self.assertEqual(shadow["first_invocation"]["elapsed_ms"], 10)
        self.assertEqual(shadow["summary"]["elapsed_ms"]["median"], 25)
        self.assertEqual(dashboard["summary"]["elapsed_ms"]["median"], 55)
        self.assertEqual(shadow["document_sha256"], "d0a54ed3c8b3716b3ac5f26fe79e4e130164e8afd128bb4146ef96774ccbaa8b")
        self.assertEqual(dashboard["document_sha256"], "c55bcf8727afc04d31646ea1a259e17723fe9be03e3cb4fd2dccbf8512764da7")
        self.assertTrue(all(not path.exists() for path in directories))
        for count in (0, 1, 201):
            with self.assertRaises(ValueError):
                benchmark.benchmark(binary, count)

    def test_changed_pixels_or_executable_are_rejected(self):
        binary = self.root / "binary"
        binary.write_bytes(b"original fixture executable")
        for change in ("pixels", "binary"):
            metrics = {key: 1 for key in ("elapsed_ms", "peak_rss_bytes", "user_cpu_ms", "system_cpu_ms")}
            hashes = iter(range(6))
            with patch.object(benchmark, "measure", return_value=(metrics, b"{}")):
                def validate(*_):
                    if change == "binary":
                        binary.write_bytes(b"modified executable")
                        return "same"
                    return str(next(hashes))
                with patch.object(benchmark, "validate_render", side_effect=validate):
                    with self.assertRaises(ValueError):
                        benchmark.benchmark(binary, samples=2)

    def test_publication_never_replaces_a_report_and_removes_failed_staging_files(self):
        output = self.root / "report.json"
        benchmark.publish({"complete": True}, output)
        original = output.read_bytes()
        with self.assertRaises(FileExistsError):
            benchmark.publish({"complete": False}, output)
        self.assertEqual(output.read_bytes(), original)
        self.assertFalse(list(self.root.glob(".native-report-*")))
        with patch.object(benchmark.os, "fsync", side_effect=OSError("fixture write failure")):
            with self.assertRaises(OSError):
                benchmark.publish({}, self.root / "incomplete.json")
        self.assertFalse((self.root / "incomplete.json").exists())
        self.assertFalse(list(self.root.glob(".native-report-*")))


if __name__ == "__main__":
    unittest.main()

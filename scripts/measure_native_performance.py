#!/usr/bin/env python3
"""Measure fresh native CLI children on the two fixed 1,000-node scenes."""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import signal
import statistics
import struct
import subprocess
import sys
import tempfile
import time

import make_dashboard_performance_fixture as dashboard
import make_native_performance_fixture as native

MAX_LOG_BYTES = 4 * 1024 * 1024
MAX_PNG_BYTES = 16 * 1024 * 1024
POLL_SECONDS = 0.001


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(64 * 1024):
            digest.update(block)
    return digest.hexdigest()


def windows_usage(process):
    """Query the retained process handle, including after it has exited."""
    import ctypes
    from ctypes import wintypes

    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in (
                "peak_working_set", "working_set", "peak_paged_pool", "paged_pool",
                "peak_nonpaged_pool", "nonpaged_pool", "pagefile", "peak_pagefile",
            )
        ]

    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    memory = psapi.GetProcessMemoryInfo
    memory.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    memory.restype = wintypes.BOOL
    counters = Counters()
    counters.cb = ctypes.sizeof(counters)
    if not memory(int(process._handle), ctypes.byref(counters), counters.cb):
        raise ctypes.WinError(ctypes.get_last_error())
    times = ctypes.WinDLL("kernel32", use_last_error=True).GetProcessTimes
    times.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    times.restype = wintypes.BOOL
    created, exited, kernel, user = [wintypes.FILETIME() for _ in range(4)]
    if not times(int(process._handle), *map(ctypes.byref, (created, exited, kernel, user))):
        raise ctypes.WinError(ctypes.get_last_error())
    milliseconds = lambda value: ((value.dwHighDateTime << 32) | value.dwLowDateTime) / 10_000
    return {
        "peak_rss_bytes": counters.peak_working_set,
        "user_cpu_ms": milliseconds(user),
        "system_cpu_ms": milliseconds(kernel),
        "resource_method": "GetProcessMemoryInfo/PeakWorkingSetSize;GetProcessTimes",
    }


def measure(command, environment, cwd, timeout=30):
    if sys.platform not in ("darwin", "linux", "win32"):
        raise ValueError("Native resource measurement supports macOS, Linux and Windows")
    if not math.isfinite(timeout) or timeout <= 0:
        raise ValueError("A finite positive child timeout is required")
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        started = time.monotonic_ns()
        process = subprocess.Popen(command, cwd=cwd, env=environment,
                                   stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
        usage = None
        try:
            while True:
                if os.name == "nt":
                    if process.poll() is not None:
                        ended = time.monotonic_ns()
                        usage = windows_usage(process)
                        break
                else:
                    # Own reaping: Popen.wait/poll would discard per-child rusage.
                    pid, status, resources = os.wait4(process.pid, os.WNOHANG)
                    if pid:
                        ended = time.monotonic_ns()
                        process.returncode = os.waitstatus_to_exitcode(status)
                        usage = {
                            "peak_rss_bytes": int(resources.ru_maxrss) * (1 if sys.platform == "darwin" else 1024),
                            "user_cpu_ms": resources.ru_utime * 1000,
                            "system_cpu_ms": resources.ru_stime * 1000,
                            "resource_method": "wait4/ru_maxrss;ru_utime;ru_stime",
                        }
                        break
                if (time.monotonic_ns() - started) / 1e9 > timeout:
                    raise TimeoutError("Native measurement child exceeded its deadline")
                if max(os.fstat(stdout.fileno()).st_size, os.fstat(stderr.fileno()).st_size) > MAX_LOG_BYTES:
                    raise ValueError("Native measurement child exceeded its log limit")
                time.sleep(POLL_SECONDS)
        finally:
            if process.returncode is None:
                if os.name == "nt":
                    process.kill()
                    process.wait()
                else:
                    os.kill(process.pid, signal.SIGKILL)
                    _, status, _ = os.wait4(process.pid, 0)
                    process.returncode = os.waitstatus_to_exitcode(status)
        elapsed = (ended - started) / 1e6
        if elapsed > timeout * 1000:
            raise TimeoutError("Native measurement child exceeded its deadline")
        stdout.seek(0)
        stderr.seek(0)
        out, err = stdout.read(MAX_LOG_BYTES + 1), stderr.read(MAX_LOG_BYTES + 1)
        if max(len(out), len(err)) > MAX_LOG_BYTES:
            raise ValueError("Native measurement child exceeded its log limit")
        if process.returncode:
            raise ValueError(f"Native measurement child failed ({process.returncode}): "
                             + err[:2048].decode("utf-8", errors="replace"))
        if usage["peak_rss_bytes"] <= 0:
            raise ValueError("The operating system did not supply a positive peak RSS")
        return dict(usage, elapsed_ms=elapsed), out


def validate_render(raw, output, asset_count):
    report = json.loads(raw)
    if not isinstance(report, dict):
        raise ValueError("Expected a native render report object")
    expected = (1920, 1080, 1000)
    if (report.get("width"), report.get("height"), report.get("node_count")) != expected:
        raise ValueError("The benchmark requires exactly 1,000 nodes at 1920x1080")
    if (report.get("diagnostics") != [] or not isinstance(report.get("assets"), list)
            or len(report["assets"]) != asset_count):
        raise ValueError("Unexpected benchmark diagnostics or asset count")
    if report.get("dynamic_artifacts") != [] or report.get("script_executions") != []:
        raise ValueError("Only static native workloads can use this measurement scope")
    if output.is_symlink() or not output.is_file() or not 24 <= output.stat().st_size <= MAX_PNG_BYTES:
        raise ValueError("Missing or oversized benchmark PNG")
    with output.open("rb") as stream:
        header = stream.read(24)
    if header[:8] != b"\x89PNG\r\n\x1a\n" or struct.unpack(">II", header[16:24]) != expected[:2]:
        raise ValueError("Unexpected benchmark PNG geometry")
    actual = sha256(output)
    if actual != report.get("png_hash"):
        raise ValueError("Benchmark PNG does not match its report hash")
    return actual


def summarize(samples):
    if not samples:
        raise ValueError("At least one repeated sample is required")
    result = {}
    for field in ("elapsed_ms", "peak_rss_bytes", "user_cpu_ms", "system_cpu_ms"):
        values = sorted(sample[field] for sample in samples)
        result[field] = {"min": values[0], "median": statistics.median(values),
                         "p95_nearest_rank": values[math.ceil(len(values) * 0.95) - 1],
                         "max": values[-1]}
    return result


def benchmark(binary, samples=20, timeout=30):
    if not 2 <= samples <= 200:
        raise ValueError("Repeated sample count must be in 2..200")
    binary = binary.resolve(strict=True)
    report = {
        "format_version": 1, "binary_sha256": sha256(binary),
        "source_commit": os.environ.get("GITHUB_SHA"),
        "environment": {"os": platform.system(), "release": platform.release(),
                        "machine": platform.machine(), "cpu_count": os.cpu_count(),
                        "runner_os": os.environ.get("RUNNER_OS"),
                        "runner_arch": os.environ.get("RUNNER_ARCH"),
                        "runner_image": os.environ.get("ImageVersion")},
        "scope": "fresh native CLI child; load/layout/render/encode/write; no process-group or interaction claim",
        "filesystem_cache": "uncontrolled; first invocation is not a cold-cache measurement",
        "completion_poll_seconds": POLL_SECONDS,
        "warm_samples_per_scene": samples,
        "scenes": [],
    }
    with tempfile.TemporaryDirectory(prefix="lazypencil-native-benchmark-") as directory:
        root = Path(directory)
        environment = dict(os.environ, LAZYPENCIL_CONFIG_DIR=str(root / "user"))
        for name, design, assets in [("shadow", native.fixture(), {}),
                                     ("dashboard", dashboard.fixture(), {"avatar.png": dashboard.avatar()})]:
            scene = root / name
            scene.mkdir()
            (scene / "lazypencil.toml").write_bytes(b"version=1\n")
            document = scene / "design.pen"
            serialized = (json.dumps(design) if name == "shadow" else
                          json.dumps(design, ensure_ascii=False, separators=(",", ":")))
            document.write_bytes((serialized + "\n").encode())
            for filename, data in assets.items():
                (scene / filename).write_bytes(data)
            output = scene / "render.png"
            results, png_hash = [], None
            for _ in range(samples + 1):
                output.unlink(missing_ok=True)
                sample, raw = measure([str(binary), "--data-dir", str(root / "data"), "--json", "render", str(document),
                                       "--output", str(output)], environment, scene, timeout)
                actual = validate_render(raw, output, len(assets))
                if png_hash is not None and actual != png_hash:
                    raise ValueError("Repeated native exports produced different pixels")
                png_hash = actual
                results.append(sample)
            report["scenes"].append({
                "name": name, "document_sha256": sha256(document),
                "width": 1920, "height": 1080, "expanded_nodes": 1000,
                "diagnostics": 0, "dynamic_artifacts": 0, "script_executions": 0,
                "assets": {filename: sha256(scene / filename) for filename in assets},
                "png_sha256": png_hash, "first_invocation": results[0],
                "repeated": results[1:], "summary": summarize(results[1:]),
            })
    if sha256(binary) != report["binary_sha256"]:
        raise ValueError("The measured executable changed during the benchmark")
    return report


def publish(report, output):
    data = (json.dumps(report, ensure_ascii=False, indent=2) + "\n").encode()
    descriptor, name = tempfile.mkstemp(dir=output.parent, prefix=".native-report-")
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="Create a new JSON report")
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--timeout", type=float, default=30)
    args = parser.parse_args()
    try:
        if args.output.exists() or args.output.is_symlink():
            raise ValueError("Refusing to overwrite an existing benchmark report")
        report = benchmark(args.binary, args.samples, args.timeout)
        # Publish only a completed report, without replacing another writer.
        publish(report, args.output)
        print(json.dumps({"native_benchmark": "ok", "scenes": len(report["scenes"]),
                          "repeated_samples_per_scene": args.samples}))
    except (OSError, ValueError, TimeoutError) as error:
        parser.exit(1, f"Native measurement failed: {error}\n")


if __name__ == "__main__":
    main()

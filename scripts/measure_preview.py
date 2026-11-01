#!/usr/bin/env python3
"""Measure owned PTY image receipt; no terminal presentation timing is claimed."""

import argparse
import base64
import fcntl
import hashlib
import json
import math
import os
import pathlib
import platform
import pty
import select
import signal
import statistics
import struct
import termios
import time
import zlib
import subprocess
import shutil

ROOT = pathlib.Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def stats(values):
    values = sorted(values)
    return dict(
        n=len(values),
        min=min(values),
        median=statistics.median(values),
        p95=values[math.ceil(len(values) * 0.95) - 1],
        max=max(values),
    )


class Terminal:
    def __init__(
        self,
        binary,
        document,
        protocol,
        geometry,
        label,
        worker_profile=False,
        capture=False,
    ):
        self.capture = capture
        self.buf = b""
        self.payload = []
        self.fields = None
        self.exact_id = None
        self.frames = []
        self.pending = []
        self.total = 0
        self.observer_ns = 0
        self.root = ROOT / label
        self.root.mkdir()
        config = self.root / "config"
        config.mkdir()
        (config / "config.toml").write_text(
            '[fonts]\nfallbacks=["Noto Sans JP", "Apple Color Emoji", "Apple Symbols"]\n'
        )
        self.started = time.perf_counter_ns()
        pid, master = pty.fork()
        if pid == 0:
            fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", *geometry))
            err = os.open(
                str(self.root / "stderr.log"), os.O_WRONLY | os.O_CREAT, 0o600
            )
            os.dup2(err, 2)
            env = dict(os.environ)
            for key in [
                "TMUX",
                "WEZTERM_EXECUTABLE",
                "KONSOLE_VERSION",
                "TERM_PROGRAM",
                "GITHUB_ACTIONS",
                "LAZYPENCIL_PROFILE_PREVIEW",
            ]:
                env.pop(key, None)
            env.update(
                TERM="xterm-kitty" if protocol == "kitty" else "xterm-256color",
                COLORTERM="truecolor",
                LAZYPENCIL_CONFIG_DIR=str(config),
            )
            if worker_profile:
                env["LAZYPENCIL_PROFILE_PREVIEW"] = "owned-local-measurement"
            os.chdir(document.parent)
            args = [
                str(binary),
                "--data-dir",
                str(self.root / "data"),
                "tui",
                str(document),
                "--adapter",
                "/usr/bin/false",
                "--image-protocol",
                protocol,
            ]
            os.execve(binary, args, env)
        self.pid, self.master = pid, master
        os.set_blocking(master, False)

    def pump(self, timeout=0.02):
        ready = select.select([self.master], [], [], timeout)[0]
        if not ready:
            return
        data = os.read(self.master, 65536)
        now = time.perf_counter_ns()
        if not data:
            raise RuntimeError("PTY closed")
        self.total += len(data)
        if b"\x1b[6n" in data:
            os.write(self.master, b"\x1b[1;1R")
        self.buf += data
        parse_start = time.perf_counter_ns()
        markers = [b"\x1b_G", b"\x1bP", b"\x1b[?2026l"]
        while self.buf:
            starts = [(self.buf.find(m), m) for m in markers if self.buf.find(m) >= 0]
            if not starts:
                self.buf = self.buf[-16:]
                break
            offset, marker = min(starts)
            self.buf = self.buf[offset:]
            if marker == markers[2]:
                for frame in self.pending:
                    frame["sync_ns"] = now
                self.pending.clear()
                self.buf = self.buf[len(marker) :]
                continue
            end = self.buf.find(b"\x1b\\", len(marker))
            if end < 0:
                break
            packet = self.buf[len(marker) : end]
            self.buf = self.buf[end + 2 :]
            if marker == markers[0]:
                header, sep, payload = packet.partition(b";")
                fields = dict(x.split(b"=", 1) for x in header.split(b",") if b"=" in x)
                if fields.get(b"a") == b"T":
                    assert self.fields is None, "Interrupted Kitty frame"
                    self.fields = fields
                    self.payload = []
                    self.image_started = now
                if self.fields is not None and sep:
                    self.payload.append(payload)
                    if fields.get(b"m", b"0") == b"0":
                        image_fields = self.fields
                        image_id = int(image_fields[b"i"])
                        if self.exact_id is None:
                            self.exact_id = image_id
                        raw = base64.b64decode(b"".join(self.payload))
                        if image_fields.get(b"o") == b"z":
                            raw = zlib.decompress(raw)
                        width, height = int(image_fields[b"s"]), int(image_fields[b"v"])
                        assert len(raw) == width * height * 4
                        self.add_frame(
                            dict(
                                kind="exact" if image_id == self.exact_id else "cached",
                                width=width,
                                height=height,
                                compressed=image_fields.get(b"o") == b"z",
                                hash=hashlib.sha256(raw).hexdigest(),
                                received_ns=now,
                                first_byte_ns=self.image_started,
                            )
                        )
                        if self.capture:
                            (
                                self.root
                                / f'frame-{len(self.frames)}-{self.frames[-1]["kind"]}-{width}x{height}.rgba'
                            ).write_bytes(raw)
                        self.fields = None
                        self.payload = []
            else:
                header, sep, payload = packet.partition(b"q")
                if sep:
                    self.add_frame(
                        dict(
                            kind="exact",
                            hash=hashlib.sha256(packet).hexdigest(),
                            received_ns=now,
                            first_byte_ns=None,
                            wire_bytes=len(packet),
                        )
                    )
        self.observer_ns += time.perf_counter_ns() - parse_start

    def add_frame(self, frame):
        self.frames.append(frame)
        self.pending.append(frame)

    def wait_exact(self, after, timeout=45, expected_hash=None):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            result = [
                f
                for f in self.frames
                if f["kind"] == "exact" and f["received_ns"] >= after and "sync_ns" in f
            ]
            if result and (
                expected_hash is None or result[-1]["hash"] == expected_hash
            ):
                return result[-1]
            self.pump()
        raise TimeoutError(
            f"No complete exact frame; frames={len(self.frames)} bytes={self.total} pending={len(self.pending)} buf={len(self.buf)}"
        )

    def drain(self, seconds=0.1):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            self.pump(min(0.02, max(0, until - time.monotonic())))

    def key(self, data):
        sent = time.perf_counter_ns()
        os.write(self.master, data)
        return sent

    def interaction(self, action, key):
        self.drain(0.03)
        observer = self.observer_ns
        previous = [f for f in self.frames if f["kind"] == "exact"][-1]
        byte_start = self.total
        sent = self.key(key)
        frame = self.wait_exact(sent)
        cached = [
            f
            for f in self.frames
            if f["kind"] == "cached"
            and sent <= f["received_ns"] <= frame["received_ns"]
            and "sync_ns" in f
        ]
        return dict(
            action=action,
            complete_ms=(frame["received_ns"] - sent) / 1e6,
            complete_sync_ms=(frame["sync_ns"] - sent) / 1e6,
            feedback_ms=(
                min([frame["sync_ns"]] + [f["sync_ns"] for f in cached]) - sent
            )
            / 1e6,
            cached_ms=(cached[0]["sync_ns"] - sent) / 1e6 if cached else None,
            bytes=self.total - byte_start,
            changed=frame["hash"] != previous["hash"],
            hash=frame["hash"],
            observer_ms=(self.observer_ns - observer) / 1e6,
        )

    def close(self):
        self.key(b"\x03")
        deadline = time.monotonic() + 4
        while time.monotonic() < deadline:
            result = os.waitpid(self.pid, os.WNOHANG)
            if result[0]:
                os.close(self.master)
                return os.waitstatus_to_exitcode(result[1])
            try:
                self.pump(0.02)
            except (OSError, RuntimeError):
                pass
        os.killpg(self.pid, signal.SIGTERM)
        _, status = os.waitpid(self.pid, 0)
        os.close(self.master)
        return os.waitstatus_to_exitcode(status)


def main():
    global ROOT
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--document", type=pathlib.Path, required=True)
    parser.add_argument(
        "--binary",
        type=pathlib.Path,
        default=pathlib.Path(shutil.which("lazypencil") or "lazypencil"),
    )
    parser.add_argument("--protocol", choices=["kitty", "sixel"], default="kitty")
    parser.add_argument("--samples", type=int, default=20)
    parser.add_argument("--geometry", default="54,160,1920,1080")
    parser.add_argument("--label", default="measurement")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--median-limit-ms", type=float)
    parser.add_argument("--reference-report", type=pathlib.Path)
    parser.add_argument("--bursts", action="store_true")
    parser.add_argument("--profile", action="store_true")
    parser.add_argument("--worker-profile", action="store_true")
    parser.add_argument("--capture", action="store_true")
    args = parser.parse_args()
    if args.samples < 2 or args.samples > 200:
        parser.error("--samples must be between 2 and 200")
    if args.median_limit_ms is not None and (
        not math.isfinite(args.median_limit_ms)
        or args.median_limit_ms <= 0
        or args.profile
        or args.worker_profile
        or args.capture
    ):
        parser.error("A finite positive median limit requires an unprofiled run")
    if pathlib.Path(args.label).name != args.label or args.label in [".", ".."]:
        parser.error("--label must be a simple file name")
    ROOT = args.output.resolve()
    ROOT.mkdir(parents=True, exist_ok=False)
    reference = (
        json.loads(args.reference_report.read_text()) if args.reference_report else None
    )
    binary = args.binary.resolve()
    document = args.document.resolve()
    geometry = tuple(map(int, args.geometry.split(",")))
    if len(geometry) != 4 or any(n <= 0 or n > 65535 for n in geometry):
        parser.error(
            "--geometry requires four positive u16 values: rows,columns,pixel_width,pixel_height"
        )
    source_hash = digest(document)
    if reference is not None:
        assert reference["document_sha256"] == source_hash, "Reference document differs"
        assert list(reference["geometry"].values()) == list(
            geometry
        ), "Reference geometry differs"
        assert reference["protocol"] == args.protocol, "Reference protocol differs"
    binary_hash = digest(binary)
    report = dict(
        binary=str(binary),
        binary_sha256=binary_hash,
        document_sha256=source_hash,
        document_bytes=document.stat().st_size,
        protocol=args.protocol,
        geometry=dict(
            zip(["rows", "columns", "pixel_width", "pixel_height"], geometry)
        ),
        platform=platform.platform(),
        scope="Key write to complete image receipt / synchronized update end in owned PTY. No display emulator. Cached feedback separate. False adapter prevents agent or account work. OS caches and machine load uncontrolled.",
        samples=[],
    )
    terminal = Terminal(
        binary,
        document,
        args.protocol,
        geometry,
        args.label,
        args.worker_profile,
        args.capture,
    )
    try:
        first = terminal.wait_exact(terminal.started)
        report["first_preview_ms"] = (first["sync_ns"] - terminal.started) / 1e6
        report["frame"] = {
            k: first[k] for k in ["width", "height", "compressed"] if k in first
        }
        terminal.key(b"\t")
        terminal.drain(0.2)
        initial = first["hash"]
        report["initial_rgba_sha256"] = initial
        profiler = (
            subprocess.Popen(
                [
                    "/usr/bin/sample",
                    str(terminal.pid),
                    "5",
                    "1",
                    "-file",
                    str(terminal.root / "sample.txt"),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            if args.profile
            else None
        )
        for i in range(args.samples):
            trial = []
            for action, key in [
                ("zoom_in", b"+"),
                ("zoom_out", b"-"),
                ("pan_right", b"\x1b[C"),
                ("pan_left", b"\x1b[D"),
            ]:
                trial.append(terminal.interaction(action, key))
            assert (
                trial[1]["hash"] == initial and trial[3]["hash"] == initial
            ), "Original pixels not restored"
            assert all(t["changed"] for t in trial), "Unchanged frame"
            report["samples"] += trial
            if reference is not None:
                for actual, expected in zip(
                    trial, reference["samples"][i * 4 : i * 4 + 4], strict=True
                ):
                    assert (
                        actual["action"] == expected["action"]
                        and actual["hash"] == expected["hash"]
                    ), "Reference image pixels differ"
            print(
                json.dumps(
                    dict(
                        trial=i,
                        complete_ms=[round(t["complete_ms"], 2) for t in trial],
                        feedback_ms=[round(t["feedback_ms"], 2) for t in trial],
                    )
                ),
                flush=True,
            )
        report["statistics"] = {
            action: {
                field: stats(
                    [x[field] for x in report["samples"] if x["action"] == action]
                )
                for field in [
                    "complete_ms",
                    "complete_sync_ms",
                    "feedback_ms",
                    "bytes",
                    "observer_ms",
                ]
            }
            for action in ["zoom_in", "zoom_out", "pan_right", "pan_left"]
        }
        if profiler is not None:
            report["profiler_exit_code"] = profiler.wait(timeout=15)
        report["pixel_restoration_verified"] = True
        report["reference_pixels_verified"] = reference is not None
        if args.bursts:
            report["bursts"] = []
            for action, keys in [
                ("pan", [b"\x1b[C"] * 20 + [b"\x1b[D"] * 20),
                ("zoom", [b"+", b"-"] * 20),
            ]:
                terminal.drain(0.2)
                before = len(terminal.frames)
                begin = time.perf_counter_ns()
                sent = []
                for i, key in enumerate(keys):
                    due = begin + i * 50_000_000
                    while time.perf_counter_ns() < due:
                        terminal.pump(
                            max(0, min(0.005, (due - time.perf_counter_ns()) / 1e9))
                        )
                    sent.append(terminal.key(key))
                final = terminal.wait_exact(sent[-1], expected_hash=initial)
                terminal.drain(0.1)
                # A faster renderer can finish an older in-flight frame after the
                # last key. Retain the latest exact image at the final camera.
                final = terminal.wait_exact(sent[-1], expected_hash=initial)
                outputs = terminal.frames[before:]
                report["bursts"].append(
                    dict(
                        action=action,
                        keys=len(keys),
                        requested_interval_ms=50,
                        actual_key_interval_ms=stats(
                            [(b - a) / 1e6 for a, b in zip(sent, sent[1:])]
                        ),
                        cached_frames=len(
                            [f for f in outputs if f["kind"] == "cached"]
                        ),
                        exact_frames=len([f for f in outputs if f["kind"] == "exact"]),
                        last_input_to_exact_sync_ms=(final["sync_ns"] - sent[-1]) / 1e6,
                        pixel_restoration_verified=final["hash"] == initial,
                    )
                )
                assert final["hash"] == initial, "Burst did not restore pixels"

    finally:
        report["exit_code"] = terminal.close()
        report["worker_profiles"] = [
            json.loads(line.removeprefix("LAZYPENCIL_PREVIEW_TIMING "))
            for line in (terminal.root / "stderr.log").read_text().splitlines()
            if line.startswith("LAZYPENCIL_PREVIEW_TIMING ")
        ]
        report["document_unchanged"] = digest(document) == source_hash
        report["binary_unchanged"] = digest(binary) == binary_hash
        (ROOT / (args.label + ".json")).write_text(json.dumps(report, indent=2) + "\n")
    assert (
        report["exit_code"] == 0
        and report["document_unchanged"]
        and report["binary_unchanged"]
    )
    if args.median_limit_ms is not None:
        assert all(
            row["complete_ms"]["median"] <= args.median_limit_ms
            for row in report["statistics"].values()
        ), "Complete-image median exceeds limit; all observations retained"
    print(
        json.dumps(
            dict(
                report=str(ROOT / (args.label + ".json")),
                first_preview_ms=report["first_preview_ms"],
                statistics=report.get("statistics"),
                exit_code=report["exit_code"],
            )
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()

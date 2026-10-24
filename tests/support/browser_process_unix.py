"""A bounded browser fixture with a root, branch and leaf process."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

directory, mode, role = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
os.chdir(directory)
signal.signal(signal.SIGTERM, signal.SIG_IGN)


def publish(name, value):
    temporary = Path(name + ".tmp")
    temporary.write_text(json.dumps(value), encoding="utf-8")
    temporary.replace(name)


if role == "supervisor":
    import select
    if sys.stdin.buffer.read(8) != b"LPBO\x02\0\r\n":
        raise RuntimeError("Missing fixture owner")
    data = {"partial": b'{"pid":', "oversized": b'x' * 4097,
            "invalid": b'not-json\n', "eof": b''}[mode]
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()
    publish("status-ready.json", {})
    if mode != "eof":
        # An unbounded host read still fails within the fixture's three seconds.
        select.select([sys.stdin.buffer], [], [], 3)
    sys.exit(0)

if role == "root":
    profile = next(argument.split("=", 1)[1] for argument in sys.argv
                   if argument.startswith("--user-data-dir="))
    Path(profile, "fixture-cache").write_bytes(b"owned-profile\n")
    publish("browser-profile.json", {"path": profile})

if role == "leaf":
    publish("browser-leaf.json", {"pid": os.getpid()})
    if mode == "adopt":
        guardian = int(sys.argv[5])
        end = time.monotonic() + 5
        while os.getppid() != guardian:
            if time.monotonic() >= end:
                raise RuntimeError("Browser orphan was not adopted by its guardian")
            time.sleep(.005)
        publish("adopted.json", {"pid": os.getpid(), "parent": os.getppid()})
        os._exit(0)
else:
    root = os.getpid() if role == "root" else int(sys.argv[4])
    guardian = os.getppid() if role == "root" else int(sys.argv[5])
    child = subprocess.Popen(
        [sys.executable, __file__, str(directory), mode,
         "branch" if role == "root" else "leaf", str(root), str(guardian)],
        stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=mode == "detach",
    )
    if role == "branch":
        end = time.monotonic() + 5
        while not Path("browser-leaf.json").exists():
            if time.monotonic() >= end or child.poll() is not None:
                raise RuntimeError("Browser fixture leaf did not become ready")
            time.sleep(.005)
        publish("browser-tree.json", {"root": root, "branch": os.getpid(),
            "leaf": child.pid, "guardian": guardian})
        if mode == "adopt":
            os._exit(0)
    elif mode == "exit":
        end = time.monotonic() + 5
        while not Path("browser-tree.json").exists():
            if time.monotonic() >= end or child.poll() is not None:
                raise RuntimeError("Browser fixture branch did not become ready")
            time.sleep(.005)
        os._exit(91)

# A failed test still leaves a bounded fixture, without signalling recycled PIDs.
time.sleep(60)

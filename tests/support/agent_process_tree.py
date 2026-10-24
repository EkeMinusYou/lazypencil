"""Owned three-level process fixture for adapter supervision tests."""

import json
import os
from pathlib import Path
import subprocess
import sys
import time


def publish(name, value):
    temporary = Path(name + ".tmp")
    temporary.write_text(json.dumps(value), encoding="utf-8")
    temporary.replace(name)


if sys.argv[1] == "leaf":
    publish("tree-leaf-ready.json", {"pid": os.getpid()})
    time.sleep(60)
else:
    flags = subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS if os.name == "nt" else 0
    leaf = subprocess.Popen([sys.executable, __file__, "leaf"], stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=flags,
        start_new_session=(os.name != "nt" and "--detach" in sys.argv[3:]))
    try:
        deadline = time.monotonic() + 10
        while not Path("tree-leaf-ready.json").exists():
            if time.monotonic() >= deadline or leaf.poll() is not None:
                raise RuntimeError("Owned leaf process did not become ready")
            time.sleep(0.01)
        publish("tree-ready.json", {"adapter": int(sys.argv[2]), "branch": os.getpid(), "leaf": leaf.pid})
        time.sleep(60)
    finally:
        if leaf.poll() is None:
            leaf.terminate()
        leaf.wait(timeout=5)

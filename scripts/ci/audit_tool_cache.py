"""Bind the fixed cargo-deny binary to its compiler and hosted runner image."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


VERSION = "0.20.2"


def identity(compiler, image, architecture):
    if not image or image == "unknown" or not architecture:
        raise ValueError("A known hosted runner image and architecture are required")
    payload = {"compiler": compiler, "image": image, "architecture": architecture, "version": VERSION}
    return hashlib.sha256(json.dumps(payload, sort_keys=True).encode()).hexdigest()


def record(directory, fingerprint):
    binary = directory / "bin/cargo-deny"
    with binary.open("rb") as stream:
        checksum = hashlib.file_digest(stream, "sha256").hexdigest()
    version = subprocess.run([str(binary), "--version"], check=True, capture_output=True, text=True, timeout=10).stdout.strip()
    if version != f"cargo-deny {VERSION}":
        raise ValueError("Unexpected cargo-deny version")
    return {"fingerprint": fingerprint, "sha256": checksum, "version": version}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("identify", "verify", "seal"))
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    compiler = subprocess.run(["rustc", "+stable", "-vV"], check=True, capture_output=True, text=True, timeout=15).stdout
    fingerprint = identity(compiler, os.environ.get("ImageVersion"), os.environ.get("RUNNER_ARCH"))
    manifest = args.directory / "manifest.json"
    if args.operation == "identify":
        key = f"cargo-deny-v1-{VERSION}-{fingerprint}"
        if args.github_output:
            with args.github_output.open("a", encoding="utf-8") as stream:
                stream.write(f"key={key}\n")
        print(key)
    elif args.operation == "seal":
        manifest.write_text(json.dumps(record(args.directory, fingerprint), sort_keys=True), encoding="utf-8")
    elif json.loads(manifest.read_text(encoding="utf-8")) != record(args.directory, fingerprint):
        raise SystemExit("Cached cargo-deny identity or binary hash changed")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Prepare checksum-pinned, original Material fonts for hosted opt-in tests."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import urllib.error
import urllib.parse
import urllib.request

STYLES = ("Outlined", "Rounded", "Sharp")
MAX_BYTES = 16 * 1024 * 1024


def digest(stream, sink=None):
    result = hashlib.sha256()
    total = 0
    while chunk := stream.read(64 * 1024):
        total += len(chunk)
        if total > MAX_BYTES:
            raise ValueError("A Material font exceeds 16 MiB")
        result.update(chunk)
        if sink is not None:
            sink.write(chunk)
    return result.hexdigest(), total


def open_source(request):
    return urllib.request.urlopen(request, timeout=30)


def prepare(catalogue, output, opener=open_source):
    records = json.loads(catalogue.read_text(encoding="utf-8"))
    if output.is_symlink():
        raise ValueError("The font output directory must not be a symlink")
    output.mkdir(parents=True, exist_ok=True)
    result = []
    for style in STYLES:
        name = f"Material Symbols {style}"
        selected = [item for item in records if item["id"] == name]
        if len(selected) != 1:
            raise ValueError(f"Expected exactly one catalogue record for {name}")
        record = selected[0]
        version, expected = record["version"], record["source_sha256"]
        filename = f"MaterialSymbols{style}[FILL,GRAD,opsz,wght].ttf"
        source = (
            f"https://raw.githubusercontent.com/google/material-design-icons/{version}"
            f"/variablefont/{urllib.parse.quote(filename, safe='')}"
        )
        if (
            record.get("source_format") != "ttf"
            or not re.fullmatch(r"[0-9a-f]{40}", version)
            or not re.fullmatch(r"[0-9a-f]{64}", expected)
            or record["source_url"] != source
        ):
            raise ValueError(f"Invalid immutable source pin for {name}")
        target = output / filename
        if target.is_symlink():
            raise ValueError("A cached font must not be a symlink")
        reused = target.exists()
        if reused:
            with target.open("rb") as stream:
                actual, size = digest(stream)
        else:
            temporary = None
            try:
                with tempfile.NamedTemporaryFile(dir=output, prefix=".font-", delete=False) as stage:
                    temporary = Path(stage.name)
                    request = urllib.request.Request(source, headers={"User-Agent": "lazypencil-ci"})
                    with opener(request) as response:
                        final = urllib.parse.urlsplit(response.geturl())
                        if final.scheme != "https" or final.hostname != "raw.githubusercontent.com":
                            raise ValueError("Unexpected Material font redirect")
                        actual, size = digest(response, stage)
                if actual != expected:
                    raise ValueError(f"Original font checksum mismatch for {name}")
                os.replace(temporary, target)
            finally:
                if temporary is not None:
                    temporary.unlink(missing_ok=True)
        if actual != expected:
            raise ValueError(f"Cached font checksum mismatch for {name}")
        result.append({"pack": name, "version": version, "filename": filename,
                       "sha256": actual, "bytes": size, "reused": reused})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    catalogue = Path(__file__).resolve().parents[2] / "resources/icons/catalogue.json"
    try:
        print(json.dumps({"fonts": prepare(catalogue, args.output)}))
    except (OSError, ValueError, KeyError, TypeError, urllib.error.URLError) as error:
        parser.exit(1, f"Material source preparation failed: {error}\n")


if __name__ == "__main__":
    main()

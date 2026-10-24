"""Collect the standalone Wasm crate's exact locked package notices on GHA."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import tomllib

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import license_notices as notices


def collect(root, output):
    notices.require(os.environ.get("GITHUB_ACTIONS") == "true", "Run collection on GitHub Actions")
    component = root / "src/shader/preprocessor"
    packages = tomllib.loads(notices.read(component / "Cargo.lock").decode())["package"]
    selected = {notices.identifier(package) for package in packages}
    staging = output.parent / (output.name + "-inputs")
    staging.mkdir()
    records = {}
    references = {}
    for directory in [root / "resources/licenses/rust", component / "notices"]:
        policy = json.loads(notices.read(directory / "upstream.json"))
        for item in policy["license_references"]:
            previous = references.setdefault(item["id"], item)
            notices.require(previous == item, "Conflicting reference license")
            data = notices.stored_blob(directory, item)
            destination = staging / item["blob"]
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
        for package in policy["packages"]:
            key = notices.identifier(package)
            if key not in selected:
                continue
            notices.require(key not in records, "Duplicate upstream package input: " + key)
            records[key] = package
            for item in package["files"]:
                data = notices.stored_blob(directory, item)
                destination = staging / item["blob"]
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
    policy = {"format_version": 1, "packages": list(records.values()), "license_references": list(references.values())}
    upstream = staging / "upstream.json"
    upstream.write_bytes(notices.json_bytes(policy))
    result = notices.collect(component, Path(os.environ["CARGO_HOME"]), upstream, output)
    notices.verify(component, output, Path(os.environ["CARGO_HOME"]))
    return {**result, "component_lock_sha256": hashlib.sha256(notices.read(component / "Cargo.lock")).hexdigest()}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(json.dumps(collect(args.root, args.output)))
    except (notices.NoticeError, OSError, ValueError, KeyError) as error:
        parser.exit(1, "Shader notice error: " + str(error) + "\n")

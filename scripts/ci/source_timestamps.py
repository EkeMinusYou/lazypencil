"""Keep verified archive extraction times from invalidating hosted Cargo caches."""

import argparse
import json
import os
from pathlib import Path
import runpy


# This applies only to an isolated, verified distribution, never the checkout.
# Native keys bind the patched dependency tree; application keys bind compiler
# inputs and validate dep-info and output hashes before reuse. Cargo still
# checks features, profiles, compiler settings and dependency fingerprints.
SOURCE_TIME_NS = 946684800 * 1_000_000_000
DISTRIBUTION = runpy.run_path(str(Path(__file__).resolve().parents[1] / "dist.py"))


def prepare(source):
    source = source.absolute()
    if source.is_symlink() or not source.is_dir() or (source / ".git").exists():
        raise ValueError("Expected an extracted source distribution, not a checkout")
    dist = DISTRIBUTION
    manifest = source / dist["MANIFEST"]
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > dist["MAX_MANIFEST"]:
        raise ValueError("Expected a regular, bounded source manifest")
    entries = json.loads(manifest.read_text(encoding="utf-8"))["files"]
    actual = dist["source_files"](source)
    if not isinstance(entries, list) or len(entries) != len(actual):
        raise ValueError("Source distribution file inventory changed")
    seen = set()
    for entry in entries:
        name = dist["safe_path"](entry["path"])
        if name in seen or name not in actual:
            raise ValueError("Source distribution file inventory changed")
        seen.add(name)
        data, mode = actual[name]
        if len(data) != entry["bytes"] or dist["digest"](data) != entry["sha256"] or mode != entry["mode"]:
            raise ValueError(f"Source distribution input changed: {name}")
    # Validate the entire tree before changing any timestamp. Directories also
    # matter to rerun-if-changed rules in native dependency build scripts.
    directories = {source}
    for name in seen:
        path = source / name
        directories.update(parent for parent in path.parents if parent == source or source in parent.parents)
        os.utime(path, ns=(SOURCE_TIME_NS, SOURCE_TIME_NS), follow_symlinks=False)
    os.utime(manifest, ns=(SOURCE_TIME_NS, SOURCE_TIME_NS), follow_symlinks=False)
    for path in sorted(directories, key=lambda path: len(path.parts), reverse=True):
        os.utime(path, ns=(SOURCE_TIME_NS, SOURCE_TIME_NS), follow_symlinks=False)
    return {"files": len(seen), "source_mtime_ns": SOURCE_TIME_NS}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.source), sort_keys=True))


if __name__ == "__main__":
    main()

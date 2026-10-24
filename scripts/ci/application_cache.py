"""Reuse package outputs only for identical source inputs and hosted build identity."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import shutil
import tomllib


class CacheError(Exception):
    pass


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def identity(source, target, native_key):
    manifest = json.loads((source / "SOURCE-MANIFEST.json").read_text(encoding="utf-8"))
    package = tomllib.loads((source / "Cargo.toml").read_text(encoding="utf-8"))["package"]
    if (source / "build.rs").exists() or package.get("build", False):
        raise CacheError("Custom application build scripts require a fresh application build")
    platform = re.match(r"native-v2-(ubuntu-24\.04-arm|ubuntu-24\.04|macos-15)-", native_key)
    if package["name"] != "lazypencil" or not platform:
        raise CacheError("Unknown package or hosted build identity")
    # Workflow scheduling cannot change compiled bytes. If Rust includes a
    # workflow as data, dep-info below explicitly disables application reuse.
    inputs = [entry for entry in manifest["files"] if not entry["path"].startswith(".github/workflows/")]
    payload = {"version": 1, "inputs": inputs, "native": native_key,
               "source": str(source.resolve()), "target": str(target.resolve())}
    fingerprint = digest(json.dumps(payload, sort_keys=True).encode())
    return {"fingerprint": fingerprint, "source_commit": manifest["git"]["revision"],
            "primary": f"application-v1-{platform[1]}-{fingerprint}-full",
            "checkpoint": f"application-v1-{platform[1]}-{fingerprint}-focus",
            "tracked": {entry["path"]: entry for entry in inputs}}


def allowed(name):
    path = PurePosixPath(name)
    if path.is_absolute() or ".." in path.parts or len(path.parts) < 2:
        return False
    profile, *parts = path.parts
    if profile not in ("debug", "release"):
        return False
    if len(parts) == 1:
        return bool(re.fullmatch(r"(?:lazypencil|liblazypencil\.(?:rlib|rmeta|dylib|so))", parts[0]))
    if parts[0] == ".fingerprint" and len(parts) == 3:
        return bool(re.fullmatch(r"lazypencil-[0-9a-f]{16}", parts[1]))
    if parts[0] == "deps" and len(parts) == 2:
        return bool(re.fullmatch(r"(?:lib)?[A-Za-z0-9_]+-[0-9a-f]{16}(?:\.(?:d|rlib|rmeta|dylib|so))?", parts[1]))
    return False


def outputs(target):
    result = set()
    for profile in ("debug", "release"):
        root = target / profile
        for directory in (root / ".fingerprint").glob("lazypencil-*"):
            suffix = directory.name.removeprefix("lazypencil-")
            if not re.fullmatch(r"[0-9a-f]{16}", suffix):
                continue
            for marker in directory.iterdir():
                match = re.fullmatch(r"(?:test-integration-test-|test-bin-|test-lib-|bin-|lib-)([A-Za-z0-9_-]+)\.json", marker.name)
                if match:
                    name = match[1].replace("-", "_")
                    for prefix in (name, "lib" + name):
                        result.update((root / "deps").glob(f"{prefix}-{suffix}*"))
            result.update(directory.iterdir())
        for name in ("lazypencil", "liblazypencil.rlib", "liblazypencil.rmeta", "liblazypencil.dylib", "liblazypencil.so"):
            if (root / name).exists():
                result.add(root / name)
    files = sorted(result)
    if not files or any(path.is_symlink() or not path.is_file() or not allowed(path.relative_to(target).as_posix()) for path in files):
        raise CacheError("Application outputs are missing or contain unsupported files")
    return files


def input_environment(files, source, tracked):
    environment = {}
    checked = set()
    dependency_files = [path for path in files if path.suffix == ".d"]
    if not dependency_files:
        raise CacheError("Application reuse requires compiler dep-info")
    for path in dependency_files:
        for line in path.read_text(encoding="utf-8").replace("\\\n", "").splitlines():
            if line.startswith("# env-dep:"):
                name = line.removeprefix("# env-dep:").split("=", 1)[0]
                # Cargo supplies and validates its own package/target variables.
                if not name.startswith("CARGO_"):
                    environment[name] = digest(json.dumps(os.environ.get(name)).encode())
            elif line and not line.startswith("#"):
                if ": " not in line:
                    continue  # Empty Make prerequisites have no input paths.
                for token in shlex.split(line.split(": ", 1)[1]):
                    dependency = Path(token)
                    if not dependency.is_absolute():
                        dependency = source / dependency
                    try:
                        relative = dependency.resolve().relative_to(source.resolve()).as_posix()
                    except ValueError as error:
                        raise CacheError("Application dep-info references an external input") from error
                    if relative not in tracked:
                        raise CacheError("Application dep-info references an unbound source input")
                    if relative not in checked and (dependency.is_symlink() or file_digest(dependency) != tracked[relative]["sha256"]):
                        raise CacheError("Compiler source input differs from the verified archive")
                    checked.add(relative)
    return environment


def stage(source, target, directory, expected):
    files = outputs(target)
    environment = input_environment(files, source, expected["tracked"])
    if directory.exists():
        shutil.rmtree(directory)
    directory.mkdir(parents=True)
    entries = []
    for path in files:
        name = path.relative_to(target).as_posix()
        destination = directory / "files" / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        os.link(path, destination)
        entries.append({"path": name, "sha256": file_digest(path), "bytes": path.stat().st_size,
                        "mode": path.stat().st_mode & 0o777})
    report = {key: expected[key] for key in ("fingerprint", "source_commit")}
    report.update(environment=environment, files=entries)
    (directory / "manifest.json").write_text(json.dumps(report, sort_keys=True), encoding="utf-8")
    return {"files": len(entries), "bytes": sum(entry["bytes"] for entry in entries)}


def restore(source, target, directory, expected):
    report = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    if report["fingerprint"] != expected["fingerprint"]:
        raise CacheError("Application inputs or build environment changed")
    files = []
    names = set()
    for entry in report["files"]:
        name = entry["path"]
        path = directory / "files" / name
        if not allowed(name) or name in names or path.is_symlink() or not path.is_file() or path.resolve() != (directory.resolve() / "files" / name):
            raise CacheError("Application cache contains an unsafe path")
        if path.stat().st_size != entry["bytes"] or path.stat().st_mode & 0o777 != entry["mode"] or file_digest(path) != entry["sha256"]:
            raise CacheError("Application output integrity check failed")
        names.add(name)
        files.append(path)
    if not files or input_environment(files, source, expected["tracked"]) != report["environment"]:
        raise CacheError("Compiler input environment changed")
    for path in files:
        relative = path.relative_to(directory / "files")
        destination = target / relative
        if destination.is_symlink() or destination.resolve() != target.resolve() / relative:
            raise CacheError("Application destination must not traverse a symlink")
    # Validate everything before changing the target. Cargo still checks its
    # feature/profile fingerprints and runs every requested test after reuse.
    for path in files:
        destination = target / path.relative_to(directory / "files")
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.unlink(missing_ok=True)
        # Both directories live on RUNNER_TEMP. Links avoid a second physical
        # copy; later Cargo writes cannot modify the remote cache archive.
        os.link(path, destination)
    return {"reused": True, "files": len(files), "producer": report["source_commit"], "source": expected["source_commit"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("identify", "stage", "restore"))
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--native-key", required=True)
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    try:
        expected = identity(args.source, args.target, args.native_key)
        if args.operation == "identify":
            report = {key: expected[key] for key in ("primary", "checkpoint", "source_commit")}
            if args.github_output:
                with args.github_output.open("a", encoding="utf-8") as stream:
                    for key in ("primary", "checkpoint"):
                        stream.write(f"{key}={expected[key]}\n")
        elif args.operation == "stage":
            report = stage(args.source, args.target, args.directory, expected)
        else:
            report = restore(args.source, args.target, args.directory, expected)
        print(json.dumps(report, sort_keys=True))
    except (CacheError, OSError, ValueError, KeyError, TypeError) as error:
        raise SystemExit(f"Application cache unavailable: {error}") from error


if __name__ == "__main__":
    main()

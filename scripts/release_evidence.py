"""Bind hosted release build observations to verified distributed sources.

This collects review evidence, not a releasable binary or a license approval.
Cargo build-script declarations do not prove which archive members were linked.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tempfile

import dist
import license_notices as notices
import skia_notice_inventory as native


MAX_EVENTS = 64 * 1024 * 1024
MAX_LINE = 4 * 1024 * 1024
GAPS = [
    "Cargo declarations and Skia graph inputs do not establish final binary linkage or used archive members.",
    "Rust standard-library and system-library attribution must be joined to this exact build separately.",
    "QuickJS and the skia-bindings wrapper need full native-source attribution review beyond package notices.",
    "Fonts, icons, other bundled assets, and optional runtimes need separate applicability review.",
    "Complete legal review, signed build provenance, packaging, signing, and publication remain required.",
]


def inside(path, root):
    path = Path(path)
    notices.require(path.is_absolute(), "Build evidence requires absolute paths")
    resolved = path.resolve(strict=True)
    notices.require(resolved.is_relative_to(root) and resolved == Path(os.path.abspath(path)),
                    "Build input escapes or aliases its selected root: " + str(path))
    return resolved


def fingerprint(path, name, maximum=512 * 1024 * 1024):
    notices.require(path.is_file() and not path.is_symlink(), "Expected a regular evidence input: " + str(path))
    size, hasher = 0, hashlib.sha256()
    with path.open("rb") as stream:
        while data := stream.read(1024 * 1024):
            size += len(data)
            notices.require(size <= maximum, "Evidence input exceeds its byte limit: " + name)
            hasher.update(data)
    return {"path": name, "bytes": size, "sha256": hasher.hexdigest()}


def source_identity(root, revision):
    notices.require(re.fullmatch(r"[0-9a-f]{40}", revision), "Expected a full Git revision")
    data = notices.read(root / dist.MANIFEST, dist.MAX_MANIFEST)
    manifest = json.loads(data)
    notices.require(isinstance(manifest, dict) and manifest.get("format_version") == 1,
                    "Unsupported distributed source manifest")
    notices.require(manifest.get("git") == {"revision": revision, "dirty": False},
                    "Release evidence requires the expected clean source revision")
    entries = manifest.get("files")
    notices.require(isinstance(entries, list) and 1 <= len(entries) <= dist.MAX_FILES,
                    "Invalid source file inventory")
    seen, total = set(), 0
    for entry in entries:
        notices.require(isinstance(entry, dict) and set(entry) == {"path", "bytes", "sha256", "mode"},
                        "Invalid source file entry")
        name = dist.safe_path(entry["path"])
        key = dist.path_key(name)
        notices.require(key not in seen and name != dist.MANIFEST, "Duplicate or reserved source path")
        seen.add(key)
        notices.require(type(entry["bytes"]) is int and 0 <= entry["bytes"] <= dist.MAX_BYTES
                        and type(entry["mode"]) is int and entry["mode"] in (0o644, 0o755)
                        and isinstance(entry["sha256"], str) and notices.SHA.fullmatch(entry["sha256"]),
                        "Invalid source file fingerprint")
        total += entry["bytes"]
        notices.require(total <= dist.MAX_BYTES, "Source inventory exceeds its byte limit")
        actual = fingerprint(inside(root / name, root), name, dist.MAX_BYTES)
        notices.require(actual == {k: entry[k] for k in ("path", "bytes", "sha256")},
                        "Distributed source changed: " + name)
    notices.require({dist.path_key(name) for name in ("Cargo.toml", "Cargo.lock", "LICENSE", "src/main.rs")}
                    <= seen, "Missing required distributed source bindings")
    actual_names = set()

    def visit(directory):
        for path in directory.iterdir():
            relative = path.relative_to(root)
            if dist.excluded(PurePosixPath(relative.as_posix())) or relative.as_posix() == dist.MANIFEST:
                continue
            notices.require(not path.is_symlink(), "Linked distributed source input")
            if path.is_dir():
                visit(path)
            else:
                notices.require(path.is_file(), "Nonregular distributed source input")
                actual_names.add(dist.path_key(relative.as_posix()))
                notices.require(len(actual_names) <= dist.MAX_FILES, "Too many distributed source inputs")

    visit(root)
    notices.require(actual_names == seen, "Unlisted or missing distributed source inputs")
    return {"revision": revision, "manifest_sha256": notices.digest(data), "files": len(entries),
            "cargo_lock": fingerprint(root / "Cargo.lock", "Cargo.lock"),
            "license": fingerprint(root / "LICENSE", "LICENSE"), "package": manifest["package"]}


def cargo_observations(events, root, target):
    artifacts, builds, finished, ignored = [], {}, [], 0
    total = 0
    notices.require(events.is_file() and not events.is_symlink(), "Expected regular Cargo JSON output")
    with events.open("rb") as stream:
        while line := stream.readline(MAX_LINE + 1):
            total += len(line)
            notices.require(len(line) <= MAX_LINE and total <= MAX_EVENTS, "Cargo event byte limit exceeded")
            try:
                item = json.loads(line)
            except (ValueError, UnicodeError):
                ignored += 1
                continue
            if not isinstance(item, dict):
                ignored += 1
                continue
            reason = item.get("reason")
            notices.require(not finished, "Cargo records follow its completion record")
            if reason == "compiler-artifact":
                artifacts.append(item)
            elif reason == "build-script-executed":
                key = item["package_id"]
                # Cargo can build unrelated packages for several feature/host
                # contexts. Require uniqueness only for each selected native lib.
                builds.setdefault(key, []).append(item)
            elif reason == "build-finished":
                finished.append(item.get("success"))
    notices.require(finished == [True], "Cargo release build did not finish successfully")
    binaries = [a for a in artifacts if a.get("target", {}).get("name") == "lazypencil"
                and a["target"].get("kind") == ["bin"] and a.get("executable") is not None]
    notices.require(len(binaries) == 1, "Expected exactly one lazypencil executable")
    binary = binaries[0]
    notices.require(Path(binary["manifest_path"]).resolve(strict=True) == root / "Cargo.toml"
                    and binary.get("profile", {}).get("test") is False
                    and binary["profile"].get("opt_level") == "3" and binary.get("fresh") is False,
                    "Expected a newly compiled release executable from the distributed root")
    executable = inside(binary["executable"], target)
    notices.require(executable.parent == target / "release" and executable.name in {"lazypencil", "lazypencil.exe"},
                    "Unexpected release executable location")
    selected = {}
    for name in ("skia_bindings", "rquickjs_sys"):
        matches = [a for a in artifacts if a.get("target", {}).get("name") == name
                   and a["target"].get("kind") == ["lib"] and a.get("profile", {}).get("test") is False]
        notices.require(len(matches) == 1, "Missing or ambiguous native package artifact: " + name)
        artifact = matches[0]
        candidates = builds.get(artifact["package_id"], [])
        notices.require(len(candidates) == 1, "Missing or ambiguous selected Cargo build-script output: " + name)
        build = candidates[0]
        out = inside(build["out_dir"], target)
        notices.require(out.is_dir() and out.is_relative_to(target / "release" / "build"),
                        "Native package is outside the release build directory")
        libraries, searches = build["linked_libs"], build["linked_paths"]
        notices.require(isinstance(libraries, list) and all(isinstance(v, str) and 0 < len(v) <= 1024 for v in libraries)
                        and isinstance(searches, list) and all(isinstance(v, str) and 0 < len(v) <= 4096 for v in searches),
                        "Invalid native link declarations")
        selected[name] = {"package_id": artifact["package_id"], "manifest_path": artifact["manifest_path"],
                          "features": artifact["features"], "linked_libs": libraries, "linked_paths": searches,
                          "out_dir": out.relative_to(target).as_posix()}
    return executable, selected, ignored


def native_archives(target, selected):
    result = {}
    expected = {"skia_bindings": set(native.TARGETS) | {"skia-bindings"}, "rquickjs_sys": {"quickjs"}}
    for name, record in selected.items():
        out = target / record["out_dir"]
        directory = inside(out / "skia" if name == "skia_bindings" else out, target)
        declared = {value.split("=", 1)[-1] for value in record["linked_libs"]}
        notices.require(expected[name] <= declared, "Missing expected native library declarations: " + name)
        # Do not silently reuse the fixed Skia graph inventory after feature changes.
        if name == "skia_bindings":
            notices.require(not ({"svg", "skresources"} & declared), "Review new Skia graph targets")
        searches = [value.split("=", 1)[-1] for value in record["linked_paths"]]
        notices.require(any(Path(value).resolve(strict=True) == directory for value in searches),
                        "Native library directory was not declared to Cargo: " + name)
        files = []
        for library in sorted(expected[name]):
            candidates = [directory / ("lib" + library + ".a"), directory / (library + ".lib")]
            candidates = [p for p in candidates if p.exists() or p.is_symlink()]
            notices.require(candidates, "Missing native archive: " + library)
            observed = []
            for candidate in candidates:
                path = inside(candidate, target)
                observed.append(fingerprint(path, path.relative_to(target).as_posix()))
            # cc creates both GNU and MSVC names for the same archive on MSVC.
            notices.require(len({(v["sha256"], v["bytes"]) for v in observed}) == 1,
                            "Conflicting native archive aliases: " + library)
            files.extend(observed)
        result[name] = files
    return result


def collect(root, target, events, revision, output, rustc="rustc", ninja="ninja"):
    root, target = root.resolve(strict=True), target.resolve(strict=True)
    output = output.absolute()
    notices.require(not output.exists() and not output.is_symlink(), "Evidence output must not exist")
    notices.require(not output.is_relative_to(root) and not output.is_relative_to(target),
                    "Evidence output must be outside the source and build roots")
    source = source_identity(root, revision)
    cargo_output = fingerprint(events, "cargo-build.jsonl", MAX_EVENTS)
    executable, selected, ignored = cargo_observations(events, root, target)
    binary = fingerprint(executable, executable.relative_to(target).as_posix())
    archives = native_archives(target, selected)
    rust = notices.verify(root, root / "resources/licenses/rust")
    compiler = subprocess.run([rustc, "-vV"], capture_output=True, check=True, timeout=30).stdout
    notices.require(len(compiler) <= 65536, "Compiler identity exceeds its byte limit")
    skia_source = inside(root / "vendor/skia-bindings/skia", root)
    skia_build = target / selected["skia_bindings"]["out_dir"] / "skia"
    upstream = root / "resources/licenses/native/upstream.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="release-evidence-", dir=output.parent) as temporary:
        stage = Path(temporary) / "evidence"
        stage.mkdir()
        skia = native.collect(skia_source, skia_build, stage / "skia", ninja, upstream)
        native.verify(skia_source, skia_build, stage / "skia", ninja, upstream)
        notices.require(source_identity(root, revision) == source, "Sources changed during evidence collection")
        notices.require(fingerprint(executable, binary["path"]) == binary
                        and native_archives(target, selected) == archives, "Build outputs changed during collection")
        notices.require(fingerprint(events, "cargo-build.jsonl", MAX_EVENTS) == cargo_output,
                        "Cargo observations changed during collection")
        shutil.copytree(root / "resources/licenses/rust", stage / "rust")
        notices.verify(root, stage / "rust")
        (stage / "SOURCE-MANIFEST.json").write_bytes(notices.read(root / dist.MANIFEST, dist.MAX_MANIFEST))
        (stage / "LICENSE").write_bytes(notices.read(root / "LICENSE"))
        metadata = {"format_version": 1, "release_ready": False, "source": source, "binary": binary,
                    "compiler": compiler.decode("utf-8", "strict"), "native_packages": selected,
                    "native_archives": archives, "rust_notices": rust, "skia_notices": skia,
                    "cargo_output": cargo_output,
                    "ignored_non_cargo_lines": ignored, "remaining_review": GAPS,
                    "hosted": {name: os.environ.get(name) for name in
                               ("GITHUB_REPOSITORY", "GITHUB_SHA", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT",
                                "RUNNER_OS", "RUNNER_ARCH", "ImageOS", "ImageVersion")}}
        (stage / "build.json").write_bytes(notices.json_bytes(metadata))
        files = [fingerprint(path, path.relative_to(stage).as_posix(), 64 * 1024 * 1024)
                 for path in sorted(stage.rglob("*")) if path.is_file()]
        (stage / "manifest.json").write_bytes(notices.json_bytes({"format_version": 1, "files": files}))
        # Reserve exclusively so an existing output is never replaced, including on POSIX.
        output.mkdir(mode=0o700)
        try:
            for child in stage.iterdir():
                child.rename(output / child.name)
        except BaseException:
            shutil.rmtree(output)
            raise
    return {"release_ready": False, "source_revision": revision, "binary_sha256": binary["sha256"],
            "files": len(files) + 1, "remaining_review": GAPS}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--cargo-json", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--ninja", default="ninja")
    args = parser.parse_args()
    try:
        print(json.dumps(collect(args.source, args.target, args.cargo_json, args.revision,
                                 args.output, args.rustc, args.ninja)))
    except (notices.NoticeError, dist.DistributionError, OSError, ValueError, KeyError, TypeError,
            subprocess.SubprocessError) as error:
        parser.exit(1, "Release evidence error: " + str(error) + "\n")


if __name__ == "__main__":
    main()

"""Inventory original Skia notices against an existing native build (Python 3.11+).

This reads Ninja's graph and recorded headers without building anything. It is
an audit input, not a certification of binary linkage or license compliance.
"""

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

import license_notices as notices


TARGETS = ("skia", "skparagraph", "skshaper", "skunicode_core", "skunicode_icu")
EXTERNALS = {"expat", "harfbuzz", "icu", "libjpeg-turbo", "libpng", "libwebp", "wuffs", "zlib", "freetype"}
WRAPPERS = EXTERNALS | {"freetype2", "icu_bidi"}
MAX_QUERY = 256 * 1024 * 1024
MAX_SOURCE = 32 * 1024 * 1024
COMMENTS = re.compile(r"^[ \t]*/\*.*?\*/|^(?:[ \t]*//[^\n]*(?:\n|$))+", re.M | re.S)
REFERENCES = re.compile(r"Chromium source repository LICENSE", re.I)


def query(build, arguments, ninja):
    # A file avoids keeping the large recorded-header report in RAM.
    with tempfile.TemporaryFile() as stream:
        result = subprocess.run([ninja, "-C", str(build), "-t", *arguments], stdout=stream,
                                stderr=subprocess.PIPE, timeout=60, check=False)
        notices.require(result.returncode == 0, "Ninja inventory query failed: " + result.stderr.decode("utf-8", "replace")[:2048])
        notices.require(stream.tell() <= MAX_QUERY, "Ninja inventory exceeds its byte budget")
        stream.seek(0)
        return stream.read().decode("utf-8", "strict")


def component(path):
    parts = path.parts
    if parts[:2] == ("third_party", "externals"):
        notices.require(len(parts) > 3 and parts[2] in EXTERNALS,
                        "Review the newly used native component: " + path.as_posix())
        return parts[2]
    if parts[0] == "third_party":
        notices.require(len(parts) > 2 and parts[1] in WRAPPERS,
                        "Review the newly used Skia wrapper: " + path.as_posix())
    return "skia"


def fingerprint(path, relative):
    notices.require(not path.is_symlink() and path.is_file(), "Missing regular native input: " + relative)
    hasher, size = hashlib.sha256(), 0
    with path.open("rb") as stream:
        while data := stream.read(1024 * 1024):
            size += len(data)
            notices.require(size <= 512 * 1024 * 1024, "Native input exceeds its byte budget: " + relative)
            hasher.update(data)
    return {"path": relative, "sha256": hasher.hexdigest(), "bytes": size}


def build_inventory(source, build, ninja="ninja", query_fn=query):
    source, build = source.resolve(), build.resolve()
    notices.require(source.is_dir() and build.is_dir(), "Provide existing Skia source and build directories")
    graph = query_fn(build, ["inputs", *TARGETS], ninja)
    headers = query_fn(build, ["deps"], ninja)
    # Deduplicate the raw names before resolving paths: the same header may be
    # recorded for thousands of object files.
    graph_names = set(graph.splitlines()) - {""}
    header_names = {line.strip() for line in headers.splitlines() if line.startswith("    ")}
    notices.require(header_names, "No recorded headers; build Skia before collecting its inventory")
    paths, generated, excluded = {}, {}, Counter()
    for kind, names in [("graph", graph_names), ("recorded_header", header_names)]:
        for name in sorted(names):
            original = build / name
            path = original.resolve()
            lexical = Path(os.path.abspath(original))
            for root in (source, build):
                if lexical.is_relative_to(root):
                    notices.require(not original.is_symlink() and path.is_relative_to(root),
                                    "Native input link escapes or aliases its root: " + name)
            if path.is_relative_to(source):
                relative = path.relative_to(source).as_posix()
                paths.setdefault(relative, set()).add(kind)
            elif path.is_relative_to(build):
                relative = path.relative_to(build).as_posix()
                if path.is_file():
                    generated[relative] = fingerprint(path, relative)
                else:
                    # Phony graph nodes have no bytes to collect.
                    notices.require(kind == "graph" and path.suffix.lower() not in {".a", ".lib", ".o", ".obj", ".h", ".c", ".cc", ".cpp", ".s"},
                                    "Missing generated input or native library: " + name)
            else:
                notices.require(kind == "recorded_header", "Graph input escapes source/build roots: " + name)
                excluded["system_or_external_headers"] += 1
    notices.require(paths, "Ninja returned no Skia source inputs")
    notices.require(any(Path(name).suffix.lower() in {".a", ".lib"} for name in generated),
                    "No built native libraries; build Skia before collecting its inventory")
    inputs, comments, used, findings = [], [], set(), set()
    for relative, kinds in sorted(paths.items()):
        path = source / relative
        group = component(Path(relative))
        used.add(group)
        notices.require(path.is_file() and not path.is_symlink(), "Missing regular native input: " + relative)
        data = notices.read(path, MAX_SOURCE)
        record = {"path": relative, "sha256": notices.digest(data), "bytes": len(data)}
        record.update({"component": group, "observed_in": sorted(kinds)})
        inputs.append(record)
        if path.suffix.lower() not in {".c", ".cc", ".cpp", ".h", ".hh", ".hpp", ".inc", ".s"}:
            continue
        text = data[:65536].decode("utf-8", "replace")
        for match in COMMENTS.finditer(text):
            body = match.group()
            if re.search(r"copyright|SPDX-|licen[cs]e", body, re.I):
                comments.append({"path": relative, "line": text.count("\n", 0, match.start()) + 1,
                                 "text": body})
                if REFERENCES.search(body):
                    findings.add("Chromium BSD terms referenced by source comments require a separate original license; the zlib LICENSE alone is insufficient.")
    texts, files = {}, []
    candidates = {source / "LICENSE", source / "AUTHORS", source / "DEPS"}
    for name in sorted(used - {"skia"}):
        root = source / "third_party/externals" / name
        primary = []
        for path in root.rglob("*"):
            relative = path.relative_to(root)
            if ".git" in relative.parts or not (notices.is_notice(relative) or path.name in {"README.ijg", "README.chromium"}):
                continue
            if path.is_dir():
                continue
            candidates.add(path)
            if len(relative.parts) == 1 and notices.is_notice(relative):
                primary.append(path)
        notices.require(primary, "No primary original notice for native component: " + name)
    for path in sorted(candidates):
        relative = path.relative_to(source).as_posix()
        notices.require(path.resolve().is_relative_to(source), "Notice escapes the Skia source directory")
        data = notices.read(path)
        files.append({"path": relative, "sha256": notices.digest(data), "blob": notices.blob(data, texts)})
    evidence = [fingerprint(build / name, name) for name in ("args.gn", "build.ninja", "toolchain.ninja")]
    index = {"format_version": 1, "scope": "Observed Skia build inputs and original notice candidates; binary linkage and complete attribution review are not established.",
             "targets": list(TARGETS), "components": sorted(used), "inputs": inputs,
             "generated_inputs": [generated[k] for k in sorted(generated)], "build_files": evidence,
             "excluded_inputs": dict(sorted(excluded.items())), "source_comments": comments,
             "notice_files": files, "review_findings": sorted(findings), "license_references": [],
             "limits": {"source_comments": "License/copyright comment candidates in the first 65536 bytes of each observed C/C++/assembly source or header.",
                        "recorded_headers": "All recorded objects in this build directory, which can include unused or stale targets; missing files are rejected.",
                        "excluded": "System/external headers, Rust toolchain, QuickJS, fonts, icons, and optional runtimes need separate inventories."}}
    return index, texts


def load_reference(upstream):
    policy = json.loads(notices.read(upstream))
    notices.require(isinstance(policy, dict) and policy.get("format_version") == 1, "Unsupported native reference format")
    references = policy.get("references")
    notices.require(isinstance(references, list) and len(references) == 1, "Review native reference declarations")
    reference = references[0]
    notices.require(isinstance(reference, dict), "Invalid native reference declaration")
    notices.require(reference["id"] == "chromium-bsd", "Unreviewed native reference identifier")
    applies = reference["applies_to"]
    notices.require(isinstance(applies, dict) and notices.SHA.fullmatch(applies["skia_deps_sha256"]) and isinstance(applies["source_sha256"], dict)
                    and 1 <= len(applies["source_sha256"]) <= 1000, "Invalid native reference source bindings")
    for path, sha in applies["source_sha256"].items():
        notices.source_path(path)
        notices.require(path.startswith("third_party/externals/zlib/") and notices.SHA.fullmatch(sha),
                        "Invalid native reference source binding")
    repository = "https://chromium.googlesource.com/chromium/src"
    revision = reference["revision"]
    notices.require(reference["repository"] == repository and re.fullmatch(r"[0-9a-f]{40}", revision)
                    and reference["path"] == "LICENSE"
                    and reference["url"] == repository + "/+/" + revision + "/LICENSE?format=TEXT",
                    "Native reference repository/revision mismatch")
    data = notices.stored_blob(upstream.parent, reference)
    notices.require(hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest() == reference["git_blob_sha1"],
                    "Native reference Git blob mismatch")
    return reference, data


def apply_references(index, texts, upstream):
    reference, data = load_reference(upstream)
    expected = {c["path"] for c in index["source_comments"] if REFERENCES.search(c["text"])}
    if not expected:
        return
    deps = next(f for f in index["notice_files"] if f["path"] == "DEPS")
    applies = reference["applies_to"]
    notices.require(applies["skia_deps_sha256"] == deps["sha256"], "Native reference applies to another Skia DEPS revision")
    inputs = {i["path"]: i["sha256"] for i in index["inputs"]}
    notices.require(all(applies["source_sha256"].get(path) == inputs[path] for path in expected),
                    "Review the changed source referring to Chromium license terms")
    reference = {**reference, "observed_source_paths": sorted(expected), "blob": notices.blob(data, texts)}
    index["license_references"] = [reference]
    index["review_findings"] = [finding for finding in index["review_findings"]
                                if not finding.startswith("Chromium BSD terms referenced by source comments")]


def rendered(index, texts):
    lines = ["Skia build notice inventory", "", index["scope"], "",
             "Observed components: " + ", ".join(index["components"]), "",
             "Required review:", *index["review_findings"], ""]
    for record in index["notice_files"]:
        lines += ["=" * 72, record["path"], "=" * 72,
                  texts[record["sha256"]].decode("utf-8", "replace"), ""]
    for record in index["license_references"]:
        lines += ["=" * 72, "Referenced upstream terms: " + record["id"], record["url"],
                  "Source copyright notices are retained separately below.", "=" * 72,
                  texts[record["sha256"]].decode("utf-8", "replace"), ""]
    lines += ["Source comment candidates (original locations; not license classifications)", ""]
    for record in index["source_comments"]:
        lines += [record["path"] + ":" + str(record["line"]), record["text"], ""]
    return ("\n".join(lines).replace("\r\n", "\n") + "\n").encode()


def bundle_files(index, texts):
    return {"inventory.json": notices.json_bytes(index), "SKIA-NOTICES.txt": rendered(index, texts),
            **{"texts/" + key + ".txt": value for key, value in texts.items()}}


def collect(source, build, output, ninja="ninja", upstream=None):
    notices.require(not output.exists(), "Inventory output must not already exist")
    index, texts = build_inventory(source, build, ninja)
    if upstream is not None:
        apply_references(index, texts, upstream)
    files = bundle_files(index, texts)
    manifest = {"format_version": 1, "files": [
        {"path": name, "sha256": notices.digest(data), "bytes": len(data)} for name, data in sorted(files.items())]}
    output.mkdir(parents=True)
    for name, data in files.items():
        path = output / name
        path.parent.mkdir(exist_ok=True)
        with path.open("xb") as stream:
            stream.write(data)
    with (output / "manifest.json").open("xb") as stream:
        stream.write(notices.json_bytes(manifest))
    return {"components": index["components"], "source_inputs": len(index["inputs"]),
            "notice_files": len(index["notice_files"]), "review_findings": index["review_findings"]}


def verify(source, build, output, ninja="ninja", upstream=None):
    # Recollect actual source bytes and graph state, rather than trusting a
    # self-consistent manifest or a claimed list of native libraries.
    index, texts = build_inventory(source, build, ninja)
    if upstream is not None:
        apply_references(index, texts, upstream)
    expected = bundle_files(index, texts)
    expected["manifest.json"] = notices.json_bytes({"format_version": 1, "files": [
        {"path": name, "sha256": notices.digest(data), "bytes": len(data)} for name, data in sorted(expected.items())]})
    paths = {p.relative_to(output).as_posix(): p for p in output.rglob("*") if not p.is_dir()}
    notices.require(set(paths) == set(expected), "Inventory contains missing or extra files")
    for name, data in expected.items():
        notices.require(paths[name].resolve().is_relative_to(output.resolve()), "Inventory path escapes its root")
        notices.require(notices.read(paths[name], MAX_SOURCE) == data, "Stale or modified native inventory: " + name)
    return {"verified_files": len(expected), "source_inputs": len(index["inputs"]),
            "review_findings": index["review_findings"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["collect", "verify", "verify-upstream"])
    parser.add_argument("--source", type=Path)
    parser.add_argument("--build", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--ninja", default="ninja")
    parser.add_argument("--upstream", type=Path, help="Reviewed, pinned reference terms; no network access is performed")
    args = parser.parse_args()
    try:
        if args.command == "verify-upstream":
            if args.upstream is None:
                parser.error("verify-upstream requires --upstream")
            reference, data = load_reference(args.upstream)
            print(json.dumps({"verified_reference": reference["id"], "source_bindings": len(reference["applies_to"]["source_sha256"]), "bytes": len(data)}))
            return
        if args.source is None or args.build is None or args.output is None:
            parser.error("collect/verify require --source, --build, and --output")
        action = collect if args.command == "collect" else verify
        print(json.dumps(action(args.source, args.build, args.output, args.ninja, args.upstream)))
    except (notices.NoticeError, OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        parser.exit(1, "Skia inventory error: " + str(error) + "\n")


if __name__ == "__main__":
    main()

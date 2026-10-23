"""Trace copied native QuickJS inputs to the reviewed, locked registry archive.

This inventories the native inputs selected by a pinned build script. It does
not establish final binary linkage, system-header coverage, or legal approval.
"""

import re
import tomllib

import license_notices as notices
import skia_notice_inventory as native


VERSION = "0.14.0"
ARCHIVE_SHA = "cee271d0eeba64f0915b846cb7ae02e16faf3dfdffdca91731101d9d30fe3423"
BUILD_SHA = "d392d7194ae5bcaa0e0dac28a14c55656f6cf2cce3b16a8e6968c09e2ecfff85"
TARGETS = {"x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-apple-darwin",
           "aarch64-apple-darwin", "x86_64-pc-windows-msvc"}
SOURCES = ("libregexp.c", "libunicode.c", "quickjs.c", "dtoa.c")
HEADERS = ("builtin-array-fromasync.h", "builtin-iterator-zip-keyed.h", "builtin-iterator-zip.h",
           "cutils.h", "dtoa.h", "libregexp-opcode.h", "libregexp.h", "libunicode-table.h",
           "libunicode.h", "list.h", "quickjs-atom.h", "quickjs-opcode.h", "quickjs-c-atomics.h", "quickjs.h")
COPIED = [("quickjs/" + name, name) for name in (*SOURCES, *HEADERS)] + [("quickjs.bind.h", "quickjs.bind.h")]
BINDINGS = re.compile(r'\s*macro_rules!\s+bindings_env\s*\{\s*\("TARGET"\)\s*=>\s*\{\s*"([A-Za-z0-9_.-]+)"\s*\};\s*\}\s*')


def original_input(directory, name, hashes):
    path = directory / name
    notices.require(path.resolve(strict=True).is_relative_to(directory), "QuickJS input escapes its selected root")
    data = notices.read(path, native.MAX_SOURCE)
    notices.require(name in hashes and notices.digest(data) == hashes[name], "QuickJS input differs from its original archive: " + name)
    return data


def build_inventory(root, manifest, out, features, cargo, target):
    notices.require(target in TARGETS, "Review the new QuickJS native target: " + target)
    notices.require(isinstance(features, list) and all(isinstance(v, str) for v in features)
                    and not ({"bindgen", "update-bindings"} & set(features)),
                    "Review generated QuickJS bindings before collecting these features")
    source, out = manifest.parent.resolve(strict=True), out.resolve(strict=True)
    notices.require(out.is_dir() and not manifest.is_symlink(), "Expected regular selected QuickJS build inputs")
    packages = [p for p in tomllib.loads(notices.read(root / "Cargo.lock").decode())["package"]
                if p["name"] == "rquickjs-sys"]
    notices.require(len(packages) == 1 and packages[0]["version"] == VERSION
                    and packages[0].get("checksum") == ARCHIVE_SHA, "Review the new QuickJS package/build script")
    package = packages[0]
    texts, hashes = {}, {}
    record, manifest_data = notices.archive_record(package, cargo, texts, hashes)
    notices.require(hashes.get("build.rs") == BUILD_SHA, "QuickJS build script is outside the reviewed input selection")
    notices.require(notices.read(manifest) == manifest_data, "Selected QuickJS manifest differs from the original archive")
    original_input(source, "build.rs", hashes)
    binding_path = "src/bindings/" + target + ".rs"
    binding = original_input(source, binding_path, hashes)
    generated = notices.read(out / "bindings.rs")
    matched = BINDINGS.fullmatch(generated.decode("utf-8", "strict"))
    notices.require(matched is not None and matched[1] == target,
                    "Generated QuickJS target binding disagrees with the observed compiler host")
    inputs, comments = [], []
    for original, copied in COPIED:
        data = original_input(source, original, hashes)
        actual = notices.read(out / copied, native.MAX_SOURCE)
        notices.require(actual == data, "Copied QuickJS native input changed: " + copied)
        inputs.append({"source": original, "output": copied, "sha256": notices.digest(data), "bytes": len(data),
                       "role": "compiled_source" if copied in SOURCES else "copied_header"})
        # Search the complete copied input rather than only its initial bytes.
        text = data.decode("utf-8", "strict")
        for match in native.COMMENTS.finditer(text):
            if re.search(r"copyright|SPDX-|licen[cs]e", match.group(), re.I):
                comments.append({"path": original, "line": text.count("\n", 0, match.start()) + 1,
                                 "text": match.group()})
    legal = [item for item in record["files"] if item["path"] == "quickjs/LICENSE"]
    notices.require(len(legal) == 1, "Missing original QuickJS license")
    # Retain only notices applicable to this reviewed native selection. The
    # separate Rust package bundle retains the package-wide notice inventory.
    used = {item["sha256"]: texts[item["sha256"]] for item in legal}
    index = {"format_version": 1, "scope": "Copied QuickJS native inputs selected by a reviewed build script; final linkage and complete attribution review are not established.",
             "package": {k: record[k] for k in ("name", "version", "source", "archive_sha256", "manifest_sha256", "repository", "vcs")},
             "build_script_sha256": BUILD_SHA, "target": target, "features": sorted(features),
             "inputs": inputs, "source_comments": comments, "notice_files": legal,
             "rust_binding": {"path": binding_path, "sha256": notices.digest(binding), "bytes": len(binding)},
             "generated_target_binding": {"path": "bindings.rs", "sha256": notices.digest(generated), "bytes": len(generated)},
             "limits": ["Copied headers may contain unused declarations; system and compiler headers are outside this inventory.",
                        "Comment candidates cover the complete selected C/header files; they are not automatic license classifications.",
                        "Archive hashes establish content integrity; build authentication and final binary linkage require separate evidence."]}
    return index, used


def bundle_files(index, texts):
    lines = ["QuickJS native build notice inventory", "", index["scope"], "",
             "Registry archive SHA-256: " + index["package"]["archive_sha256"],
             "Observed target: " + index["target"], ""]
    for record in index["notice_files"]:
        lines += ["=" * 72, record["path"], "=" * 72, texts[record["sha256"]].decode("utf-8", "strict"), ""]
    lines += ["Source comment candidates (original locations; not license classifications)", ""]
    for record in index["source_comments"]:
        lines += [record["path"] + ":" + str(record["line"]), record["text"], ""]
    files = {"inventory.json": notices.json_bytes(index), "QUICKJS-NOTICES.txt": ("\n".join(lines) + "\n").encode(),
             **{"texts/" + sha + ".txt": data for sha, data in texts.items()}}
    files["manifest.json"] = notices.json_bytes({"format_version": 1, "files": [
        {"path": name, "sha256": notices.digest(data), "bytes": len(data)} for name, data in sorted(files.items())]})
    return files


def collect(root, manifest, out, features, cargo, target, output):
    notices.require(not output.exists() and not output.is_symlink(), "QuickJS inventory output must not exist")
    index, texts = build_inventory(root, manifest, out, features, cargo, target)
    files = bundle_files(index, texts)
    output.mkdir(parents=True)
    for name, data in files.items():
        path = output / name
        path.parent.mkdir(exist_ok=True)
        with path.open("xb") as stream:
            stream.write(data)
    return {"native_inputs": len(index["inputs"]), "compiled_sources": len(SOURCES),
            "source_comments": len(index["source_comments"]), "registry_archive_verified": True, "target": target}


def verify(root, manifest, out, features, cargo, target, output):
    index, texts = build_inventory(root, manifest, out, features, cargo, target)
    files = bundle_files(index, texts)
    actual = {p.relative_to(output).as_posix(): p for p in output.rglob("*") if not p.is_dir()}
    notices.require(set(actual) == set(files), "Missing or extra QuickJS notice inventory files")
    for name, data in files.items():
        notices.require(actual[name].resolve().is_relative_to(output.resolve())
                        and notices.read(actual[name], native.MAX_SOURCE) == data,
                        "Stale or modified QuickJS notice inventory: " + name)
    return {"verified_files": len(files), "native_inputs": len(index["inputs"])}

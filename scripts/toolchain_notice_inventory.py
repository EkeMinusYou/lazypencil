"""Collect installed Rust standard-library notice evidence, offline (Python 3.11+).

The inventory covers installed sources and target libraries, including optional
and test dependencies. It does not identify the exact code linked into an app.
"""

import argparse
import hashlib
import html
import json
from pathlib import Path
import re
import subprocess
import tarfile
import tomllib

import license_notices as notices
import skia_notice_inventory as native


MAX_FILES = 20000
MAX_BYTES = 256 * 1024 * 1024
SOURCE_CODE = {".rs", ".c", ".cc", ".cpp", ".h", ".hpp", ".s"}


def tree(root):
    notices.require(root.is_dir() and not root.is_symlink(), "Expected a regular standard-library source directory")
    result, size = {}, 0
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root)
        if ".git" in relative.parts:
            continue
        notices.require(not path.is_symlink(), "Standard-library input must not be a link: " + relative.as_posix())
        if path.is_dir():
            continue
        data = notices.read(path, native.MAX_SOURCE)
        size += len(data)
        notices.require(len(result) < MAX_FILES and size <= MAX_BYTES, "Standard-library source exceeds inspection limits")
        result[relative.as_posix()] = data
    return result


def vendor_packages(files):
    lock = tomllib.loads(files["Cargo.lock"].decode())
    packages, findings = [], []
    for package in lock["package"]:
        if "source" not in package:
            continue
        notices.require(package["source"] == "registry+https://github.com/rust-lang/crates.io-index", "Review the standard-library package source")
        key = notices.identifier(package)
        notices.require(isinstance(package.get("checksum"), str) and notices.SHA.fullmatch(package["checksum"]), "Invalid standard-library archive checksum: " + key)
        prefix = "vendor/" + package["name"] + "-" + package["version"] + "/"
        notices.require(prefix + "Cargo.toml" in files and prefix + ".cargo-checksum.json" in files,
                        "Missing standard-library vendor package: " + key)
        metadata_data = files[prefix + "Cargo.toml"]
        metadata = tomllib.loads(metadata_data.decode())["package"]
        notices.require(metadata["name"] == package["name"] and metadata["version"] == package["version"], "Vendor package identity mismatch: " + key)
        checksums = json.loads(files[prefix + ".cargo-checksum.json"])
        notices.require(isinstance(checksums, dict) and isinstance(checksums.get("files"), dict), "Invalid vendor checksum metadata: " + key)
        notices.require(checksums["package"] == package["checksum"], "Vendor package checksum differs from the standard-library lock: " + key)
        actual = {name.removeprefix(prefix): data for name, data in files.items() if name.startswith(prefix)
                  and name != prefix + ".cargo-checksum.json"}
        notices.require(set(actual) == set(checksums["files"]), "Missing or extra standard-library vendor files: " + key)
        for path, sha in checksums["files"].items():
            notices.source_path(path)
            notices.require(notices.digest(actual[path]) == sha, "Changed standard-library vendor file: " + key + "/" + path)
        legal = [name for name in actual if notices.is_notice(Path(name))]
        if not legal:
            findings.append("No standalone notice in vendored package " + key + "; retain and review its publisher declaration and source references.")
        packages.append({"name": package["name"], "version": package["version"], "archive_sha256_declared": package["checksum"],
                         "manifest_sha256": notices.digest(metadata_data), "license": metadata.get("license"),
                         "authors": metadata.get("authors", []), "repository": metadata.get("repository"),
                         "notice_paths": sorted(prefix + path for path in legal), "verified_vendor_files": len(actual)})
    actual_manifests = {p for p in files if re.fullmatch(r"vendor/[^/]+/Cargo.toml", p)}
    expected_manifests = {"vendor/" + p["name"] + "-" + p["version"] + "/Cargo.toml" for p in packages}
    notices.require(actual_manifests == expected_manifests, "Unreviewed vendored packages outside the standard-library lock")
    return sorted(packages, key=lambda p: (p["name"], p["version"])), sorted(findings)


def reference_terms(upstream):
    if upstream is None:
        return {}
    policy = json.loads(notices.read(upstream))
    notices.require(isinstance(policy, dict), "Invalid toolchain upstream policy")
    references = policy.get("license_references", [])
    notices.require(isinstance(references, list) and len(references) <= 100, "Invalid reference term list")
    result = {}
    for item in references:
        identifier, revision = item["id"], item["revision"]
        notices.require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+-]{0,127}", identifier) and identifier not in result,
                        "Invalid or duplicate reference term identifier")
        notices.require(item["kind"] == "reference_terms_only" and item["repository"] == "https://github.com/spdx/license-list-data"
                        and re.fullmatch(r"[0-9a-f]{40}", revision) and item["path"] == "text/" + identifier + ".txt"
                        and item["url"] == "https://raw.githubusercontent.com/spdx/license-list-data/" + revision + "/" + item["path"],
                        "Reference term provenance mismatch")
        data = notices.stored_blob(upstream.parent, item)
        notices.require(hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest() == item["git_blob_sha1"],
                        "Reference term Git blob mismatch")
        result[identifier] = (item, data)
    return result


def upstream_policy(upstream):
    policy = json.loads(notices.read(upstream)) if upstream is not None else {"format_version": 1, "packages": [], "license_references": []}
    notices.require(isinstance(policy, dict) and policy.get("format_version") == 1, "Unsupported toolchain upstream policy")
    reference_terms(upstream)
    notices.require(isinstance(policy.get("packages"), list) and len(policy["packages"]) <= 1000, "Invalid toolchain upstream package list")
    supplements = {notices.identifier(p): p for p in policy["packages"]}
    notices.require(len(supplements) == len(policy["packages"]), "Duplicate toolchain upstream package")
    for package in supplements.values():
        notices.require(notices.SHA.fullmatch(package["archive_sha256"]), "Invalid upstream archive checksum")
        notices.require(isinstance(package["files"], list) and 1 <= len(package["files"]) <= 100, "Missing upstream notice files")
        for item in package["files"]:
            notices.source_path(item["path"])
            revision = item["revision"]
            notices.require(re.fullmatch(r"[0-9a-f]{40}", revision) and re.fullmatch(
                r"https://raw\.githubusercontent\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/" + revision + "/" + re.escape(item["path"]), item["url"]),
                "Upstream notice URL/revision mismatch")
            data = notices.stored_blob(upstream.parent, item)
            notices.require(hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest() == item["git_blob_sha1"], "Upstream notice Git blob mismatch")
    return supplements


def registry_packages(files, cargo, upstream, texts):
    supplements = upstream_policy(upstream)
    packages, findings, sources = [], [], []
    locked = tomllib.loads(files["Cargo.lock"].decode())["package"]
    for package in sorted(locked, key=lambda p: (p["name"], p["version"])):
        if "source" not in package:
            continue
        key, hashes = notices.identifier(package), {}
        record, manifest = notices.archive_record(package, cargo, texts, hashes)
        if key in supplements:
            notices.supplement(record, manifest, supplements, upstream.parent, texts)
        record["files"].append({"path": "Cargo.toml", "kind": "publisher_declaration", "sha256": notices.digest(manifest), "blob": notices.blob(manifest, texts)})
        if not any(f["kind"] != "publisher_declaration" for f in record["files"]):
            findings.append("No standalone notice in registry package " + key + "; review its publisher declaration and source references.")
        if (record.get("vcs") or {}).get("git", {}).get("dirty"):
            findings.append("Published VCS metadata declares dirty=true for " + key + "; pinned upstream terms do not attest a pristine source checkout.")
        prefix = "vendor/" + package["name"] + "-" + package["version"] + "/"
        actual = {p.removeprefix(prefix): notices.digest(data) for p, data in files.items() if p.startswith(prefix) and p != prefix + ".cargo-checksum.json"}
        comparison = {"present": bool(actual), "matches": None}
        if actual:
            comparison.update({"missing_files": sorted(hashes.keys() - actual.keys()), "extra_files": sorted(actual.keys() - hashes.keys()),
                               "changed_files": sorted(p for p in hashes.keys() & actual.keys() if hashes[p] != actual[p])})
            comparison["matches"] = not any(comparison[name] for name in ("missing_files", "extra_files", "changed_files"))
            if not comparison["matches"]:
                findings.append("Installed vendor sources differ from the original registry archive for " + key + "; inspect vendor_registry_comparison.")
        record["vendor_registry_comparison"] = comparison
        record["original_source_hashes"] = hashes
        if "MPL-2.0" in re.findall(r"[A-Za-z0-9][A-Za-z0-9.+-]*", record.get("license") or ""):
            data = notices.read(notices.archive_path(package, cargo), 256 * 1024 * 1024)
            sha = notices.digest(data)
            texts[sha] = data
            sources.append({"package": key, "path": "sources/" + package["name"] + "-" + package["version"] + ".crate",
                            "sha256": sha, "bytes": len(data), "license": "MPL-2.0", "unmodified_registry_archive": True,
                            "url": "https://static.crates.io/crates/" + package["name"] + "/" + package["name"] + "-" + package["version"] + ".crate"})
        packages.append(record)
    return packages, sorted(findings), sources


def build_inventory(source, legal, docs, target_libraries, identity, cargo=None, upstream=None):
    notices.require(upstream is None or cargo is not None, "--upstream requires --cargo-home for original package applicability checks")
    library = source / "library"
    files = tree(library)
    notices.require("Cargo.lock" in files and "Cargo.toml" in files, "Rust sources must contain library/Cargo.toml and Cargo.lock")
    has_vendor = any(re.fullmatch(r"vendor/[^/]+/Cargo.toml", path) for path in files)
    notices.require(has_vendor or cargo is not None, "Missing standard-library vendor sources; provide --cargo-home with the locked registry archives")
    packages, findings = vendor_packages(files) if has_vendor else ([], [])
    texts, originals = {}, []
    registry, corresponding_sources = [], []
    if cargo is not None:
        registry, findings, corresponding_sources = registry_packages(files, cargo, upstream, texts)

    def retain(path, data, kind):
        originals.append({"path": path, "kind": kind, "sha256": notices.digest(data), "blob": notices.blob(data, texts)})

    for name in ("COPYRIGHT", "LICENSE-MIT", "LICENSE-APACHE"):
        retain("distribution/" + name, notices.read(legal / name), "distribution")
    copyright_data = notices.read(docs / "COPYRIGHT-library.html")
    retain("documentation/COPYRIGHT-library.html", copyright_data, "standard_library_copyright")
    expressions = re.findall(r"<b>License:</b>\s*([^<]+)", copyright_data.decode())
    notices.require(expressions, "No license declarations in the standard-library copyright document")
    terms = {name for expression in expressions for name in re.findall(r"[A-Za-z0-9][A-Za-z0-9.+-]*", html.unescape(expression))
             if name not in {"OR", "AND", "WITH"}}
    references = reference_terms(upstream)
    for name in sorted(terms):
        path = docs / "licenses" / (name + ".txt")
        if path.exists() or path.is_symlink() or name not in references:
            retain("documentation/licenses/" + name + ".txt", notices.read(path), "referenced_terms")
        else:
            reference, data = references[name]
            retain("documentation/licenses/" + name + ".txt", data, "referenced_terms_upstream")
            originals[-1]["provenance"] = reference
    comments, inputs = [], []
    for relative, data in sorted(files.items()):
        inputs.append({"path": "library/" + relative, "sha256": notices.digest(data), "bytes": len(data)})
        if notices.is_notice(Path(relative)) or relative in {"Cargo.toml", "Cargo.lock"} or re.fullmatch(r"vendor/[^/]+/Cargo.toml", relative):
            retain("library/" + relative, data, "source")
        if Path(relative).suffix.lower() in SOURCE_CODE:
            text = data[:65536].decode("utf-8", "replace")
            for match in native.COMMENTS.finditer(text):
                if re.search(r"copyright|SPDX-|licen[cs]e", match.group(), re.I):
                    comments.append({"path": "library/" + relative, "line": text.count("\n", 0, match.start()) + 1, "text": match.group()})
    for package in registry:
        prefix = "registry/" + notices.identifier(package) + "/"
        for item in package["files"]:
            retain(prefix + item["path"], texts[item["sha256"]], item["kind"])
        for item in package["attribution_comments"]:
            for line in item["lines"]:
                comments.append({"path": prefix + item["path"], "line": line["line"], "text": line["text"]})
    unwind = source / "src/llvm-project/libunwind/LICENSE.TXT"
    if unwind.exists():
        retain("src/llvm-project/libunwind/LICENSE.TXT", notices.read(unwind), "native_unwind_reference")
    binaries = [native.fingerprint(path, path.name) for path in sorted(target_libraries.iterdir())
                if path.is_file() and path.suffix.lower() in {".rlib", ".rmeta", ".so", ".dylib", ".dll", ".lib"}]
    notices.require(any(p["path"].startswith("libstd-") and p["path"].endswith(".rlib") for p in binaries),
                    "No installed target standard library")
    return {"format_version": 1, "scope": "Installed Rust standard-library source/notice and target-library inventory; not exact application linkage.",
            "observed_toolchain": identity, "source_inputs": inputs, "vendor_packages": packages, "registry_packages": registry,
            "corresponding_sources": corresponding_sources,
            "notice_files": originals, "source_comments": comments, "target_libraries": binaries,
            "review_findings": findings, "vendor_file_checksums_verified": has_vendor, "registry_archives_verified": cargo is not None,
            "limits": {"source_provenance": "Local source/target hashes and observed compiler identity; upstream authenticity and source-to-binary provenance require separate verification.",
                       "vendor_checksums": "Present vendor files are checked against vendor metadata and the library lock. With --cargo-home, original archive checksums and all source hashes are also verified; vendor/archive differences remain explicit review findings. Collection never fetches inputs.",
                       "coverage": "All installed library sources and library lock registry packages, including unused, optional and test code; compiler executables, system libraries and exact app linkage are excluded.",
                       "comments": "Comment candidates in the first 65536 bytes of each observed Rust/C/C++/assembly source, not automatic license classifications."}}, texts


def bundle_files(index, texts):
    lines = ["Rust standard-library notice inventory", "", index["scope"], "",
             "Compiler: " + index["observed_toolchain"]["release"], "Target: " + index["observed_toolchain"]["target"], "",
             "Review findings:", *index["review_findings"], ""]
    for item in index["notice_files"]:
        lines += ["=" * 72, item["path"] + " (" + item["kind"] + ")", "=" * 72]
        if item["kind"].startswith("referenced_terms"):
            lines.append("Reference terms only; original package/source copyright notices are retained separately.")
        lines += [texts[item["sha256"]].decode("utf-8", "replace"), ""]
    lines += ["Source comment candidates; source copyright notices are retained verbatim", ""]
    for item in index["source_comments"]:
        lines += [item["path"] + ":" + str(item["line"]), item["text"], ""]
    for item in index["corresponding_sources"]:
        lines += ["Corresponding source for " + item["package"] + " (" + item["license"] + "):",
                  item["path"], "Complete, unmodified registry archive. Original notices are retained above.", "SHA-256: " + item["sha256"], ""]
    source_paths = {item["sha256"]: item["path"] for item in index["corresponding_sources"]}
    files = {"inventory.json": notices.json_bytes(index), "TOOLCHAIN-NOTICES.txt": ("\n".join(lines).replace("\r\n", "\n") + "\n").encode(),
             **{source_paths.get(sha, "texts/" + sha + ".txt"): data for sha, data in texts.items()}}
    files["manifest.json"] = notices.json_bytes({"format_version": 1, "files": [
        {"path": name, "sha256": notices.digest(data), "bytes": len(data)} for name, data in sorted(files.items())]})
    return files


def publish(files, output):
    notices.require(not output.exists(), "Toolchain inventory output must not already exist")
    notices.require(sum(map(len, files.values())) <= notices.MAX_BUNDLE, "Toolchain notice bundle exceeds its byte budget")
    output.mkdir(parents=True)
    for name, data in files.items():
        path = output / name
        path.parent.mkdir(exist_ok=True)
        with path.open("xb") as stream:
            stream.write(data)


def verify(files, output):
    actual = {p.relative_to(output).as_posix(): p for p in output.rglob("*") if not p.is_dir()}
    notices.require(set(actual) == set(files), "Missing or extra toolchain inventory files")
    for name, data in files.items():
        notices.require(actual[name].resolve().is_relative_to(output.resolve()), "Toolchain inventory link escapes its root")
        notices.require(notices.read(actual[name], native.MAX_SOURCE) == data, "Changed or stale toolchain inventory: " + name)


def observe(rustc, target):
    def run(arguments):
        result = subprocess.run([rustc, *arguments], check=True, capture_output=True, text=True, encoding="utf-8", timeout=20)
        notices.require(len(result.stdout) < 65536, "Unexpected rustc metadata response")
        return result.stdout.strip()
    verbose = run(["--version", "--verbose"])
    fields = dict(line.split(": ", 1) for line in verbose.splitlines() if ": " in line)
    target = target or fields["host"]
    notices.require(re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", target), "Invalid target triple")
    return {"verbose": verbose, "release": fields["release"], "commit_hash": fields["commit-hash"], "target": target}, Path(run(["--print", "sysroot"])), Path(run(["--print", "target-libdir", "--target", target]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["collect", "verify", "verify-upstream"])
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument("--target")
    parser.add_argument("--source-root", type=Path)
    parser.add_argument("--legal-root", type=Path)
    parser.add_argument("--doc-root", type=Path)
    parser.add_argument("--cargo-home", type=Path, help="Offline cache of the standard-library lock's original registry archives")
    parser.add_argument("--upstream", type=Path, help="Pinned workspace-root package notices omitted from registry archives")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "verify-upstream":
            if args.upstream is None:
                parser.error("verify-upstream requires --upstream")
            policy = upstream_policy(args.upstream)
            print(json.dumps({"verified_upstream_packages": len(policy), "verified_original_files": sum(len(p["files"]) for p in policy.values()),
                              "canonical_reference_terms": len(reference_terms(args.upstream)), "registry_applicability_verified": False}))
            return
        if args.output is None:
            parser.error("collect/verify require --output")
        if args.command == "collect":
            notices.require(not args.output.exists(), "Toolchain inventory output must not already exist")
        identity, sysroot, target = observe(args.rustc, args.target)
        docs = args.doc_root or sysroot / "share/doc/rustc"
        if args.doc_root is None and not (docs / "COPYRIGHT-library.html").is_file():
            docs = sysroot / "share/doc/rust"
        index, texts = build_inventory(args.source_root or sysroot / "lib/rustlib/src/rust", args.legal_root or sysroot,
                                       docs, target, identity, args.cargo_home, args.upstream)
        files = bundle_files(index, texts)
        if args.command == "collect":
            publish(files, args.output)
        else:
            verify(files, args.output)
        print(json.dumps({"source_inputs": len(index["source_inputs"]), "vendor_packages": len(index["vendor_packages"]),
                          "registry_packages": len(index["registry_packages"]), "registry_archives_verified": index["registry_archives_verified"],
                          "target_libraries": len(index["target_libraries"]), "bundle_files": len(files), "review_findings": index["review_findings"]}))
    except (notices.NoticeError, OSError, ValueError, KeyError, TypeError, tarfile.TarError, EOFError, subprocess.SubprocessError) as error:
        parser.exit(1, "Toolchain inventory error: " + str(error) + "\n")


if __name__ == "__main__":
    main()

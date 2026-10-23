"""Collect and verify original Rust package notices (Python 3.11+).

This is notice provenance, not an SPDX policy solver or a native/toolchain audit.
Collection is offline: exact crate archives and curated upstream inputs must
already be present. Nothing is extracted from a third-party archive.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import tarfile
import tempfile
import tomllib

MAX_FILE = 4 * 1024 * 1024
MAX_BUNDLE = 64 * 1024 * 1024
LEGAL = re.compile(r"^(?:licen[cs]e|copying|copyright|notice|authors|unlicense)(?:[._-].*)?$", re.I)
CODE = {".rs", ".py", ".c", ".h", ".hpp", ".cpp", ".s", ".asm", ".toml", ".json", ".lock"}
ATTRIBUTION = re.compile(r"copyright|©|SPDX-(?:FileCopyrightText|License-Identifier)", re.I)
SHA = re.compile(r"[0-9a-f]{64}")


class NoticeError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise NoticeError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()


def read(path, maximum=MAX_FILE):
    require(not path.is_symlink() and path.is_file(), f"Expected a regular notice input: {path}")
    with path.open("rb") as stream:
        data = stream.read(maximum + 1)
    require(len(data) <= maximum, f"Notice input exceeds its byte limit: {path}")
    return data


def identifier(package):
    name, version = package["name"], package["version"]
    require(isinstance(name, str) and re.fullmatch(r"[A-Za-z0-9_-]{1,128}", name), "Invalid package name")
    require(isinstance(version, str) and re.fullmatch(r"[A-Za-z0-9.+_-]{1,128}", version), "Invalid package version")
    return name + "@" + version


def source_path(value):
    require(isinstance(value, str) and value and "\\" not in value and "\0" not in value, "Invalid archive path")
    path = PurePosixPath(value)
    require(not path.is_absolute() and all(part not in ("", ".", "..") for part in value.split("/")), "Escaping archive path")
    return path


def is_notice(path):
    return ((LEGAL.fullmatch(path.name) is not None and path.suffix.lower() not in CODE)
            or any(part.upper() == "LICENSES" for part in path.parts[:-1]))


def blob(data, texts):
    value = digest(data)
    texts[value] = data
    return "texts/" + value + ".txt"


def stored_blob(directory, item):
    value = item["sha256"]
    require(isinstance(value, str) and SHA.fullmatch(value), "Invalid notice checksum")
    name = "texts/" + value + ".txt"
    require(item["blob"] == name, "Unexpected notice blob path")
    data = read(directory / name)
    require(digest(data) == value, f"Notice checksum mismatch: {name}")
    return data


def archive_path(package, cargo):
    key = identifier(package)
    require(package["source"] == "registry+https://github.com/rust-lang/crates.io-index", f"Unreviewed package source: {key}")
    expected = package.get("checksum")
    require(isinstance(expected, str) and SHA.fullmatch(expected), f"Missing archive checksum: {key}")
    candidates = sorted((cargo / "registry/cache").glob("*/" + package["name"] + "-" + package["version"] + ".crate"))
    archive = next((path for path in candidates if digest(read(path, 256 * 1024 * 1024)) == expected), None)
    require(archive is not None, f"Fetch the locked crate archive before collecting notices: {key}")
    return archive


def archive_record(package, cargo, texts, source_hashes=None):
    key = identifier(package)
    archive = archive_path(package, cargo)
    expected = package["checksum"]
    prefix = package["name"] + "-" + package["version"]
    files, attributions, seen = [], [], set()
    manifest, manifest_data, vcs = None, None, None
    total = 0
    with tarfile.open(archive, "r:gz") as stream:
        for entry in stream:
            path = source_path(entry.name)
            require(path.parts[0] == prefix, f"Wrong archive root: {key}")
            require(entry.name not in seen, f"Duplicate archive member: {key}")
            seen.add(entry.name)
            total += entry.size
            require(len(seen) <= 100000 and total <= 512 * 1024 * 1024, f"Archive exceeds inspection limits: {key}")
            relative = PurePosixPath(*path.parts[1:])
            if not entry.isfile():
                require(source_hashes is None or entry.isdir(), f"Linked source member is not supported: {key}/{relative}")
                require(not is_notice(relative) or entry.isdir(), f"Linked notice is not supported: {key}")
                continue
            if source_hashes is not None:
                require(entry.size <= 128 * 1024 * 1024, f"Oversized source member: {key}/{relative}")
                source_hashes[str(relative)] = digest(stream.extractfile(entry).read())
            if str(relative) in ("Cargo.toml", ".cargo_vcs_info.json"):
                require(entry.size <= MAX_FILE, f"Oversized package metadata: {key}")
                data = stream.extractfile(entry).read()
                if str(relative) == "Cargo.toml":
                    manifest_data, manifest = data, tomllib.loads(data.decode())
                else:
                    vcs = json.loads(data)
            if is_notice(relative):
                require(entry.size <= MAX_FILE, f"Oversized notice: {key}/{relative}")
                data = stream.extractfile(entry).read()
                files.append({"path": str(relative), "kind": "archive", "sha256": digest(data), "blob": blob(data, texts)})
            elif relative.suffix.lower() in CODE | {".md", ".txt"}:
                data = stream.extractfile(entry).read(min(entry.size, 32768))
                lines = []
                for number, line in enumerate(data.decode("utf-8", errors="replace").splitlines(), 1):
                    if ATTRIBUTION.search(line) and (relative.suffix.lower() in {".md", ".txt"} or line.lstrip().startswith(("//", "/*", "*", "#", ";", "!"))):
                        lines.append({"line": number, "text": line})
                if lines:
                    attributions.append({"path": str(relative), "lines": lines})
    require(manifest is not None, f"Missing package manifest: {key}")
    metadata = manifest["package"]
    require(metadata["name"] == package["name"] and metadata["version"] == package["version"], f"Package identity mismatch: {key}")
    if metadata.get("license-file") and not any(item["path"] == metadata["license-file"] for item in files):
        relative = source_path(metadata["license-file"])
        with tarfile.open(archive, "r:gz") as stream:
            member = stream.getmember(prefix + "/" + str(relative))
            require(member.isfile() and member.size <= MAX_FILE, f"Invalid declared license file: {key}")
            data = stream.extractfile(member).read()
            files.append({"path": str(relative), "kind": "archive", "sha256": digest(data), "blob": blob(data, texts)})
    return {"name": package["name"], "version": package["version"], "source": package["source"],
            "archive_sha256": expected, "manifest_sha256": digest(manifest_data),
            "license": metadata.get("license"), "license_file": metadata.get("license-file"),
            "authors": metadata.get("authors", []), "repository": metadata.get("repository"),
            "vcs": vcs, "files": sorted(files, key=lambda f: f["path"]),
            "attribution_comments": attributions}, manifest_data


def local_record(package, root, location, texts):
    path = (root / location).resolve()
    require(path.is_relative_to(root.resolve()), "Local package must remain within the workspace")
    metadata_data = read(path / "Cargo.toml")
    metadata = tomllib.loads(metadata_data.decode())["package"]
    require(metadata["name"] == package["name"] and metadata["version"] == package["version"], "Local package identity mismatch")
    files = []
    for member in sorted(path.iterdir()):
        if member.is_file() and is_notice(PurePosixPath(member.name)):
            data = read(member)
            files.append({"path": member.name, "kind": "workspace", "sha256": digest(data), "blob": blob(data, texts)})
    require(files, "Local package has no notice")
    return {"name": package["name"], "version": package["version"], "source": "workspace", "workspace_path": location,
            "license": metadata.get("license"), "manifest_sha256": digest(metadata_data), "files": files,
            "authors": metadata.get("authors", []), "attribution_comments": []}


def supplement(record, manifest_data, inputs, directory, texts):
    key = identifier(record)
    primary = any((len(PurePosixPath(f["path"]).parts) == 1 and re.match(r"^(?:licen[cs]e|copying|unlicense)", f["path"], re.I)) or f["path"] == record.get("license_file") for f in record["files"])
    extra = inputs.get(key)
    if extra is None:
        require(primary, f"Review omitted upstream notices before collecting: {key}")
        return
    require(extra["archive_sha256"] == record["archive_sha256"], f"Upstream inputs refer to another archive: {key}")
    record["supplement_reason"] = extra.get("reason", "Original workspace-root notices omitted by the published archive.")
    for item in extra["files"]:
        revision = (record.get("vcs") or {}).get("git", {}).get("sha1")
        require(revision and item["revision"] == revision, f"Upstream revision does not match the published package: {key}")
        source_path(item["path"])
        repository = record["repository"].rstrip("/").removesuffix(".git")
        require(repository.startswith("https://github.com/") and item["url"].startswith(repository.replace("https://github.com/", "https://raw.githubusercontent.com/") + "/" + revision + "/"), f"Upstream repository mismatch: {key}")
        data = stored_blob(directory, item)
        require(hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest() == item["git_blob_sha1"], f"Upstream Git blob mismatch: {key}")
        entry = {**item, "kind": "upstream"}
        entry["blob"] = blob(data, texts)
        record["files"].append(entry)
    if extra.get("license_reference"):
        require(extra["license_reference"] == "MIT" and "MIT" in (record.get("license") or ""), f"Unreviewed license reference: {key}")
        record["license_reference"] = extra["license_reference"]
        # Keep the publisher's actual statement and authors; a reference text
        # must never be presented as a populated package copyright notice.
        record["files"].append({"path": "Cargo.toml", "kind": "publisher_declaration", "sha256": digest(manifest_data), "blob": blob(manifest_data, texts)})
    require(record["files"], f"No original notice or publisher declaration: {key}")


def render(index, texts):
    lines = ["Rust package notices", "====================", "", index["scope"], "",
             "Package authors below are copied from published metadata, not inferred copyright holders.",
             "Shared license reference texts are labeled separately from original package notices.",
             "Original notice bytes and detailed provenance are retained in index.json and texts/.", ""]
    by_blob = {}
    for package in index["packages"]:
        key = identifier(package)
        lines += [key, "Declared license: " + str(package.get("license"))]
        if package.get("authors"):
            lines.append("Published package authors: " + "; ".join(package["authors"]))
        if package.get("supplement_reason"):
            lines.append(package["supplement_reason"])
        if package.get("license_reference"):
            lines.append("Reference terms: " + package["license_reference"] + " (not a package copyright notice)")
        for item in package["files"]:
            by_blob.setdefault(item["blob"], []).append(key + " :: " + item["kind"] + " :: " + item["path"])
        comments = {}
        for item in package["attribution_comments"]:
            for line in item["lines"]:
                comments.setdefault(line["text"], []).append(item["path"] + ":" + str(line["line"]))
        if comments:
            lines.append("Verbatim source attribution comments:")
            for value, paths in sorted(comments.items()):
                lines += [value, "  Source: " + ", ".join(paths)]
        lines.append("")
    for item in index["license_references"]:
        by_blob.setdefault(item["blob"], []).append("Shared reference terms: " + item["id"] + "; " + item["role"])
    for name, uses in sorted(by_blob.items()):
        data = texts[PurePosixPath(name).stem]
        text = data.decode("utf-8") if valid_utf8(data) else data.decode("latin-1")
        # Normalize display whitespace only. The content-addressed source
        # blobs remain byte-identical to the original notices.
        text = "\n".join(line.rstrip(" \t\r") for line in text.splitlines())
        lines += ["=" * 72, "\n".join(uses), "SHA-256: " + PurePosixPath(name).stem, "", text, ""]
    return ("\n".join(lines) + "\n").encode()


def valid_utf8(data):
    try:
        data.decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


def collect(root, cargo, upstream, output):
    require(not output.exists() and not output.is_symlink(), "Output must not already exist")
    lock_data = read(root / "Cargo.lock")
    locked = tomllib.loads(lock_data.decode())["package"]
    require(len(locked) <= 10000, "Too many locked packages")
    policy = json.loads(read(upstream))
    require(policy["format_version"] == 1, "Unknown upstream input format")
    inputs = {identifier(p): p for p in policy["packages"]}
    require(len(inputs) == len(policy["packages"]), "Duplicate upstream package")
    texts = {}
    references = []
    for item in policy["license_references"]:
        data = stored_blob(upstream.parent, item)
        references.append({**item, "blob": blob(data, texts)})
    cargo_manifest = tomllib.loads(read(root / "Cargo.toml").decode())
    locations = {cargo_manifest["package"]["name"]: "."}
    locations.update({name: item["path"] for name, item in cargo_manifest.get("patch", {}).get("crates-io", {}).items() if "path" in item})
    records = []
    seen = set()
    for package in sorted(locked, key=lambda p: (p["name"], p["version"])):
        key = identifier(package)
        require(key not in seen, "Duplicate package identity")
        seen.add(key)
        if package.get("source"):
            record, data = archive_record(package, cargo, texts)
            supplement(record, data, inputs, upstream.parent, texts)
        else:
            require(package["name"] in locations, "Unmapped local package")
            record = local_record(package, root, locations[package["name"]], texts)
        records.append(record)
    require(set(inputs) <= seen, "Stale upstream package inputs")
    index = {"format_version": 1, "cargo_lock_sha256": digest(lock_data),
             "scope": "All Cargo.lock packages, including optional, build, and platform dependencies. This is not a linked-binary inventory. Native engines, the Rust toolchain, fonts, and icons require their separate notices.",
             "packages": records, "license_references": references}
    files = {"index.json": json_bytes(index), "upstream.json": json_bytes(policy),
             "RUST-NOTICES.txt": render(index, texts)}
    files.update({"texts/" + key + ".txt": data for key, data in texts.items()})
    # Preserve every curated source input, even when its text deduplicates.
    require(sum(map(len, files.values())) <= MAX_BUNDLE, "Notice bundle exceeds 64 MiB")
    require(read(root / "Cargo.lock") == lock_data, "Cargo.lock changed during collection")
    manifest = {"format_version": 1, "files": [{"path": key, "bytes": len(value), "sha256": digest(value)} for key, value in sorted(files.items())]}
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="lazypencil-notices-", dir=output.parent) as temporary:
        stage = Path(temporary) / "bundle"
        stage.mkdir()
        for name, data in files.items():
            destination = stage / name
            destination.parent.mkdir(exist_ok=True)
            destination.write_bytes(data)
        (stage / "manifest.json").write_bytes(json_bytes(manifest))
        output.mkdir()
        for member in stage.iterdir():
            member.rename(output / member.name)
    return verify(root, output)


def verify(root, bundle, cargo=None):
    require(not (bundle / "texts").is_symlink(), "Notice text directory must not be a symlink")
    manifest = json.loads(read(bundle / "manifest.json"))
    require(manifest["format_version"] == 1 and len(manifest["files"]) <= 10000, "Unsupported notice manifest")
    files, total = {}, 0
    for item in manifest["files"]:
        require(set(item) == {"path", "bytes", "sha256"} and type(item["bytes"]) is int and item["bytes"] >= 0, "Invalid notice member metadata")
        name = str(source_path(item["path"]))
        require(name in {"index.json", "upstream.json", "RUST-NOTICES.txt"} or re.fullmatch(r"texts/[0-9a-f]{64}\.txt", name), "Unexpected notice member")
        require(name not in files, "Duplicate notice member")
        data = read(bundle / name, MAX_BUNDLE)
        total += len(data)
        require(total <= MAX_BUNDLE and len(data) == item["bytes"] and digest(data) == item["sha256"], f"Notice integrity mismatch: {name}")
        if name.startswith("texts/"):
            require(digest(data) == PurePosixPath(name).stem, "Notice blob name mismatch")
        files[name] = data
    actual = {str(p.relative_to(bundle).as_posix()) for p in bundle.rglob("*") if p.is_file() or p.is_symlink()}
    require(actual == set(files) | {"manifest.json"}, "Unlisted or missing notice members")
    index = json.loads(files["index.json"])
    lock_data = read(root / "Cargo.lock")
    require(index["format_version"] == 1 and index["cargo_lock_sha256"] == digest(lock_data), "Notice inventory is stale; regenerate for Cargo.lock")
    locked = {identifier(p): p for p in tomllib.loads(lock_data.decode())["package"]}
    packages = {identifier(p): p for p in index["packages"]}
    require(len(packages) == len(index["packages"]) and set(packages) == set(locked), "Notice inventory does not cover the locked packages")
    for key, package in packages.items():
        require(package["files"], f"No original notice/declaration: {key}")
        if locked[key].get("source"):
            require(package["source"] == locked[key]["source"] and package["archive_sha256"] == locked[key]["checksum"], f"Registry package mismatch: {key}")
        else:
            require(package["source"] == "workspace", f"Local source mismatch: {key}")
            location = source_path(package["workspace_path"]) if package["workspace_path"] != "." else PurePosixPath(".")
            require(digest(read(root / location / "Cargo.toml")) == package["manifest_sha256"], f"Local package metadata changed: {key}")
        for item in package["files"]:
            allowed = {"archive", "upstream", "publisher_declaration"} if locked[key].get("source") else {"workspace"}
            require(item["kind"] in allowed, f"Notice origin does not match the package source: {key}")
            require(item["blob"] in files and digest(files[item["blob"]]) == item["sha256"], f"Missing original notice: {key}")
            if item["kind"] == "workspace":
                relative = source_path(item["path"])
                require(digest(read(root / location / relative)) == item["sha256"], f"Workspace notice changed: {key}")
    for item in index["license_references"]:
        require(item["blob"] in files and digest(files[item["blob"]]) == item["sha256"], "Missing license reference")
    texts = {PurePosixPath(name).stem: data for name, data in files.items() if name.startswith("texts/")}
    require(render(index, texts) == files["RUST-NOTICES.txt"], "Readable notices disagree with the inventory")
    if cargo is not None:
        policy = json.loads(files["upstream.json"])
        inputs = {identifier(p): p for p in policy["packages"]}
        for key, package in packages.items():
            if locked[key].get("source"):
                captured, data = archive_record(locked[key], cargo, {})
                supplement(captured, data, inputs, bundle, {})
            else:
                captured = local_record(locked[key], root, package["workspace_path"], {})
            require(captured == package, f"Package provenance differs from the locked source: {key}")
    return {"rust_packages": len(packages), "verified_files": len(files) + 1, "bytes": total, "cargo_lock_sha256": digest(lock_data), "source_archives_verified": cargo is not None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    make = commands.add_parser("collect")
    make.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    make.add_argument("--cargo-home", type=Path, default=Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")))
    make.add_argument("--upstream", type=Path, required=True)
    make.add_argument("--output", type=Path, required=True)
    check = commands.add_parser("verify")
    check.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    check.add_argument("--bundle", type=Path, default=Path(__file__).resolve().parent.parent / "resources/licenses/rust")
    check.add_argument("--cargo-home", type=Path, help="Also compare notices and metadata with the exact cached crate archives")
    args = parser.parse_args()
    try:
        result = collect(args.root, args.cargo_home, args.upstream, args.output) if args.command == "collect" else verify(args.root, args.bundle, args.cargo_home)
        print(json.dumps(result))
    except (NoticeError, OSError, ValueError, KeyError, TypeError, tarfile.TarError, EOFError, RecursionError) as error:
        parser.exit(1, f"Notice error: {error}\n")


if __name__ == "__main__":
    main()

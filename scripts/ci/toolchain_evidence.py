"""Prepare pinned standard-library audit inputs on a hosted runner, without compiling."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import license_notices as notices
import toolchain_notice_inventory as toolchain


RELEASE = "1.94.0"
COMMIT = "4a4ef493e3a1488c6e321570238084b38948f6db"
SOURCE_SHA = "0b53ae34f5c0c3612cfe1de139f9167a018cd5737bc2205664fd69ba9b25f600"
SOURCE_URL = "https://static.rust-lang.org/dist/rustc-1.94.0-src.tar.xz"
PREFIX = "rustc-1.94.0-src"
LEGAL = {"COPYRIGHT", "LICENSE-MIT", "LICENSE-APACHE", "src/llvm-project/libunwind/LICENSE.TXT"}


def download(url, sha, path, maximum):
    notices.require(not path.exists() and not path.is_symlink(), "Download destination already exists")
    notices.require(isinstance(sha, str) and notices.SHA.fullmatch(sha), "Invalid upstream checksum")
    hasher, size, started = hashlib.sha256(), 0, time.monotonic()
    with urllib.request.urlopen(url, timeout=30) as response, path.open("xb") as stream:
        while data := response.read(1024 * 1024):
            size += len(data)
            notices.require(size <= maximum and time.monotonic() - started <= 300, "Upstream download exceeds its budget")
            hasher.update(data)
            stream.write(data)
    notices.require(hasher.hexdigest() == sha, "Upstream checksum mismatch: " + url)
    return size


def prepare_source(archive, destination):
    files, size = 0, 0
    with tarfile.open(archive, "r|xz") as stream:
        for member in stream:
            path = PurePosixPath(member.name)
            notices.require(path.parts and path.parts[0] == PREFIX, "Unexpected Rust source archive root")
            relative = PurePosixPath(*path.parts[1:]).as_posix()
            if relative not in LEGAL and relative != "library" and not relative.startswith("library/"):
                continue
            notices.source_path(member.name)
            notices.require(member.isfile() or member.isdir(), "Linked standard-library source member")
            if member.isfile():
                files += 1
                size += member.size
                notices.require(files <= 20000 and 0 <= member.size <= 32 * 1024 * 1024
                                and size <= 256 * 1024 * 1024, "Selected Rust sources exceed their budget")
            # The archive is checksum-pinned; the standard data filter also
            # prevents archive paths, links and permissions escaping staging.
            stream.extract(member, destination, filter="data")
    source = destination / PREFIX
    for name in ("COPYRIGHT", "LICENSE-MIT", "LICENSE-APACHE", "library/Cargo.toml", "library/Cargo.lock"):
        notices.require((source / name).is_file() and not (source / name).is_symlink(), "Missing Rust source input: " + name)
    return source, {"selected_files": files, "selected_bytes": size}


def collect(root, output, target=None):
    notices.require(os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_TEMP"),
                    "Run this large-input preparation on GitHub Actions")
    notices.require(not output.exists() and not output.is_symlink(), "Toolchain evidence output must not exist")
    identity, sysroot, libraries = toolchain.observe("rustc", target)
    notices.require(identity["release"] == RELEASE and identity["commit_hash"] == COMMIT,
                    "Review source provenance for the selected compiler")
    work = Path(os.environ["RUNNER_TEMP"]) / "stdlib-notice-inputs"
    work.mkdir(mode=0o700)
    archive = work / "source.tar.xz"
    download(SOURCE_URL, SOURCE_SHA, archive, 512 * 1024 * 1024)
    source, selected = prepare_source(archive, work)
    archive.unlink()
    cargo = work / "cargo"
    cache = cargo / "registry/cache/stdlib"
    cache.mkdir(parents=True)
    packages = [p for p in tomllib.loads(notices.read(source / "library/Cargo.lock").decode())["package"] if "source" in p]
    notices.require(len(packages) <= 1000, "Standard-library registry inventory exceeds its budget")
    total = 0
    for package in packages:
        notices.identifier(package)
        notices.require(package["source"] == "registry+https://github.com/rust-lang/crates.io-index", "Review standard-library source registry")
        filename = package["name"] + "-" + package["version"] + ".crate"
        total += download("https://static.crates.io/crates/" + package["name"] + "/" + filename,
                          package["checksum"], cache / filename, 64 * 1024 * 1024)
        notices.require(total <= 512 * 1024 * 1024, "Standard-library archives exceed their total byte budget")
    candidates = [sysroot / "share/doc" / directory for directory in ("rust", "rust/html", "rustc", "rustc/html")]
    docs = next((path for path in candidates if (path / "COPYRIGHT-library.html").is_file()), None)
    notices.require(docs is not None, "Install matching rust-docs; missing COPYRIGHT-library.html in " + str(candidates))
    upstream = root / "resources/licenses/toolchain/upstream.json"
    index, texts = toolchain.build_inventory(source, source, docs, libraries, identity, cargo, upstream)
    output.mkdir(mode=0o700)
    inventory = output / "inventory"
    toolchain.publish(toolchain.bundle_files(index, texts), inventory)
    # Reconstruct from installed libraries and original source/archive bytes.
    again, again_texts = toolchain.build_inventory(source, source, docs, libraries, identity, cargo, upstream)
    toolchain.verify(toolchain.bundle_files(again, again_texts), inventory)
    distribution = {"format_version": 1, "release_ready": False, "observed_toolchain": identity,
                    "source_url": SOURCE_URL, "source_sha256": SOURCE_SHA, "source_selection": selected,
                    "registry_packages": len(packages), "registry_download_bytes": total,
                    "scope": "This runner's installed standard-library input evidence; no application binary or final linkage is established.",
                    "collector_revision": os.environ.get("GITHUB_SHA"), "run_id": os.environ.get("GITHUB_RUN_ID")}
    (output / "source-distribution.json").write_bytes(notices.json_bytes(distribution))
    shutil.rmtree(work)
    return {"source_inputs": len(index["source_inputs"]), "registry_packages": len(index["registry_packages"]),
            "target_libraries": len(index["target_libraries"]), "review_findings": index["review_findings"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target", help="Installed target whose standard-library inputs are inventoried")
    args = parser.parse_args()
    try:
        print(json.dumps(collect(Path(__file__).resolve().parents[2], args.output, args.target)))
    except (notices.NoticeError, OSError, ValueError, KeyError, TypeError, tarfile.TarError,
            EOFError, subprocess.SubprocessError) as error:
        parser.exit(1, "Toolchain evidence error: " + str(error) + "\n")


if __name__ == "__main__":
    main()

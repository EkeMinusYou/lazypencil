"""Bind original standard-library evidence to the compiler/libraries used by a build."""

import json
import os
from pathlib import Path
import re
import shutil

from ci.toolchain_evidence import COMMIT, RELEASE, SOURCE_SHA, SOURCE_URL
import license_notices as notices
import toolchain_notice_inventory as toolchain


def validate(root, compiler, rustc="rustc"):
    notices.require(root.is_dir() and not root.is_symlink(), "Expected a regular toolchain evidence directory")
    root = root.resolve(strict=True)
    notices.require({p.name for p in root.iterdir()} == {"inventory", "source-distribution.json"},
                    "Unexpected toolchain evidence members")
    paths = {}
    for path in root.rglob("*"):
        notices.require(not path.is_symlink() and path.resolve(strict=True) == Path(os.path.abspath(path)),
                        "Linked toolchain evidence member")
        if not path.is_dir():
            paths[path.relative_to(root).as_posix()] = path
            notices.require(len(paths) <= 10000, "Too many toolchain evidence members")
    distribution_bytes = notices.read(root / "source-distribution.json")
    distribution = json.loads(distribution_bytes)
    notices.require(distribution.get("format_version") == 1 and distribution.get("release_ready") is False
                    and distribution.get("source_url") == SOURCE_URL and distribution.get("source_sha256") == SOURCE_SHA,
                    "Unreviewed standard-library source distribution")
    manifest_bytes = notices.read(root / "inventory/manifest.json")
    manifest = json.loads(manifest_bytes)
    notices.require(manifest.get("format_version") == 1 and isinstance(manifest.get("files"), list)
                    and len(manifest["files"]) <= 10000, "Invalid toolchain notice manifest")
    expected, total = {"source-distribution.json", "inventory/manifest.json"}, 0
    for item in manifest["files"]:
        notices.require(isinstance(item, dict) and set(item) == {"path", "sha256", "bytes"}
                        and type(item["bytes"]) is int and item["bytes"] >= 0, "Invalid toolchain notice member")
        name = str(notices.source_path(item["path"]))
        notices.require(name in {"inventory.json", "TOOLCHAIN-NOTICES.txt"}
                        or re.fullmatch(r"texts/[0-9a-f]{64}\.txt", name)
                        or re.fullmatch(r"sources/[A-Za-z0-9_.+-]+\.crate", name), "Unreviewed toolchain notice member")
        key = "inventory/" + name
        notices.require(key not in expected, "Duplicate toolchain notice member")
        expected.add(key)
        data = notices.read(root / key, notices.MAX_BUNDLE)
        total += len(data)
        notices.require(total <= notices.MAX_BUNDLE and len(data) == item["bytes"]
                        and notices.digest(data) == item["sha256"], "Changed toolchain notice member: " + name)
    notices.require(set(paths) == expected, "Missing or extra toolchain notice members")
    notices.require({"inventory/inventory.json", "inventory/TOOLCHAIN-NOTICES.txt"} <= expected,
                    "Missing required standard-library notice documents")
    index_bytes = notices.read(root / "inventory/inventory.json", notices.MAX_BUNDLE)
    index = json.loads(index_bytes)
    identity, sysroot, libraries = toolchain.observe(rustc, None)
    notices.require(index.get("format_version") == 1 and index.get("registry_archives_verified") is True
                    and identity["release"] == RELEASE and identity["commit_hash"] == COMMIT
                    and index["observed_toolchain"] == distribution["observed_toolchain"] == identity
                    and identity["verbose"] == compiler.replace("\r\n", "\n").strip(),
                    "Toolchain evidence belongs to another compiler/target")
    notices.require(index["target_libraries"] == toolchain.library_fingerprints(libraries),
                    "Installed target libraries differ from the prepared evidence")
    return {"source_distribution_sha256": notices.digest(distribution_bytes),
            "manifest_sha256": notices.digest(manifest_bytes), "inventory_sha256": notices.digest(index_bytes),
            "target": identity["target"], "target_libraries": len(index["target_libraries"]),
            "registry_packages": len(index["registry_packages"]), "review_findings": index["review_findings"],
            "scope": "Source inventory bound to observed compiler and installed library bytes; final binary linkage and source-to-binary reproducibility are not established."}


def attach(root, output, compiler, rustc="rustc"):
    notices.require(not output.exists() and not output.is_symlink(), "Toolchain attachment output must not exist")
    result = validate(root, compiler, rustc)
    shutil.copytree(root, output)
    notices.require(validate(output, compiler, rustc) == result, "Toolchain evidence changed during attachment")
    return result

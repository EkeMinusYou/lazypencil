"""Identify hosted dependency caches without coupling them to job scheduling."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


INPUTS = (
    "Cargo.lock", "Cargo.toml", ".gitattributes", "vendor/skia-bindings",
    "resources/icons/catalogue.json", ".cargo/config.toml", ".cargo/config",
    "rust-toolchain.toml", "rust-toolchain", "scripts/ci/test.sh",
    "scripts/ci/prune_application_cache.sh", "scripts/ci/release-smoke.sh",
)
PLATFORMS = ("ubuntu-24.04", "ubuntu-24.04-arm", "macos-15", "macos-15-intel", "windows-2025")
BASELINE_FLAGS = {
    "CARGO_BUILD_JOBS": "1", "CARGO_INCREMENTAL": "0",
    "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0",
}
# Audited input tree at c9dd6cb. Its dependency sources and compiler policy match
# the successfully saved v1 archives; later input/flag changes disable migration.
AUDITED_TREE = "54c7d2d9cc2e1b5e2d3a0297c8648f27b1aff64c76c685a30e63864b41d60a9f"
# Audited at 4a8d082: only an existing windows-sys normal dependency and its
# Windows JobObjects features were added. Registry versions/checksums, native
# sources and compiler policy are unchanged. Cargo rebuilds changed Rust feature
# sets; each workflow still prunes all application outputs before verification.
AUDITED_PREVIOUS_INPUTS = {
    "c95e9172415f92d6f202a226bf3696f9f6ede08eaa66b81a612b892f8ca3bfc7": AUDITED_TREE,
    # e9c90da only adds .gitattributes rules for Wasm and its new notice tree.
    # All 67 prior native-input paths retain identical attributes; manifests,
    # lockfile, native sources and compiler policy are byte-for-byte unchanged.
    "b24ec03467f4fe02cfb29f305f8a393d83c9588dd4729e01787a36e2b994c62d":
        "c95e9172415f92d6f202a226bf3696f9f6ede08eaa66b81a612b892f8ca3bfc7",
}
AUDITED_RUST_COMMIT = "4a4ef493e3a1488c6e321570238084b38948f6db"
LEGACY_HASHES = {
    "ubuntu-24.04": "cb2b7cd30a1786210e3b9e72b5de57f84878cae8ce5bed69fe87d93e31143d38",
    "ubuntu-24.04-arm": "cb2b7cd30a1786210e3b9e72b5de57f84878cae8ce5bed69fe87d93e31143d38",
    "macos-15": "cb2b7cd30a1786210e3b9e72b5de57f84878cae8ce5bed69fe87d93e31143d38",
    "windows-2025": "95421657d6df7423d75b54047dfd01994254d6f35d60d00b156fb4664bf299e2",
}
FLAG_NAMES = {
    "CARGO_BUILD_JOBS", "CARGO_BUILD_TARGET", "CARGO_INCREMENTAL", "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS", "CC", "CXX", "AR", "CFLAGS", "CXXFLAGS", "LDFLAGS",
    "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "LIBCLANG_PATH", "RUSTC",
    "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTFLAGS",
}
FLAG_PREFIXES = ("CARGO_PROFILE_", "CARGO_TARGET_", "SKIA_", "BINDGEN_", "CC_", "CXX_", "CFLAGS_", "CXXFLAGS_")


class CacheError(Exception):
    pass


def dependency_tree(root):
    base = ["git", "-c", "core.quotePath=true", "-C", str(root)]
    clean = subprocess.run(base + ["diff", "--quiet", "HEAD", "--", *INPUTS])
    untracked = subprocess.run(base + ["ls-files", "--others", "--exclude-standard", "--", *INPUTS], check=True, stdout=subprocess.PIPE).stdout
    if clean.returncode != 0 or untracked:
        raise CacheError("Dependency cache inputs must match the committed source")
    tree = subprocess.run(base + ["ls-tree", "-r", "--full-tree", "HEAD", "--", *INPUTS], check=True, stdout=subprocess.PIPE).stdout
    if b"\tCargo.lock\n" not in tree or b"\tCargo.toml\n" not in tree:
        raise CacheError("Dependency cache inputs need both Cargo manifest and lockfile")
    return hashlib.sha256(tree).hexdigest()


def compile_flags(environment):
    return {
        name: value for name, value in environment.items()
        if value and name != "CARGO_TARGET_DIR"
        and (name in FLAG_NAMES or name.startswith(FLAG_PREFIXES))
    }


def identify(platform, image, tree, compiler, environment):
    if platform not in PLATFORMS or image == "unknown" or not re.fullmatch(r"[A-Za-z0-9_.-]{1,100}", image):
        raise CacheError("A known hosted platform and runner image are required")
    if not re.fullmatch(r"[0-9a-f]{64}", tree):
        raise CacheError("Dependency input fingerprint is invalid")
    flags = compile_flags(environment)
    def build_fingerprint(inputs):
        payload = json.dumps({"inputs": inputs, "rustc": compiler, "flags": flags}, sort_keys=True, separators=(",", ":")).encode()
        return hashlib.sha256(payload).hexdigest()
    fingerprint = build_fingerprint(tree)
    primary = f"native-v2-{platform}-{image}-{fingerprint}"
    legacy = ""
    compiler_lines = compiler.splitlines()
    audited_policy = ("release: 1.94.0" in compiler_lines
            and "commit-hash: " + AUDITED_RUST_COMMIT in compiler_lines
            and flags == BASELINE_FLAGS)
    if tree in AUDITED_PREVIOUS_INPUTS and audited_policy:
        previous = build_fingerprint(AUDITED_PREVIOUS_INPUTS[tree])
        legacy = f"native-v2-{platform}-{image}-{previous}"
    elif tree == AUDITED_TREE and audited_policy and platform in LEGACY_HASHES:
        legacy = f"native-v1-{platform}-{image}-{LEGACY_HASHES[platform]}"
    return {"primary": primary, "legacy": legacy, "dependency_fingerprint": tree,
            "build_fingerprint": fingerprint, "compile_flag_names": sorted(flags)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", required=True, choices=PLATFORMS)
    parser.add_argument("--image", required=True)
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    try:
        compiler = subprocess.run(["rustc", "-vV"], check=True, stdout=subprocess.PIPE, text=True, encoding="utf-8").stdout
        report = identify(args.platform, args.image, dependency_tree(root), compiler, os.environ)
        if args.github_output:
            with args.github_output.open("a", encoding="utf-8") as stream:
                stream.write("primary=" + report["primary"] + "\nlegacy=" + report["legacy"] + "\n")
        print(json.dumps(report, sort_keys=True))
    except (CacheError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Dependency cache identity unavailable: {error}\n")


if __name__ == "__main__":
    main()

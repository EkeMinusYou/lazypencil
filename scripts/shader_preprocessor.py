"""Verify the GHA-built shader component and its exact source/notice bindings."""

import argparse
import json
from pathlib import Path

import license_notices as notices


def memory_import(binary):
    notices.require(binary[:8] == b"\0asm\x01\0\0\0", "Invalid Wasm module header")
    cursor = 8

    def integer():
        nonlocal cursor
        value = shift = 0
        for _ in range(5):
            notices.require(cursor < len(binary), "Truncated Wasm integer")
            byte = binary[cursor]
            cursor += 1
            value |= (byte & 127) << shift
            if byte < 128:
                return value
            shift += 7
        raise notices.NoticeError("Oversized Wasm integer")

    def string():
        nonlocal cursor
        length = integer()
        notices.require(cursor + length <= len(binary), "Truncated Wasm string")
        value = binary[cursor:cursor + length].decode("utf-8")
        cursor += length
        return value

    found = False
    while cursor < len(binary):
        section = binary[cursor]
        cursor += 1
        size = integer()
        end = cursor + size
        notices.require(end <= len(binary), "Truncated Wasm section")
        if section == 2:
            notices.require(not found and integer() == 1, "Unexpected Wasm import count")
            module, name = string(), string()
            notices.require(cursor < end and binary[cursor] == 2, "Expected only a memory import")
            cursor += 1
            flags, minimum, maximum = integer(), integer(), integer()
            notices.require((module, name, flags, minimum, maximum) == ("env", "memory", 1, 256, 1024), "Unexpected Wasm memory boundary")
            notices.require(cursor == end, "Unlisted Wasm import bytes")
            found = True
        cursor = end
    notices.require(found, "Missing bounded Wasm memory import")
    return {"module": "env", "name": "memory", "initial_pages": 256, "maximum_pages": 1024}


def verify(root, rebuilt=None):
    directory = root / "resources/shader-preprocessor"
    report = json.loads(notices.read(directory / "build.json"))
    binary = notices.read(directory / "module.wasm", 16 * 1024 * 1024)
    notices.require(report["format_version"] == 1 and report["target"] == "wasm32-unknown-unknown", "Invalid shader build identity")
    notices.require(notices.SHA.fullmatch(report["wasm_sha256"]) and len(binary) == report["wasm_bytes"] and notices.digest(binary) == report["wasm_sha256"], "Shader module differs from the verified GHA build")
    notices.require(report["memory_maximum_bytes"] == 64 * 1024 * 1024, "Unexpected shader memory policy")
    memory = memory_import(binary)
    component = root / "src/shader/preprocessor"
    files = {}
    for path in component.rglob("*"):
        if "target" in path.relative_to(component).parts or "__pycache__" in path.parts:
            continue
        notices.require(not path.is_symlink(), "Shader source must not be linked")
        if path.is_file():
            name = path.relative_to(root).as_posix()
            files[name] = notices.digest(notices.read(path))
    notices.require(files == report["source_files"], "Shader component source changed; regenerate the module on GHA")
    evidence = json.loads(notices.read(directory / "verification.json"))
    notices.require(evidence["source"] == report["source_commit"] and evidence["wasm_sha256"] == report["wasm_sha256"] and evidence["source_files_verified"] == len(files), "Shader producer evidence does not bind these inputs")
    legal = root / "resources/licenses/shader-preprocessor"
    packages = notices.verify(component, legal / "packages")
    std_directory = legal / "toolchain/inventory"
    manifest = json.loads(notices.read(std_directory / "manifest.json"))
    names = set()
    for item in manifest["files"]:
        relative = notices.source_path(item["path"])
        notices.require(relative.as_posix() not in names, "Duplicate shader toolchain notice")
        names.add(relative.as_posix())
        data = notices.read(std_directory / relative)
        notices.require(len(data) == item["bytes"] and notices.digest(data) == item["sha256"], "Shader toolchain notice changed")
    actual = {path.relative_to(std_directory).as_posix() for path in std_directory.rglob("*") if path.is_file() or path.is_symlink()}
    notices.require(actual == names | {"manifest.json"}, "Missing or unlisted shader toolchain notice")
    std = json.loads(notices.read(std_directory / "inventory.json"))
    identity = std["observed_toolchain"]
    notices.require(identity["target"] == "wasm32-unknown-unknown" and identity["release"] == "1.94.0" and identity["commit_hash"] == "4a4ef493e3a1488c6e321570238084b38948f6db", "Wrong shader target/compiler notices")
    notices.require(std["target_libraries"], "Missing Wasm standard-library fingerprints")
    if rebuilt is not None:
        notices.require(notices.read(rebuilt, 16 * 1024 * 1024) == binary, "Rebuilt Wasm differs; import the new GHA module and its notices")
    return {"wasm_sha256": report["wasm_sha256"], "wasm_bytes": len(binary), "source_files": len(files), "memory_import": memory, "original_package_notices": packages["rust_packages"], "target_library_notices": len(std["target_libraries"]), "rebuilt_bytes_equal": rebuilt is not None}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--rebuilt", type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(verify(args.root, args.rebuilt)))
    except (notices.NoticeError, OSError, ValueError, KeyError, IndexError) as error:
        parser.exit(1, "Shader component error: " + str(error) + "\n")

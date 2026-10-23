"""Inventory reviewed embedded font/icon declarations and original notices.

This checks source inputs, not final linker inclusion or complete legal review.
Downloaded icon geometry/fonts and user/project/system assets are not bundled.
"""

from pathlib import Path
import hashlib
import json
import os
import re

import license_notices as notices


PACKS = {"lucide", "feather", "phosphor", "Material Symbols Outlined", "Material Symbols Rounded", "Material Symbols Sharp"}
FONT_FILES = {"NotoSansJP.ttf", "OFL.txt", "METADATA.pb"}
EXPECTED = {("src/text.rs", "resources/fonts/NotoSansJP.ttf"),
            ("src/cli.rs", "resources/fonts/OFL.txt"), ("src/cli.rs", "resources/fonts/manifest.json"),
            ("src/icons/mod.rs", "resources/icons/catalogue.json"),
            ("src/icons/mod.rs", "resources/icons/lucide/icons.json"),
            *(("src/icons/mod.rs", "resources/icons/" + name + "/LICENSE") for name in ("lucide", "feather")),
            *(("src/icons/optional.rs", "resources/icons/" + name + "/LICENSE") for name in PACKS - {"lucide", "feather"}),
            ("src/icons/optional.rs", "resources/icons/phosphor/index.json"),
            *(("src/icons/optional.rs", "resources/icons/Material Symbols " + style + "/codepoints.txt")
              for style in ("Outlined", "Rounded", "Sharp"))}
CALL = re.compile(r"include_(?:str|bytes)!\s*\(")
LITERAL = re.compile(r'include_(?:str|bytes)!\s*\(\s*"([^"\\]+)"\s*\)')


def source_file(root, name):
    notices.source_path(name)
    path = root / name
    notices.require(path.is_file() and not path.is_symlink()
                    and path.resolve(strict=True) == Path(os.path.abspath(path)), "Linked or missing asset input: " + name)
    return path


def fingerprint(root, name):
    path = source_file(root, name)
    hasher, size = hashlib.sha256(), 0
    with path.open("rb") as stream:
        while data := stream.read(1024 * 1024):
            size += len(data)
            notices.require(size <= 64 * 1024 * 1024, "Embedded asset exceeds its byte limit")
            hasher.update(data)
    return {"path": name, "bytes": size, "sha256": hasher.hexdigest()}


def declarations(root):
    result, count, size = set(), 0, 0
    notices.require((root / "src").is_dir() and not (root / "src").is_symlink(), "Expected regular Rust source root")
    for path in (root / "src").rglob("*"):
        count += 1
        notices.require(count <= 20000, "Rust declaration inventory exceeds its entry budget")
        notices.require(not path.is_symlink() and path.resolve(strict=True) == Path(os.path.abspath(path)),
                        "Linked Rust source input")
        if not path.is_file() or path.suffix != ".rs":
            continue
        body = notices.read(path).decode("utf-8", "strict")
        size += len(body.encode())
        notices.require(size <= 32 * 1024 * 1024, "Rust declaration inventory exceeds its budget")
        matches = list(LITERAL.finditer(body))
        notices.require([m.start() for m in CALL.finditer(body)] == [m.start() for m in matches],
                        "Review a computed/nonliteral include declaration: " + path.relative_to(root).as_posix())
        for match in matches:
            target = (path.parent / match[1]).resolve(strict=True)
            notices.require(target.is_relative_to(root) and target == Path(os.path.abspath(path.parent / match[1])),
                            "Embedded declaration aliases or escapes its source root")
            name = target.relative_to(root).as_posix()
            if name.startswith(("resources/fonts/", "resources/icons/")):
                pair = (path.relative_to(root).as_posix(), name)
                notices.require(pair not in result, "Review repeated embedded font/icon declarations")
                result.add(pair)
    notices.require(result == EXPECTED, "Review changed embedded font/icon declarations")
    return sorted(result)


def build_inventory(root):
    root = root.resolve(strict=True)
    declared = declarations(root)
    embedded = {name for source, name in declared}
    originals, inputs = {}, {}

    def observe(name):
        value = fingerprint(root, name)
        inputs[name] = {**value, "embedded_declaration": name in embedded}
        return value

    def retain(name):
        value = observe(name)
        originals[name] = notices.read(root / name)
        return value

    font = json.loads(notices.read(source_file(root, "resources/fonts/manifest.json")))
    notices.require(font["family"] == "Noto Sans JP" and font["license"] == "OFL-1.1"
                    and {v["file"] for v in font["files"]} == FONT_FILES and len(font["files"]) == 3,
                    "Review bundled font identity and license declarations")
    for entry in font["files"]:
        name = "resources/fonts/" + entry["file"]
        value = observe(name)
        notices.require(value["sha256"] == entry["sha256"] and value["bytes"] == entry["bytes"],
                        "Font input differs from its source manifest: " + name)
        if entry["file"] != "NotoSansJP.ttf":
            retain(name)
    retain("resources/fonts/manifest.json")
    retain("resources/icons/catalogue.json")
    catalogue = json.loads(originals["resources/icons/catalogue.json"])
    notices.require(isinstance(catalogue, list) and len(catalogue) == len(PACKS)
                    and {v["id"] for v in catalogue} == PACKS
                    and {v["id"] for v in catalogue if v["bundled"]} == {"lucide"},
                    "Review icon catalogue/bundled geometry coverage")
    for descriptor in catalogue:
        identifier = descriptor["id"]
        directory = "resources/icons/" + identifier + "/"
        legal = retain(directory + "LICENSE")
        notices.require(legal["sha256"] == descriptor["license_sha256"], "Icon license differs from its catalogue pin")
        if identifier == "lucide":
            notices.require(observe(directory + "icons.json")["sha256"] == descriptor["data_sha256"],
                            "Embedded Lucide data differs from its catalogue pin")
        elif identifier != "feather":
            name = "index.json" if identifier == "phosphor" else "codepoints.txt"
            notices.require(observe(directory + name)["sha256"] == descriptor["index_sha256"],
                            "Embedded optional icon index differs from its catalogue pin")
            retain(directory + "provenance.json")
            notices.require(json.loads(originals[directory + "provenance.json"])["descriptor"] == descriptor,
                            "Icon provenance descriptor differs from its catalogue")
    notices.require(embedded <= inputs.keys(), "Embedded input lacks attribution coverage")
    index = {"format_version": 1, "scope": "Reviewed source declarations and original font/icon notices; final binary linkage and complete legal review are not established.",
             "declarations": [{"source": source, "input": name} for source, name in declared],
             "inputs": [inputs[name] for name in sorted(inputs)], "font": font, "icon_packs": catalogue,
             "limits": ["All reviewed include declarations are inventoried, including optional metadata whose final linker inclusion is not established.",
                        "Only Noto Sans JP and Lucide geometry are bundled; other icon geometry/fonts are acquired separately.",
                        "User/project/system assets, optional runtime distributions, and other embedded project code need separate applicability review."]}
    return index, originals


def bundle_files(index, originals):
    lines = ["Embedded font and icon notice inventory", "", index["scope"], "", *index["limits"], "",
             "Font: " + index["font"]["family"] + " (" + index["font"]["license"] + ")", ""]
    for pack in index["icon_packs"]:
        lines += [pack["id"] + " " + pack["version"] + " (" + pack["license"] + ")",
                  "Source: " + pack["source_url"], pack.get("copyright", ""), ""]
    for name, data in sorted(originals.items()):
        if name.endswith(("/LICENSE", "/OFL.txt")):
            lines += ["=" * 72, name, "=" * 72, data.decode("utf-8", "strict"), ""]
    files = {**originals, "inventory.json": notices.json_bytes(index), "ASSET-NOTICES.txt": ("\n".join(lines) + "\n").encode()}
    files["manifest.json"] = notices.json_bytes({"format_version": 1, "files": [
        {"path": name, "sha256": notices.digest(data), "bytes": len(data)} for name, data in sorted(files.items())]})
    return files


def collect(root, output):
    notices.require(not output.exists() and not output.is_symlink(), "Asset inventory output must not exist")
    index, originals = build_inventory(root)
    files = bundle_files(index, originals)
    output.mkdir(parents=True)
    for name, data in files.items():
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as stream:
            stream.write(data)
    return {"embedded_declarations": len(index["declarations"]), "asset_inputs": len(index["inputs"]),
            "icon_packs": len(index["icon_packs"]), "original_files": len(originals)}


def verify(root, output):
    index, originals = build_inventory(root)
    expected = bundle_files(index, originals)
    actual = {p.relative_to(output).as_posix(): p for p in output.rglob("*") if not p.is_dir()}
    notices.require(set(actual) == set(expected), "Missing or extra asset notice members")
    for name, data in expected.items():
        notices.require(actual[name].resolve().is_relative_to(output.resolve()) and notices.read(actual[name]) == data,
                        "Stale or modified asset notice input: " + name)
    return {"verified_files": len(expected), "asset_inputs": len(index["inputs"])}

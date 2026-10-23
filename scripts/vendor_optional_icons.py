"""Reproduce optional icon indices and pins from original sources; never execute JS."""

import argparse
import base64
import hashlib
import json
from pathlib import Path
import re
import tarfile
import xml.etree.ElementTree as ET

COMMIT = "10ecc14790612ca8fd164e3f4ffe9c16fc50d5b7"
PHOSPHOR_SHA = "313332be6190b724da24107addd781799b48bf76b13963f24501112ffe1baadd"
PHOSPHOR_SRI = "sha512-v4ARvrip4qBCImOE5rmPUylOEK4iiED9ZyKjcvzuezqMaiRASCHKcRIuvvxL/twvLpkfnEODCOJp5dM4eZilxQ=="
CODEPOINT_SHA = "a7404d9e5a1b63f496cf05ace45d386075886b5a7f123fba9f29527cb6006fde"
LICENSE_SHA = "58d1e17ffe5109a7ae296caafcadfdbe6a7d176f0bc4ab01e12a689b0499d8bd"
FONTS = {
    "Outlined": "f7837ba8573d9a7a74931618ced8307bc81f96d49af296bf6c8052c772ed5282",
    "Rounded": "7e74e36533ccee12c2b81e75bfcf3ccf123d3bb7e7e46d65d94445f23db0c97c",
    "Sharp": "1fd72f99ec1f44ec461c74f99341c1dcfa8c140d7087a2902e747d4ee01e379b",
}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def checked(path, expected):
    body = path.read_bytes()
    if digest(body) != expected:
        raise ValueError(f"Source differs from its pin: {path.name}")
    return body


def catalogue_literal(source):
    """Read the pinned catalogue's data literal, not its executable enum prelude."""
    prefix = "const n = "
    if source.count(prefix) != 1:
        raise ValueError("Unexpected pinned catalogue declaration")
    text = source.split(prefix, 1)[1]
    cursor = 0
    decoder = json.JSONDecoder()

    def space():
        nonlocal cursor
        while cursor < len(text) and text[cursor].isspace():
            cursor += 1

    def value():
        nonlocal cursor
        space()
        if text[cursor] in "[{":
            object_ = text[cursor] == "{"
            close = "}" if object_ else "]"
            cursor += 1
            result = {} if object_ else []
            space()
            while text[cursor] != close:
                if object_:
                    match = re.match(r"[A-Za-z_][A-Za-z_0-9]*\s*:", text[cursor:])
                    if not match:
                        raise ValueError("Unexpected catalogue property")
                    key = match[0][:-1].strip()
                    cursor += len(match[0])
                    if key in result:
                        raise ValueError("Duplicate catalogue property")
                    result[key] = value()
                else:
                    result.append(value())
                space()
                if text[cursor] != ",":
                    break
                cursor += 1
                space()
            if text[cursor] != close:
                raise ValueError("Unclosed catalogue data")
            cursor += 1
            return result
        if text[cursor] == '"' or text[cursor].isdigit():
            result, end = decoder.raw_decode(text, cursor)
            cursor = end
            return result
        match = re.match(r"[ae]\.[A-Z_]+", text[cursor:])
        if not match:
            raise ValueError("Executable expressions are not catalogue data")
        cursor += len(match[0])
        return match[0]  # Categories are not used by the normalized index.

    result = value()
    if not text[cursor:].lstrip().startswith(";\nexport {"):
        raise ValueError("Unexpected catalogue suffix")
    return result


def base_data(identifier, version, box, icons, aliases=None, tags=None):
    return {"schema": 2, "id": identifier, "version": version,
            "view_box": box, "stroke_width": 0.0, "icons": icons,
            "aliases": aliases or {}, "tags": tags or {}}


def phosphor(directory, output):
    body = checked(directory / "phosphor-core-2.1.1.tgz", PHOSPHOR_SHA)
    if "sha512-" + base64.b64encode(hashlib.sha512(body).digest()).decode() != PHOSPHOR_SRI:
        raise ValueError("Phosphor SRI mismatch")
    with tarfile.open(directory / "phosphor-core-2.1.1.tgz") as archive:
        license_ = archive.extractfile("package/LICENSE").read()
        source = archive.extractfile("package/dist/index.mjs").read()
        entries = catalogue_literal(source.decode())
        aliases, tags, icons = {}, {}, {}
        for entry in entries:
            name = entry["name"]
            if name in tags:
                raise ValueError("Duplicate Phosphor name")
            tags[name] = entry["tags"]
            for alias in [entry["pascal_name"], *entry.get("alias", {}).values()]:
                if alias != name:
                    if alias in aliases:
                        raise ValueError("Duplicate Phosphor alias")
                    aliases[alias] = name
        for item in archive.getmembers():
            if item.name.startswith("package/assets/regular/") and item.name.endswith(".svg"):
                name = Path(item.name).stem
                element = ET.fromstring(archive.extractfile(item).read())
                if element.attrib != {"viewBox": "0 0 256 256", "fill": "currentColor"}:
                    raise ValueError("Unexpected Phosphor viewport/paint")
                if len(element) != 1 or element[0].tag.rsplit("}", 1)[-1] != "path" or set(element[0].attrib) != {"d"}:
                    raise ValueError("Unexpected Phosphor path")
                if name in icons:
                    raise ValueError("Duplicate Phosphor geometry")
                icons[name] = [["path", {"d": element[0].attrib["d"], "fill": "currentColor"}]]
        if set(icons) != set(tags) or set(aliases) & set(icons):
            raise ValueError("Phosphor catalogue and geometry differ")
    tags, aliases, icons = [dict(sorted(value.items())) for value in [tags, aliases, icons]]
    index = encode({"tags": tags, "aliases": aliases})
    data = encode(base_data("phosphor", "2.1.1", [0.0, 0.0, 256.0, 256.0], icons, aliases, tags))
    folder = output / "phosphor"
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "LICENSE").write_bytes(license_)
    (folder / "index.json").write_bytes(index)
    (directory / "phosphor.icons.json").write_bytes(data)
    return {"id": "phosphor", "version": "2.1.1",
            "source_url": "https://registry.npmjs.org/@phosphor-icons/core/-/core-2.1.1.tgz",
            "source_sha256": PHOSPHOR_SHA, "source_integrity": PHOSPHOR_SRI,
            "data_sha256": digest(data), "license_sha256": digest(license_), "license": "MIT",
            "icon_count": len(icons), "alias_count": len(aliases), "bundled": False,
            "source_format": "phosphor-tgz", "index_sha256": digest(index),
            "copyright": "Copyright (c) 2023 Phosphor Icons"}, {"catalogue_member": "package/dist/index.mjs", "catalogue_sha256": digest(source), "normalization": "Original regular paths; canonical names, PascalCase and deprecated aliases, tags. Package JavaScript is not executed."}


def material(directory, output, style):
    from urllib.parse import quote
    identifier = "Material Symbols " + style
    filename = "MaterialSymbols" + style + "[FILL,GRAD,opsz,wght]"
    font = checked(directory / (filename + ".ttf"), FONTS[style])
    index = checked(directory / (filename + ".codepoints"), CODEPOINT_SHA)
    license_ = checked(directory / "Material-LICENSE", LICENSE_SHA)
    names = {}
    for line in index.decode().splitlines():
        name, code = line.split()
        number = int(code, 16)
        if not re.fullmatch(r"[a-z0-9_]+", name) or name in names or not (0 < number <= 0x10FFFF) or 0xD800 <= number <= 0xDFFF:
            raise ValueError("Invalid Material codepoint entry")
        names[name] = number
    names = dict(sorted(names.items()))
    data = base_data(identifier, COMMIT, [0.0, -960.0, 960.0, 960.0], {name: [] for name in names})
    data["font"] = names
    encoded = encode(data)
    folder = output / identifier
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "LICENSE").write_bytes(license_)
    (folder / "codepoints.txt").write_bytes(index)
    (directory / ("material-" + style.lower() + ".icons.json")).write_bytes(encoded)
    prefix = "https://raw.githubusercontent.com/google/material-design-icons/" + COMMIT + "/"
    return {"id": identifier, "version": COMMIT, "source_url": prefix + "variablefont/" + quote(filename + ".ttf"),
            "source_sha256": digest(font), "source_integrity": "sha512-" + base64.b64encode(hashlib.sha512(font).digest()).decode(),
            "data_sha256": digest(encoded), "license_sha256": digest(license_), "license": "Apache-2.0",
            "icon_count": len(names), "alias_count": 0, "bundled": False,
            "source_format": "ttf", "index_sha256": digest(index),
            "index_source_url": prefix + "variablefont/" + quote(filename + ".codepoints"),
            "license_source_url": prefix + "LICENSE",
            "copyright": "Copyright 2026 Google LLC.  All Rights Reserved."}, {"commit": COMMIT, "font_bytes": len(font), "font_name_copyright_id": 0, "normalization": "Codepoints unchanged. Font downloaded/installed unchanged. Glyph outlines use FILL=0, GRAD=0, opsz=24 and wght=100..700; default 400."}


def build(directory, output):
    catalogue_path = output / "catalogue.json"
    existing = json.loads(catalogue_path.read_text())
    records = [phosphor(directory, output), *[material(directory, output, style) for style in FONTS]]
    identifiers = {descriptor["id"] for descriptor, _ in records}
    catalogue_path.write_text(json.dumps([d for d in existing if d["id"] not in identifiers] + [d for d, _ in records], indent=2) + "\n")
    for descriptor, provenance in records:
        (output / descriptor["id"] / "provenance.json").write_text(json.dumps({"descriptor": descriptor, **provenance}, indent=2) + "\n")
        print(json.dumps(descriptor))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("sources", type=Path)
    parser.add_argument("--output", type=Path, default=Path("resources/icons"))
    arguments = parser.parse_args()
    build(arguments.sources, arguments.output)

"""Reproduce pinned icon data from upstream npm archives, without running package scripts."""

import argparse
import base64
import hashlib
import json
from pathlib import Path
import tarfile
import xml.etree.ElementTree as ET

SOURCES = {
    "lucide": ("lucide-static", "1.57.0", "afc8602c5231ce64ee227286f93772d0438eddd3b947b645bc14bfd1e49da518", "sha512-nuf4YoaKt3Pq9scXVQ71SLr0Q1/UZiTcfBBivFsCVL6biAzPplTikVWXLetBmK3oFEJS6f2L1rzgEsoJN/Ce0Q==", "ISC AND MIT"),
    "feather": ("feather-icons", "4.29.2", "28b6cad56600feee25c53051d65326621f2675d30d6f3d9bd3175f6117f84476", "sha512-0TaCFTnBTVCz6U+baY2UJNKne5ifGh7sMG4ZC2LoBWCZdIyPa+y6UiR4lEYGws1JOFWdee8KAsAIvu0VcXqiqA==", "MIT"),
}


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def shapes(element):
    result = []
    for child in element:
        kind = child.tag.rsplit("}", 1)[-1]
        assert kind in ["path", "line", "circle", "ellipse", "rect", "polygon", "polyline"], kind
        assert not list(child)
        result.append([kind, dict(sorted(child.attrib.items()))])
    return result


def build(directory, destination):
    descriptors = []
    for pack, (package, version, expected, integrity, license_name) in SOURCES.items():
        archive = directory / f"{package}-{version}.tgz"
        body = archive.read_bytes()
        assert hashlib.sha256(body).hexdigest() == expected
        assert "sha512-" + base64.b64encode(hashlib.sha512(body).digest()).decode() == integrity
        with tarfile.open(archive) as source:
            license_bytes = source.extractfile("package/LICENSE").read()
            aliases = {}
            if pack == "lucide":
                icons = json.load(source.extractfile("package/icon-nodes.json"))
                icons = {name: [[kind, dict(sorted(attributes.items()))] for kind, attributes in value] for name, value in sorted(icons.items())}
                matching = {}
                for name, value in reversed(icons.items()):
                    matching[encode(value)] = name
                for item in source.getmembers():
                    if item.name.startswith("package/icons/") and item.name.endswith(".svg"):
                        name = Path(item.name).stem
                        element = ET.fromstring(source.extractfile(item).read())
                        for attribute, expected_attribute in {"viewBox":"0 0 24 24","fill":"none","stroke":"currentColor","stroke-width":"2","stroke-linecap":"round","stroke-linejoin":"round"}.items():
                            assert element.attrib[attribute] == expected_attribute, (name, attribute)
                        value = shapes(element)
                        if name in icons:
                            assert value == icons[name], name
                        else:
                            assert encode(value) in matching, name
                            aliases[name] = matching[encode(value)]
                tags = json.load(source.extractfile("package/tags.json"))
                tags = dict(sorted(tags.items()))
            else:
                fragments = json.load(source.extractfile("package/dist/icons.json"))
                icons = {name: shapes(ET.fromstring("<svg>" + value + "</svg>")) for name, value in sorted(fragments.items())}
                tags = {}
            data = encode({"schema": 1, "id": pack, "version": version, "view_box": [0.0, 0.0, 24.0, 24.0], "stroke_width": 2.0, "icons": icons, "aliases": dict(sorted(aliases.items())), "tags": tags})
        folder = destination / pack
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "LICENSE").write_bytes(license_bytes)
        if pack == "lucide":
            (folder / "icons.json").write_bytes(data)
        else:
            (directory / "feather-4.29.2.icons.json").write_bytes(data)
        descriptors.append({"id": pack, "version": version, "source_url": f"https://registry.npmjs.org/{package}/-/{package}-{version}.tgz", "source_sha256": expected, "source_integrity": integrity, "data_sha256": hashlib.sha256(data).hexdigest(), "license_sha256": hashlib.sha256(license_bytes).hexdigest(), "license": license_name, "icon_count": len(icons), "alias_count": len(aliases), "bundled": pack == "lucide"})
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "catalogue.json").write_text(json.dumps(descriptors, indent=2) + "\n")
    for descriptor in descriptors:
        print(json.dumps(descriptor))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("archives", type=Path)
    parser.add_argument("--output", type=Path, default=Path("resources/icons"))
    args = parser.parse_args()
    build(args.archives, args.output)

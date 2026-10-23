#!/usr/bin/env python3
"""Generate the fixed 1,000-node native shadow regression scene."""

import argparse
import json
import sys
from pathlib import Path


def fixture():
    children = []
    for index in range(999):
        node = {
            "id": f"cell-{index}",
            "type": "rectangle",
            "x": 12 + (index % 40) * 47,
            "y": 12 + (index // 40) * 40,
            "width": 32,
            "height": 24,
            "cornerRadius": 4,
            "fill": "#2563EB",
        }
        if index == 500:
            node["effect"] = {
                "type": "shadow",
                "offset": {"x": 4, "y": 4},
                "blur": 6,
                "color": "#00000080",
                "blendMode": "multiply",
            }
        children.append(node)
    return {
        "version": "2.21",
        "children": [{
            "id": "canvas",
            "type": "frame",
            "layout": "none",
            "width": 1920,
            "height": 1080,
            "fill": "#F8FAFC",
            "clip": True,
            "children": children,
        }],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, help="Create this file; existing files are not overwritten")
    args = parser.parse_args()
    text = json.dumps(fixture()) + "\n"
    if args.output is None:
        sys.stdout.write(text)
    else:
        try:
            with args.output.open("x", encoding="utf-8") as output:
                output.write(text)
        except OSError as error:
            parser.error(str(error))


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Create the fixed 1,000-expanded-node dashboard and its local PNG asset."""

import argparse
import binascii
import hashlib
import json
from pathlib import Path
import struct
import zlib


def avatar():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", binascii.crc32(kind + data))

    pixels = b"".join(
        b"\0" + b"".join(bytes((37 + x * 5, 99 + y * 3, 235 - x * 2, 255)) for x in range(16))
        for y in range(16)
    )
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(pixels, 9)) + chunk(b"IEND", b"")


def rect(id_, x, y, width, height, fill, **extra):
    return {"id": id_, "type": "rectangle", "x": x, "y": y, "width": width, "height": height, "fill": fill, **extra}


def text(id_, content, x, y, width, size=12, **extra):
    return {
        "id": id_, "type": "text", "content": content, "x": x, "y": y,
        "width": width, "textGrowth": "fixed-width", "fontFamily": "Noto Sans JP",
        "fontSize": size, "lineHeight": 1.3, "fill": "$ink", **extra,
    }


def image(id_, x, y, size):
    return rect(id_, x, y, size, size, {"type": "image", "url": "avatar.png", "mode": "cover"}, cornerRadius=6)


def fixture():
    header = [
        text("brand", "LAZYPENCIL / ANALYTICS", 0, 0, 400, 13, fill="$accent", fontWeight="700"),
        text("heading", "プロジェクトの状況", 0, 28, 760, 28, fontWeight="700"),
        text("description", "日本語・English・部品・画像を含む固定の性能確認用データ", 0, 72, 760, 13, fill="$muted"),
        image("profile", 812, 0, 32),
    ]
    for i, label in enumerate(["概要", "プロジェクト", "チーム", "レポート", "設定"]):
        header.extend([
            rect(f"nav-{i}", i * 152, 112, 140, 32, "$surface", cornerRadius=6),
            rect(f"nav-dot-{i}", i * 152 + 12, 124, 8, 8, "$accent", cornerRadius=4),
            text(f"nav-label-{i}", label, i * 152 + 28, 119, 108),
        ])
    for i, (label, value) in enumerate([("アクティブ", "12"), ("完了したタスク", "2,486"), ("達成率", "94.2%")]):
        header.append({
            "id": f"summary-{i}", "type": "frame", "layout": "none", "x": 880 + i * 330, "y": 0,
            "width": 316, "height": 156, "fill": "$surface", "cornerRadius": 10,
            "children": [
                text(f"summary-label-{i}", label, 16, 14, 284, 13, fill="$muted"),
                text(f"summary-value-{i}", value, 16, 40, 284, 28, fontWeight="700"),
                text(f"summary-delta-{i}", "+12.4% / previous period", 16, 84, 284, 11, fill="$accent"),
                rect(f"summary-progress-{i}", 16, 110, 284, 6, "$accent", cornerRadius=3),
                text(f"summary-note-{i}", "Updated / 本日 09:00", 16, 128, 284, 10, fill="$muted"),
            ],
        })
    header.append(rect("divider", 0, 178, 1872, 1, "$border"))

    content = [
        text("card-title", "プロジェクト 01", 16, 14, 360, 16, fontWeight="700"),
        text("card-value", "1,248", 16, 40, 120, 24, fontWeight="700"),
        text("card-unit", "タスク / tasks", 140, 48, 220, 11, fill="$muted"),
    ]
    for i in range(4):
        content.append(rect(f"grid-{i}", 16, 84 + i * 16, 418, 1, "$border"))
    for i in range(47):
        height = 9 + (i * 17 + i * i * 3) % 39
        content.append(rect(f"bar-{i}", 16 + i * 8.6, 132 - height, 5, height, "$accent", cornerRadius=2))
    for i, label in enumerate(["Week 1", "Week 2", "Week 3", "Week 4"]):
        content.append(text(f"axis-{i}", label, 16 + i * 104, 137, 96, 9, fill="$muted"))
    content.extend([
        rect("legend-dot", 16, 158, 6, 6, "$accent", cornerRadius=3),
        text("legend-label", "チームの進捗 / Team progress", 28, 153, 390, 10, fill="$muted"),
    ])
    for i, label in enumerate(["企画", "Design", "開発", "Review", "検証", "公開準備"]):
        content.extend([
            text(f"row-label-{i}", label, 16, 174 + i * 12, 150, 9),
            text(f"row-value-{i}", f"{72 + i * 4}%", 342, 174 + i * 12, 76, 9, fill="$muted"),
            rect(f"row-progress-{i}", 176, 178 + i * 12, 80 + i * 10, 3, "$accent", cornerRadius=1),
        ])
    content.append(image("card-avatar", 402, 16, 28))
    card = {
        "id": "project-card", "type": "frame", "name": "Reusable project card", "reusable": True,
        "layout": "none", "x": 24, "y": 232, "width": 450, "height": 260,
        "fill": "$surface", "cornerRadius": 12, "clip": True, "children": content,
        "effect": {"type": "shadow", "offset": {"x": 0, "y": 2}, "blur": 4, "color": "#0F172A14"},
    }
    cards = [card]
    for i in range(1, 12):
        cards.append({
            "id": f"project-{i + 1}", "type": "ref", "ref": "project-card",
            "x": 24 + (i % 4) * 468, "y": 232 + (i // 4) * 272,
            "descendants": {"card-title": {"content": f"プロジェクト {i + 1:02}"}, "card-value": {"content": f"{1248 + i * 117:,}"}},
        })
    return {
        "version": "2.21",
        "variables": {name: {"type": "color", "value": value} for name, value in {
            "canvas": "#F8FAFC", "surface": "#FFFFFF", "ink": "#0F172A",
            "muted": "#475569", "border": "#E2E8F0", "accent": "#2563EB",
        }.items()},
        "children": [{
            "id": "dashboard", "type": "frame", "layout": "none", "width": 1920, "height": 1080,
            "clip": True, "fill": "$canvas", "children": [
                {"id": "header", "type": "frame", "layout": "none", "x": 24, "y": 24, "width": 1872, "height": 184, "children": header},
                *cards,
            ],
        }],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", required=True, type=Path, help="Create this new directory; never overwrite an existing fixture")
    args = parser.parse_args()
    data = json.dumps(fixture(), ensure_ascii=False, separators=(",", ":")).encode("utf-8") + b"\n"
    asset = avatar()
    try:
        args.output_dir.mkdir()
        (args.output_dir / "dashboard.pen").write_bytes(data)
        (args.output_dir / "avatar.png").write_bytes(asset)
    except OSError as error:
        parser.error(str(error))
    print(json.dumps({"document_sha256": hashlib.sha256(data).hexdigest(), "asset_sha256": hashlib.sha256(asset).hexdigest()}))


if __name__ == "__main__":
    main()

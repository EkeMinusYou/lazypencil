"""Build original, tiny OpenType fixtures using only Python's standard library.

The rectangular outlines and names are original lazypencil test data (MIT).
These are diagnostic fonts, not fonts intended for real design work.
"""

from pathlib import Path
import argparse
import hashlib
import json
import struct


def u16(*values):
    return struct.pack(">" + "H" * len(values), *values)


def i16(*values):
    return struct.pack(">" + "h" * len(values), *values)


def u32(*values):
    return struct.pack(">" + "I" * len(values), *values)


def padded(data):
    return data + bytes(-len(data) % 4)


def checksum(data):
    body = padded(data)
    return sum(struct.unpack(">" + "I" * (len(body) // 4), body)) & 0xFFFFFFFF


def names(family, style):
    values = {1: family, 2: style, 3: f"lazypencil-test-{family}-{style}", 4: f"{family} {style}", 5: "Version 1.0", 6: (family + "-" + style).replace(" ", ""), 16: family, 17: style}
    strings = bytearray()
    records = bytearray()
    for name, value in values.items():
        body = value.encode("utf-16-be")
        records += u16(3, 1, 0x409, name, len(body), len(strings))
        strings += body
    return u16(0, len(values), 6 + len(records)) + records + strings


def cmap(codepoint):
    # Format 12 handles supplementary characters without surrogate shortcuts.
    subtable = u16(12, 0) + u32(28, 0, 1, codepoint, codepoint, 1)
    return u16(0, 1, 3, 10) + u32(12) + subtable


def glyph(width, skew=0):
    points = [(80, 0), (width - 80, 0), (width - 80 + skew, 700), (80 + skew, 700)]
    header = i16(1, 80, 0, width - 80 + skew, 700) + u16(3, 0)
    flags = bytes([1] * len(points))
    x = [points[0][0]] + [b[0] - a[0] for a, b in zip(points, points[1:])]
    y = [points[0][1]] + [b[1] - a[1] for a, b in zip(points, points[1:])]
    return padded(header + flags + i16(*x) + i16(*y))


def cff_index(values):
    if not values:
        return u16(0)
    body = b"".join(values)
    width = 1 if len(body) < 255 else 2
    offsets = [1]
    for value in values:
        offsets.append(offsets[-1] + len(value))
    return u16(len(values)) + bytes([width]) + b"".join(value.to_bytes(width, "big") for value in offsets) + body


def cff_number(value):
    return bytes([28]) + i16(value)


def cff_table(family, width):
    header = bytes([1, 0, 4, 4])
    name = cff_index([(family + "-Regular").replace(" ", "").encode()])
    empty = cff_index([])
    # Fixed-length short integer operands make offsets reproducible.
    top_size = len(cff_index([bytes(8)]))
    charset_offset = len(header + name) + top_size + len(empty) * 2
    charset = bytes([0]) + u16(34)  # Adobe SID 34 is A.
    charstrings_offset = charset_offset + len(charset)
    top = cff_index([cff_number(charset_offset) + bytes([15]) + cff_number(charstrings_offset) + bytes([17])])
    outline = (cff_number(width) + cff_number(80) + cff_number(0) + bytes([21])
               + cff_number(width - 160) + cff_number(0) + cff_number(0) + cff_number(700)
               + cff_number(160 - width) + cff_number(0) + cff_number(0) + cff_number(-700) + bytes([5, 14]))
    return header + name + top + empty + empty + charset + cff_index([bytes([14]), outline])


def font_tables(family, style="Regular", weight=400, codepoint=65, width=600, cff=False):
    italic = style == "Italic"
    head = u32(0x10000, 0x10000, 0, 0x5F0F3CF5) + u16(3, 1000) + bytes(16)
    head += i16(0, 0, width, 800) + u16((1 if weight >= 700 else 0) | (2 if italic else 0), 8) + i16(2, 1, 0)
    hhea = u32(0x10000) + i16(800, -200, 0) + u16(width) + i16(0, 0, width, 1, 0, 0, 0, 0, 0, 0, 0) + u16(2)
    os2 = bytearray(96)
    os2[0:2] = u16(4)
    os2[2:10] = i16(width) + u16(weight, 5, 0)
    os2[58:62] = b"LPEN"
    os2[62:68] = u16((1 if italic else 0) | (32 if weight >= 700 else 0) | (64 if style == "Regular" else 0), min(codepoint, 65535), min(codepoint, 65535))
    os2[68:78] = i16(800, -200, 0) + u16(800, 200)
    os2[86:96] = i16(500, 700) + u16(0, 32, 1)
    post = u32(0x30000, 0xFFF40000 if italic else 0) + i16(-100, 50) + bytes(20)
    tables = {b"head": head, b"hhea": hhea, b"hmtx": u16(width, 0, width, 80), b"OS/2": bytes(os2), b"post": post, b"cmap": cmap(codepoint), b"name": names(family, style)}
    if cff:
        tables[b"maxp"] = u32(0x5000) + u16(2)
        tables[b"CFF "] = cff_table(family, width)
    else:
        shape = glyph(width, 100 if italic else 0)
        tables[b"glyf"] = shape + shape
        tables[b"loca"] = u32(0, len(shape), len(shape) * 2)
        tables[b"maxp"] = u32(0x10000) + u16(2, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0)
    return tables


def sfnt(tables, start=0):
    count = len(tables)
    power = 2 ** (count.bit_length() - 1)
    magic = b"OTTO" if b"CFF " in tables else u32(0x10000)
    directory = bytearray(magic + u16(count, power * 16, power.bit_length() - 1, count * 16 - power * 16))
    body = bytearray()
    for tag, data in sorted(tables.items()):
        directory += tag + u32(checksum(data), start + 12 + count * 16 + len(body), len(data))
        body += padded(data)
    result = directory + body
    if start == 0:
        index = sorted(tables).index(b"head")
        offset = struct.unpack_from(">I", directory, 12 + index * 16 + 8)[0]
        result[offset + 8:offset + 12] = u32((0xB1B0AFBA - checksum(result)) & 0xFFFFFFFF)
    return result


def collection(fonts):
    body = bytearray()
    offsets = []
    start = 12 + len(fonts) * 4 + 12
    for tables in fonts:
        offsets.append(start + len(body))
        body += sfnt(tables, offsets[-1])
    return b"ttcf" + u32(0x20000, len(fonts), *offsets, 0, 0, 0) + body


def main(destination):
    destination.mkdir(parents=True, exist_ok=True)
    regular = font_tables("LP Test Sans")
    # Family case differences must not split a registered style family.
    bold = font_tables("lp test sans", "Bold", 700, width=820)
    italic = font_tables("LP Test Sans", "Italic")
    supplementary = font_tables("LP Test Supplement", codepoint=0xF0000, width=740)
    cff = font_tables("LP Test CFF", cff=True, width=900)
    files = {"regular.ttf": sfnt(regular), "bold.ttf": sfnt(bold), "italic.ttf": sfnt(italic), "supplement.ttf": sfnt(supplementary), "faces.ttc": collection([regular, bold, italic, supplementary]), "cff.otf": sfnt(cff), "cff.otc": collection([cff, supplementary])}
    manifest = {}
    for name, data in files.items():
        (destination / name).write_bytes(data)
        manifest[name] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
    (destination / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parent.parent / "tests/fixtures/fonts")
    main(parser.parse_args().output)

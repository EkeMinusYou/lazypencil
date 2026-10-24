"""Incrementally decode the raw RGBA Kitty frames emitted by the real TUI."""

import base64
from functools import lru_cache
import sys


class KittyFrames:
    def __init__(self):
        self.images = []
        self.pending = bytearray()
        self.payload = bytearray()
        self.expected = None

    def feed(self, data):
        self.pending.extend(data)
        offset = 0
        while True:
            start = self.pending.find(b"\x1b_G", offset)
            if start < 0:
                # Keep only enough bytes for a marker split across reads.
                self.pending = self.pending[-2:]
                return
            end = self.pending.find(b"\x1b\\", start + 3)
            if end < 0:
                del self.pending[:start]
                assert len(self.pending) <= 8192, "Oversized Kitty protocol packet"
                return
            assert end - start <= 8192, "Oversized Kitty protocol packet"
            packet = bytes(self.pending[start + 3:end])
            parameters, separator, encoded = packet.partition(b";")
            fields = dict(field.split(b"=", 1) for field in parameters.split(b",") if b"=" in field)
            action = fields.get(b"a")
            if action == b"T":
                assert self.expected is None, "A new Kitty frame interrupted an incomplete frame"
                assert fields.get(b"f") == b"32" and b"o" not in fields, "Expected uncompressed RGBA Kitty pixels"
                width, height = int(fields[b"s"]), int(fields[b"v"])
                assert 0 < width <= 8192 and 0 < height <= 8192 and width * height <= 16 * 1024 * 1024
                self.expected = width * height * 4
                self.payload.clear()
            if self.expected is not None and action in (None, b"T"):
                assert separator, "A Kitty image packet needs a payload separator"
                self.payload.extend(base64.b64decode(encoded, validate=True))
                assert len(self.payload) <= self.expected, "Kitty pixels exceed the declared dimensions"
                more = fields.get(b"m", b"0")
                assert more in (b"0", b"1"), "Invalid Kitty continuation marker"
                if more == b"0":
                    assert len(self.payload) == self.expected, "Incomplete Kitty pixel data"
                    self.images.append(bytes(self.payload))
                    self.expected = None
                    self.payload.clear()
            offset = end + 2


@lru_cache(maxsize=1)
def _rgba_colors(frame):
    if len(frame) % 4:
        raise ValueError("Expected complete RGBA pixels")
    # Cache one immutable frame's palette rather than rescanning its channels
    # on every PTY wait predicate. Native byte order is used on both sides.
    return frozenset(memoryview(frame).cast("I"))


def has_rgba_color(frame, color):
    value = bytes(color)
    if len(value) != 4:
        raise ValueError("Expected one RGBA color")
    return int.from_bytes(value, sys.byteorder) in _rgba_colors(frame)

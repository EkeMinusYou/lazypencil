"""Keep the real PTY's image observer strict without replaying old payloads."""

import base64
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "terminal_images", Path(__file__).resolve().parents[1] / "tests/support/terminal_images.py"
)
images = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(images)


def frame(pixels, width, height, chunk=3072):
    result = bytearray()
    parts = [pixels[index:index + chunk] for index in range(0, len(pixels), chunk)]
    for index, part in enumerate(parts):
        parameters = f"a=T,f=32,s={width},v={height}," if index == 0 else ""
        parameters += "m=" + str(int(index + 1 < len(parts)))
        result.extend(b"\x1b_G" + parameters.encode() + b";" + base64.b64encode(part) + b"\x1b\\")
    return bytes(result)


class TerminalImages(unittest.TestCase):
    def test_color_checks_use_pixel_boundaries_and_alpha(self):
        red = bytes([255, 0, 0, 255]) * 8
        blue = bytes([0, 0, 255, 255])
        self.assertIn(blue, red, "The unaligned byte-search regression needs a real counterexample")
        self.assertTrue(images.has_rgba_color(red, (255, 0, 0, 255)))
        self.assertFalse(images.has_rgba_color(red, (0, 0, 255, 255)))
        mixed = red + bytes([0, 255, 0, 128]) + blue
        self.assertTrue(images.has_rgba_color(mixed, (0, 0, 255, 255)))
        self.assertTrue(images.has_rgba_color(mixed, (0, 255, 0, 128)))
        self.assertFalse(images.has_rgba_color(mixed, (0, 255, 0, 255)))

    def test_split_packets_preserve_alpha_and_decode_each_chunk_only_once(self):
        decoder = images.KittyFrames()
        red = bytes([255, 0, 0, 255]) * 4
        blue = bytes([0, 0, 255, 128]) * 4
        stream = (b"\x1b[3;4Htext\x1b_Ga=q,f=24,s=1,v=1;AAAA\x1b\\"
                  + frame(red, 2, 2, 6) + b"more text" + frame(blue, 2, 2, 6)
                  + b"\x1b_Ga=d,d=I,i=31,q=2\x1b\\")
        with patch.object(images.base64, "b64decode", wraps=base64.b64decode) as decoded:
            for value in stream:
                decoder.feed(bytes([value]))
            self.assertEqual(decoder.images, [red, blue])
            self.assertEqual(decoded.call_count, 6)
            for _ in range(100):
                decoder.feed(b"")
                list(decoder.images)
            self.assertEqual(decoded.call_count, 6)
        self.assertLessEqual(len(decoder.pending), 2)

    def test_native_sized_chunks_and_unrelated_text_do_not_retain_old_wire_data(self):
        decoder = images.KittyFrames()
        pixels = bytes(range(256)) * 100
        stream = frame(pixels, 80, 80)
        for offset in range(0, len(stream), 4095):
            decoder.feed(stream[offset:offset + 4095])
        self.assertEqual(decoder.images, [pixels])
        for _ in range(20):
            decoder.feed(b"ordinary terminal output" * 1000)
            self.assertLessEqual(len(decoder.pending), 2)
        self.assertFalse(decoder.payload)

    def test_partial_or_corrupted_pixels_never_become_complete_images(self):
        decoder = images.KittyFrames()
        incomplete = b"\x1b_Ga=T,f=32,s=2,v=2,m=1;" + base64.b64encode(b"first") + b"\x1b\\"
        decoder.feed(incomplete)
        self.assertEqual(decoder.images, [])
        with self.assertRaisesRegex(AssertionError, "Incomplete"):
            decoder.feed(b"\x1b_Gm=0;" + base64.b64encode(b"last") + b"\x1b\\")
        with self.assertRaises(ValueError):
            images.KittyFrames().feed(b"\x1b_Ga=T,f=32,s=1,v=1,m=0;invalid!\x1b\\")
        with self.assertRaisesRegex(AssertionError, "interrupted"):
            decoder = images.KittyFrames()
            decoder.feed(incomplete)
            decoder.feed(frame(bytes(16), 2, 2))

    def test_dimensions_encoding_and_packet_limits_are_checked(self):
        for parameters in ("f=100,s=1,v=1", "f=32,o=z,s=1,v=1", "f=32,s=0,v=1",
                           "f=32,s=8193,v=1", "f=32,s=8192,v=8192"):
            with self.subTest(parameters=parameters), self.assertRaises(AssertionError):
                images.KittyFrames().feed(b"\x1b_Ga=T," + parameters.encode() + b",m=0;AAAA\x1b\\")
        for ending in (b"", b"\x1b\\"):
            with self.assertRaisesRegex(AssertionError, "Oversized"):
                images.KittyFrames().feed(b"\x1b_G" + b"x" * 8193 + ending)


if __name__ == "__main__":
    unittest.main()

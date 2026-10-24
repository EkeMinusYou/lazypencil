import base64
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest


root = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("terminal_screen", root / "tests/support/terminal_screen.py")
terminal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(terminal)


class TerminalScreenTests(unittest.TestCase):
    def test_shrinking_the_screen_clamps_the_cursor_before_erasing_or_writing(self):
        screen = terminal.Screen(width=8, height=3)
        screen.feed(b"\x1b[3;8H")
        screen.resize(2, 1)
        self.assertEqual((screen.x, screen.y), (1, 0))
        screen.feed(b"\x1b[KX")
        self.assertEqual(screen.text(), " X")

    def text(self, output):
        screen = terminal.Screen(width=12, height=1)
        screen.feed(output.encode("utf-8"))
        return screen.text()

    def test_overwriting_either_half_of_a_wide_cell_clears_the_other_half(self):
        self.assertEqual(self.text("あcd\x1b[1;1Ha"), "a cd")
        self.assertEqual(self.text("あcd\x1b[1;2Hb"), " bcd")
        self.assertEqual(self.text("aあc\x1b[1;1H語"), "語 c")

    def test_erasing_a_span_does_not_leave_half_of_a_wide_cell(self):
        self.assertEqual(self.text("abcあde\x1b[1;5H\x1b[K"), "abc")
        self.assertEqual(self.text("abcあde\x1b[1;4H\x1b[1K"), "     de")

    def test_macos_saved_log_refresh_capture_preserves_spaces_and_streamed_utf8(self):
        fixture = json.loads((root / "tests/fixtures/terminal/macos-saved-log-refresh.json").read_text(encoding="utf-8"))
        data = base64.b64decode(fixture["terminal_base64"], validate=True)
        self.assertEqual(hashlib.sha256(data).hexdigest(), fixture["terminal_sha256"])
        for size in (1, 7, 137, len(data)):
            with self.subTest(chunk_bytes=size):
                screen = terminal.Screen()
                for offset in range(0, len(data), size):
                    screen.feed(data[offset:offset + size])
                for text in fixture["expected_text"]:
                    self.assertIn(text, screen.text())


if __name__ == "__main__":
    unittest.main()

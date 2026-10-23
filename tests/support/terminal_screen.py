"""Small VT screen for assertions about Crossterm output, with streaming UTF-8."""

import codecs
import re
import unicodedata


class Screen:
    def __init__(self, width=130, height=34):
        self.width, self.height = width, height
        self.cells = [[" "] * width for _ in range(height)]
        self.x = self.y = 0
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def resize(self, width, height):
        self.width, self.height = width, height
        self.cells = [(row + [" "] * width)[:width] for row in self.cells[:height]]
        self.cells += [[" "] * width for _ in range(height - len(self.cells))]

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        index = 0
        while index < len(self.pending):
            symbol = self.pending[index]
            if symbol == "\x1b":
                if index + 1 == len(self.pending):
                    break
                kind = self.pending[index + 1]
                if kind == "[":
                    match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", self.pending[index:])
                    if not match:
                        break
                    self.csi(match[1], match[3])
                    index += len(match[0])
                    continue
                if kind in "_P]":
                    end = self.pending.find("\x1b\\", index + 2)
                    bell = self.pending.find("\x07", index + 2) if kind == "]" else -1
                    if bell >= 0 and (end < 0 or bell < end):
                        index = bell + 1
                    elif end >= 0:
                        index = end + 2
                    else:
                        break
                    continue
                index += 2
                continue
            if symbol == "\r":
                self.x = 0
            elif symbol == "\n":
                self.y = min(self.height - 1, self.y + 1)
            elif symbol == "\t":
                self.x = min(self.width - 1, (self.x // 8 + 1) * 8)
            elif not unicodedata.category(symbol).startswith("C") and not unicodedata.combining(symbol):
                width = 2 if unicodedata.east_asian_width(symbol) in "WF" else 1
                if self.x < self.width and self.y < self.height:
                    self.cells[self.y][self.x] = symbol
                    if width == 2 and self.x + 1 < self.width:
                        self.cells[self.y][self.x + 1] = ""
                self.x = min(self.width, self.x + width)
            index += 1
        self.pending = self.pending[index:]

    def csi(self, params, command):
        if params.startswith(("?", ">", "=", "<")):
            return
        values = [int(value) if value else 0 for value in params.split(";")]
        first = values[0] or 1
        if command in "Hf":
            self.y = min(self.height - 1, first - 1)
            self.x = min(self.width - 1, ((values + [0])[1] or 1) - 1)
        elif command == "A":
            self.y = max(0, self.y - first)
        elif command == "B":
            self.y = min(self.height - 1, self.y + first)
        elif command == "C":
            self.x = min(self.width - 1, self.x + first)
        elif command == "D":
            self.x = max(0, self.x - first)
        elif command == "G":
            self.x = min(self.width - 1, first - 1)
        elif command == "d":
            self.y = min(self.height - 1, first - 1)
        elif command == "J" and values[0] in [2, 3]:
            self.cells = [[" "] * self.width for _ in range(self.height)]
        elif command == "K":
            begin, end = (0, self.width) if values[0] == 2 else ((0, self.x + 1) if values[0] == 1 else (self.x, self.width))
            self.cells[self.y][begin:end] = [" "] * (end - begin)

    def text(self):
        return "\n".join("".join(row).rstrip() for row in self.cells)

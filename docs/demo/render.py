#!/usr/bin/env python3
"""Renders an asciicast from record.py as a GIF (needs Pillow, and ffmpeg for the palette).

    python docs/demo/render.py demo.cast docs/demo/calc.gif

A small terminal emulator: what bash, nano and A0's diagnostics write (text, colors, cursor
movement, erasing, scroll regions, the alternate screen), not all of xterm.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFont

FPS = 20
FONT_SIZE = 15
PAD = 18
BAR = 34
BG = (24, 26, 33)
FG = (216, 222, 233)
# The 16 ANSI colors (One Dark-like), normal then bright.
PALETTE = [
    (40, 44, 52), (224, 108, 117), (152, 195, 121), (229, 192, 123),
    (97, 175, 239), (198, 120, 221), (86, 182, 194), (171, 178, 191),
    (92, 99, 112), (240, 128, 136), (172, 215, 141), (239, 206, 143),
    (117, 190, 255), (214, 140, 235), (106, 202, 214), (230, 235, 245),
]
FONTS = [
    "C:/Windows/Fonts/CascadiaMono.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/System/Library/Fonts/Menlo.ttc",
]


def color256(n):
    if n < 16:
        return PALETTE[n]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return (steps[n // 36], steps[n // 6 % 6], steps[n % 6])
    v = 8 + (n - 232) * 10
    return (v, v, v)


class Cell:
    __slots__ = ("ch", "fg", "bg", "bold", "rev")

    def __init__(self, ch=" ", fg=None, bg=None, bold=False, rev=False):
        self.ch, self.fg, self.bg, self.bold, self.rev = ch, fg, bg, bold, rev


class Screen:
    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.main = self.blank()
        self.alt = None
        self.grid = self.main
        self.x = self.y = 0
        self.saved = self.alt_saved = (0, 0)
        self.top, self.bottom = 0, rows - 1
        self.fg = self.bg = None
        self.bold = self.rev = False
        self.cursor = True
        self.wrap = False
        self.pending = ""

    def blank(self):
        return [[Cell() for _ in range(self.cols)] for _ in range(self.rows)]

    def cell(self):
        return Cell(" ", None, self.bg)

    def scroll_up(self, n=1):
        for _ in range(n):
            del self.grid[self.top]
            self.grid.insert(self.bottom, [self.cell() for _ in range(self.cols)])

    def scroll_down(self, n=1):
        for _ in range(n):
            del self.grid[self.bottom]
            self.grid.insert(self.top, [self.cell() for _ in range(self.cols)])

    def newline(self):
        if self.y == self.bottom:
            self.scroll_up()
        elif self.y < self.rows - 1:
            self.y += 1

    def put(self, ch):
        if self.wrap:
            self.x, self.wrap = 0, False
            self.newline()
        self.grid[self.y][self.x] = Cell(ch, self.fg, self.bg, self.bold, self.rev)
        if self.x == self.cols - 1:
            self.wrap = True
        else:
            self.x += 1

    def feed(self, data):
        data = self.pending + data
        self.pending = ""
        i = 0
        while i < len(data):
            c = data[i]
            if c == "\x1b":
                m = re.compile(r"\x1b(\[[?>=]?[0-9;]*[ -/]*[@-~]|\][^\x07\x1b]*(\x07|\x1b\\)|[()][0-9A-Za-z]|[78DEMc=>])").match(data, i)
                if not m:
                    self.pending = data[i:]
                    return
                self.escape(m.group(1))
                i = m.end()
                continue
            if c == "\r":
                self.x, self.wrap = 0, False
            elif c == "\n":
                self.wrap = False
                self.newline()
            elif c == "\b":
                self.x, self.wrap = max(0, self.x - 1), False
            elif c == "\t":
                self.x = min(self.cols - 1, (self.x // 8 + 1) * 8)
            elif c >= " ":
                self.put(c)
            i += 1

    def escape(self, s):
        if s[0] == "[":
            self.csi(s[1:])
        elif s == "7":
            self.saved = (self.x, self.y)
        elif s == "8":
            self.x, self.y = self.saved
        elif s == "M":
            if self.y == self.top:
                self.scroll_down()
            else:
                self.y = max(0, self.y - 1)
        elif s == "D":
            self.newline()
        elif s == "E":
            self.x = 0
            self.newline()

    def csi(self, s):
        final, body = s[-1], s[:-1]
        private = body[:1] in ("?", ">", "=")
        if private:
            mode, body = body[0], body[1:]
        args = [int(a) if a.isdigit() else None for a in body.split(";")] if body else []

        def arg(k, default):
            return args[k] if k < len(args) and args[k] is not None else default

        self.wrap = False
        if private:
            if mode == "?" and final in "hl":
                on = final == "h"
                for a in args:
                    if a == 25:
                        self.cursor = on
                    elif a in (47, 1047, 1049):
                        if on and self.alt is None:
                            self.alt_saved = (self.x, self.y)
                            self.alt = self.grid = self.blank()
                        elif not on and self.alt is not None:
                            self.alt, self.grid = None, self.main
                            self.x, self.y = self.alt_saved
            return
        g = self.grid
        if final in "Hf":
            self.y = min(self.rows - 1, arg(0, 1) - 1)
            self.x = min(self.cols - 1, arg(1, 1) - 1)
        elif final == "A":
            self.y = max(0, self.y - arg(0, 1))
        elif final in "Be":
            self.y = min(self.rows - 1, self.y + arg(0, 1))
        elif final in "Ca":
            self.x = min(self.cols - 1, self.x + arg(0, 1))
        elif final == "D":
            self.x = max(0, self.x - arg(0, 1))
        elif final in "G`":
            self.x = min(self.cols - 1, arg(0, 1) - 1)
        elif final == "d":
            self.y = min(self.rows - 1, arg(0, 1) - 1)
        elif final == "J":
            n = arg(0, 0)
            rows = {0: range(self.y + 1, self.rows), 1: range(0, self.y), 2: range(self.rows), 3: []}[n]
            for r in rows:
                g[r] = [self.cell() for _ in range(self.cols)]
            if n == 0:
                self.erase(self.x, self.cols)
            elif n == 1:
                self.erase(0, self.x + 1)
        elif final == "K":
            n = arg(0, 0)
            self.erase(*{0: (self.x, self.cols), 1: (0, self.x + 1), 2: (0, self.cols)}[n])
        elif final == "X":
            self.erase(self.x, min(self.cols, self.x + arg(0, 1)))
        elif final == "P":
            row = g[self.y]
            n = min(arg(0, 1), self.cols - self.x)
            del row[self.x : self.x + n]
            row.extend(self.cell() for _ in range(n))
        elif final == "@":
            row = g[self.y]
            n = min(arg(0, 1), self.cols - self.x)
            for _ in range(n):
                row.insert(self.x, self.cell())
            del row[self.cols :]
        elif final in "LM":
            if self.top <= self.y <= self.bottom:
                top = self.top
                self.top = self.y
                (self.scroll_down if final == "L" else self.scroll_up)(arg(0, 1))
                self.top = top
        elif final == "S":
            self.scroll_up(arg(0, 1))
        elif final == "T":
            self.scroll_down(arg(0, 1))
        elif final == "r":
            self.top, self.bottom = arg(0, 1) - 1, arg(1, self.rows) - 1
            self.x = self.y = 0
        elif final == "m":
            self.sgr(args or [0])

    def erase(self, a, b):
        row = self.grid[self.y]
        for c in range(a, b):
            row[c] = self.cell()

    def sgr(self, args):
        it = iter(a or 0 for a in args)
        for a in it:
            if a == 0:
                self.fg = self.bg = None
                self.bold = self.rev = False
            elif a == 1:
                self.bold = True
            elif a == 22:
                self.bold = False
            elif a == 7:
                self.rev = True
            elif a == 27:
                self.rev = False
            elif 30 <= a <= 37:
                self.fg = PALETTE[a - 30]
            elif 90 <= a <= 97:
                self.fg = PALETTE[a - 90 + 8]
            elif 40 <= a <= 47:
                self.bg = PALETTE[a - 40]
            elif 100 <= a <= 107:
                self.bg = PALETTE[a - 100 + 8]
            elif a == 39:
                self.fg = None
            elif a == 49:
                self.bg = None
            elif a in (38, 48):
                kind = next(it, None)
                if kind == 5:
                    c = color256(next(it, 0))
                elif kind == 2:
                    c = (next(it, 0), next(it, 0), next(it, 0))
                else:
                    continue
                if a == 38:
                    self.fg = c
                else:
                    self.bg = c


class Painter:
    def __init__(self, cols, rows):
        path = next(f for f in FONTS if os.path.exists(f))
        self.font = ImageFont.truetype(path, FONT_SIZE)
        try:
            self.bold = ImageFont.truetype(path, FONT_SIZE, layout_engine=None)
            self.bold.set_variation_by_name("Bold")
        except Exception:
            self.bold = self.font
        self.cw = round(self.font.getlength("M"))
        self.ch = round(FONT_SIZE * 1.4)
        self.cols, self.rows = cols, rows
        self.size = (PAD * 2 + cols * self.cw, BAR + PAD + rows * self.ch + PAD)

    def frame(self, s):
        img = Image.new("RGB", self.size, BG)
        d = ImageDraw.Draw(img)
        d.rectangle([0, 0, self.size[0], BAR - 6], fill=(33, 36, 45))
        for k, c in enumerate([(255, 95, 87), (254, 188, 46), (40, 200, 64)]):
            x = PAD + k * 20
            d.ellipse([x, 9, x + 12, 21], fill=c)
        title = "a0 — Calc"
        d.text(((self.size[0] - self.font.getlength(title)) / 2, 6), title, font=self.font, fill=(140, 147, 160))
        for r, row in enumerate(s.grid):
            y = BAR + PAD + r * self.ch
            for c, cell in enumerate(row):
                fg, bg = cell.fg or FG, cell.bg
                if cell.rev:
                    fg, bg = bg or BG, cell.fg or FG
                if cell.bold and cell.fg in PALETTE[:8]:
                    fg = PALETTE[PALETTE.index(cell.fg) + 8]
                x = PAD + c * self.cw
                if bg:
                    d.rectangle([x, y, x + self.cw - 1, y + self.ch - 1], fill=bg)
                if cell.ch != " ":
                    d.text((x, y + 2), cell.ch, font=self.bold if cell.bold else self.font, fill=fg)
        if s.cursor:
            x, y = PAD + s.x * self.cw, BAR + PAD + s.y * self.ch
            d.rectangle([x, y + 1, x + self.cw - 1, y + self.ch - 2], fill=(200, 205, 215))
            cell = s.grid[s.y][s.x]
            if cell.ch != " ":
                d.text((x, y + 2), cell.ch, font=self.font, fill=BG)
        return img


def main():
    cast, out = sys.argv[1], sys.argv[2]
    with open(cast, encoding="utf-8") as f:
        header = json.loads(f.readline())
        events = [json.loads(line) for line in f if line.strip()]
    screen = Screen(header["width"], header["height"])
    painter = Painter(header["width"], header["height"])
    end = events[-1][0] + 2.5
    tmp = tempfile.mkdtemp()
    k = 0
    for n in range(int(end * FPS) + 1):
        t = n / FPS
        while k < len(events) and events[k][0] <= t:
            if events[k][1] == "o":
                screen.feed(events[k][2])
            k += 1
        painter.frame(screen).save(os.path.join(tmp, f"{n:05}.png"))
    subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-y", "-framerate", str(FPS), "-i", os.path.join(tmp, "%05d.png"),
         "-vf", "split[a][b];[a]palettegen=max_colors=64:stats_mode=full[p];[b][p]paletteuse=dither=none",
         "-loop", "0", out],
        check=True,
    )
    shutil.rmtree(tmp)
    print(f"{out}: {end:.1f} s, {painter.size[0]}x{painter.size[1]}, {os.path.getsize(out) // 1024} KB")


if __name__ == "__main__":
    main()

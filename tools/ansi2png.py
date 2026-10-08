import re
import sys
import unicodedata

from PIL import Image, ImageDraw, ImageFont

FONT_DIR = "/usr/share/fonts/TTF/"
REG = ImageFont.truetype(FONT_DIR + "CaskaydiaMonoNerdFontMono-Regular.ttf", 16)
BOLD = ImageFont.truetype(FONT_DIR + "CaskaydiaMonoNerdFontMono-Bold.ttf", 16)
CW, CH = 10, 21

XTERM16 = [
    (0, 0, 0), (205, 0, 0), (0, 205, 0), (205, 205, 0), (0, 0, 238), (205, 0, 205), (0, 205, 205), (229, 229, 229),
    (127, 127, 127), (255, 0, 0), (0, 255, 0), (255, 255, 0), (92, 92, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
]


def xterm256(n):
    if n < 16:
        return XTERM16[n]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return (steps[n // 36], steps[(n // 6) % 6], steps[n % 6])
    v = 8 + (n - 232) * 10
    return (v, v, v)


DEFAULT_FG, DEFAULT_BG = (220, 220, 220), (20, 20, 24)
SGR = re.compile(r"\x1b\[([0-9;:]*)m")


def width(ch):
    return 2 if unicodedata.east_asian_width(ch) in "WF" else 1


def parse(text):
    rows = []
    fg, bg, bold = None, None, False
    for line in text.split("\n"):
        row, pos = [], 0
        for m in re.finditer(r"\x1b\[[0-9;:]*m|[^\x1b]", line):
            tok = m.group(0)
            if tok.startswith("\x1b"):
                params = [int(p) if p else 0 for p in re.split("[;:]", tok[2:-1])] or [0]
                i = 0
                while i < len(params):
                    p = params[i]
                    if p == 0:
                        fg, bg, bold = None, None, False
                    elif p == 1:
                        bold = True
                    elif p == 22:
                        bold = False
                    elif p in (38, 48) and i + 1 < len(params):
                        if params[i + 1] == 2:
                            col = tuple(params[i + 2:i + 5]); i += 4
                        else:
                            col = xterm256(params[i + 2]); i += 2
                        if p == 38:
                            fg = col
                        else:
                            bg = col
                    elif p == 39:
                        fg = None
                    elif p == 49:
                        bg = None
                    elif 30 <= p <= 37:
                        fg = XTERM16[p - 30]
                    elif 90 <= p <= 97:
                        fg = XTERM16[p - 82]
                    elif 40 <= p <= 47:
                        bg = XTERM16[p - 40]
                    i += 1
                continue
            row.append((tok, fg, bg, bold))
        rows.append(row)
    return rows


def draw_block(d, ch, x, y, fg):
    eighths = "▁▂▃▄▅▆▇█"
    if ch in eighths:
        h = round(CH * (eighths.index(ch) + 1) / 8)
        d.rectangle([x, y + CH - h, x + CW - 1, y + CH - 1], fill=fg)
    elif ch == "▀":
        d.rectangle([x, y, x + CW - 1, y + CH // 2 - 1], fill=fg)
    elif ch == "▌":
        d.rectangle([x, y, x + CW // 2 - 1, y + CH - 1], fill=fg)
    else:
        return False
    return True


def render(src, dst):
    rows = parse(open(src, encoding="utf-8", errors="replace").read().rstrip("\n"))
    cols = max(sum(width(c[0]) for c in r) for r in rows)
    img = Image.new("RGB", (cols * CW, len(rows) * CH), DEFAULT_BG)
    d = ImageDraw.Draw(img)
    for ry, row in enumerate(rows):
        x = 0
        for ch, fg, bg, bold in row:
            w = width(ch)
            px, py = x * CW, ry * CH
            d.rectangle([px, py, px + w * CW - 1, py + CH - 1], fill=bg or DEFAULT_BG)
            col = fg or DEFAULT_FG
            if ch != " " and not draw_block(d, ch, px, py, col):
                d.text((px, py + 1), ch, font=BOLD if bold else REG, fill=col)
            x += w
    img.save(dst)


if __name__ == "__main__":
    render(sys.argv[1], sys.argv[2])

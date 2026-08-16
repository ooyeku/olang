#!/usr/bin/env python3
"""Generate every olang brand asset from one set of design tokens.

The mark is *the through-line o*: a bold ring — the letter olang is named
for — with the pipeline running straight through it and out both sides.
It states the language's thesis in one glyph: the flow goes in and comes
out, and nothing is concealed inside. It is deliberately not a chevron in
a circle, which every media player on earth already owns.

Everything is hand-authored geometry: no fonts, no external tools, no
rasterizer dependency (the PNG icons are supersampled and encoded here
with stdlib zlib), so every asset renders identically everywhere and can
be regenerated from the tokens below.

Run from branding/:  python3 generate.py
"""

import math
import os
import struct
import zlib

# ── design tokens ──────────────────────────────────────────────────────
TEAL = "#2DD4BF"        # primary on dark
TEAL_DEEP = "#0D9488"   # primary on light (holds 4.5:1 against paper)
AMBER = "#FBBF24"       # accent on dark
AMBER_DEEP = "#D97706"  # accent on light
INK = "#0B1220"         # near-black (light-bg text, dark surfaces)
PAPER = "#E8EFF5"       # off-white (dark-bg text)
SLATE = "#7C93A8"       # secondary text

# ── the mark ───────────────────────────────────────────────────────────
# One 512 canvas, one set of numbers, used by every rendering of the mark.
MC = 256.0        # centre
RING_R = 168.0    # ring centreline radius
RING_W = 68.0     # ring stroke weight  -> outer edge at 202
BAR_H = 52.0      # through-line weight
BAR_X0 = 34.0     # left end   (overhangs the ring by 20)
BAR_X1 = 478.0    # right end  (overhangs by 20)


def mark_parts(cx, cy, scale, ring_color, bar_color):
    """The mark, positioned and scaled. Returns a list of SVG elements."""
    t = f"translate({fmt(cx)} {fmt(cy)}) scale({fmt(scale)})"
    r = BAR_H / 2
    return [
        f'<g transform="{t}">'
        f'<circle cx="0" cy="0" r="{fmt(RING_R)}" fill="none" '
        f'stroke="{ring_color}" stroke-width="{fmt(RING_W)}"/>'
        f'<rect x="{fmt(BAR_X0 - MC)}" y="{fmt(-r)}" '
        f'width="{fmt(BAR_X1 - BAR_X0)}" height="{fmt(BAR_H)}" '
        f'rx="{fmt(r)}" fill="{bar_color}"/>'
        f"</g>"
    ]


# ── wordmark letterforms ───────────────────────────────────────────────
# Monoline geometry tuned to the mark: the same stroke-to-body ratio, and
# round terminals only where a stroke genuinely ends. Round letters
# overshoot the x-height band by OVER, the standard optical correction
# that stops o/a/g reading as short beside the flat-topped n.
STROKE = 28
XH = 132                     # x-height
BASE = 178                   # baseline
TOP = BASE - XH              # 46
ASC = 6                      # ascender top (l)
DESC = 250                   # descender bottom (g)
R = 53                       # bowl radius
OVER = 3                     # optical overshoot on round letters
GAP = 22                     # inter-letter gap
H = STROKE / 2


def fmt(v):
    """Trim float noise so the SVG stays readable and diffs stay small."""
    return f"{v:.10g}"


def _bowl(cx, cy, color, rx=R, ry=None):
    ry = R if ry is None else ry
    return (f'<ellipse cx="{fmt(cx)}" cy="{fmt(cy)}" rx="{fmt(rx)}" ry="{fmt(ry)}" '
            f'fill="none" stroke="{color}" stroke-width="{STROKE}"/>')


def _stem(x, y0, y1, color, cap="butt"):
    return (f'<path d="M {fmt(x)} {fmt(y0)} L {fmt(x)} {fmt(y1)}" fill="none" '
            f'stroke="{color}" stroke-width="{STROKE}" stroke-linecap="{cap}"/>')


def letter_o(x, color):
    cx = x + H + R
    return [_bowl(cx, BASE - H - R, color, R, R + OVER)], x + STROKE + 2 * R


def letter_l(x, color):
    return [_stem(x + H, ASC, BASE - H, color, "round")], x + STROKE


def letter_a(x, color):
    cx = x + H + R
    sx = cx + R
    return [
        _bowl(cx, BASE - H - R, color, R, R + OVER),
        _stem(sx, TOP + H, BASE - H, color, "round"),
    ], x + STROKE + 2 * R


def letter_n(x, color):
    lx = x + H
    rx = lx + 96
    mid = (lx + rx) / 2
    return [
        _stem(lx, TOP + H, BASE - H, color, "round"),
        f'<path d="M {fmt(lx)} {fmt(TOP + 56)} C {fmt(lx)} {fmt(TOP + 10)} '
        f'{fmt(mid - 20)} {fmt(TOP - OVER)} {fmt(mid)} {fmt(TOP - OVER)} '
        f'C {fmt(mid + 20)} {fmt(TOP - OVER)} {fmt(rx)} {fmt(TOP + 10)} '
        f'{fmt(rx)} {fmt(TOP + 56)}" fill="none" stroke="{color}" '
        f'stroke-width="{STROKE}" stroke-linecap="butt"/>',
        _stem(rx, TOP + 50, BASE - H, color, "round"),
    ], x + STROKE + 96


def letter_g(x, color):
    cx = x + H + R
    sx = cx + R
    return [
        _bowl(cx, BASE - H - R, color, R, R + OVER),
        # the tail: straight down out of the bowl, then a short hook left
        f'<path d="M {fmt(sx)} {fmt(TOP + H)} L {fmt(sx)} {fmt(DESC - 46)} '
        f'C {fmt(sx)} {fmt(DESC)} {fmt(sx - 38)} {fmt(DESC + 6)} '
        f'{fmt(sx - 70)} {fmt(DESC - 6)}" fill="none" stroke="{color}" '
        f'stroke-width="{STROKE}" stroke-linecap="round"/>',
    ], x + STROKE + 2 * R


LETTERS = {"o": letter_o, "l": letter_l, "a": letter_a, "n": letter_n, "g": letter_g}


def wordmark_parts(text, x, color):
    parts = []
    for i, ch in enumerate(text):
        got, x = LETTERS[ch](x, color)
        parts += got
        if i < len(text) - 1:
            x += GAP
    return parts, x


# ── SVG plumbing ───────────────────────────────────────────────────────
def svg(w, h, body, label):
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {fmt(w)} {fmt(h)}" '
            f'role="img" aria-label="{label}">\n  '
            + "\n  ".join(body) + "\n</svg>\n")


def write(name, content):
    with open(name, "w") as f:
        f.write(content)
    print(f"  {name}")


# ── assets ─────────────────────────────────────────────────────────────
def gen_marks():
    for name, ring, bar in [
        ("mark.svg", TEAL, AMBER),
        ("mark-light.svg", TEAL_DEEP, AMBER_DEEP),
        ("mark-mono.svg", "currentColor", "currentColor"),
    ]:
        write(name, svg(512, 512, mark_parts(MC, MC, 1.0, ring, bar), "olang mark"))

    # The favicon carries its own ink tile: a bare mark on a transparent
    # square disappears into a dark browser chrome.
    body = [f'<rect width="512" height="512" rx="112" fill="{INK}"/>'] + \
        mark_parts(MC, MC, 0.80, TEAL, AMBER)
    write("favicon.svg", svg(512, 512, body, "olang"))


def gen_wordmarks():
    for name, color in [("wordmark.svg", PAPER), ("wordmark-light.svg", INK)]:
        parts, w = wordmark_parts("olang", 8, color)
        write(name, svg(w + 8, 262, parts, "olang"))


def gen_lockups():
    """mark + 'lang', the mark standing in as the word's own first letter."""
    for name, bar, rest in [
        ("logo.svg", AMBER, PAPER),
        ("logo-light.svg", AMBER_DEEP, INK),
        ("logo-mono.svg", "currentColor", "currentColor"),
    ]:
        ring = {"logo.svg": TEAL, "logo-light.svg": TEAL_DEEP}.get(name, "currentColor")
        # The mark's ring outer diameter (404) maps onto the *overshot*
        # bowl height, not the plain x-height: the wordmark's own o
        # overshoots the band, and the mark has to match what sits beside
        # it rather than the theoretical line.
        scale = (XH + 2 * OVER) / 404
        cx, cy = 8 + XH / 2, TOP + XH / 2
        mark = mark_parts(cx, cy, scale, ring, bar)
        text, w = wordmark_parts("lang", 8 + XH + GAP, rest)
        # The bar overhangs the ring, so the lockup needs air on the left.
        pad = (MC - BAR_X0) * scale - XH / 2
        write(name, svg(w + 8 + pad, 262, mark + text, "olang"))


def gen_banner():
    W, HGT = 1200, 420
    scale = 0.66
    parts, word_w = wordmark_parts("lang", 8 + XH + GAP, PAPER)
    lock_w = (word_w + 8) * scale
    ox = (W - lock_w) / 2
    oy = 96
    mark = mark_parts(ox + (8 + XH / 2) * scale, oy + (TOP + XH / 2) * scale,
                      ((XH + 2 * OVER) / 404) * scale, TEAL, AMBER)
    text, _ = wordmark_parts("lang", 8 + XH + GAP, PAPER)
    mono = ("ui-monospace, SFMono-Regular, Menlo, Consolas, monospace")
    body = [
        f'<rect width="{W}" height="{HGT}" fill="{INK}"/>',
        # The through-line, extended across the whole card at a whisper —
        # the mark's own gesture, used as the composition's spine.
        f'<rect x="0" y="{HGT / 2 - 1.5}" width="{W}" height="3" fill="{TEAL}" opacity="0.10"/>',
        f'<g opacity="0.05">' + mark_parts(W - 40, HGT / 2, 0.85, TEAL, TEAL)[0] + "</g>",
        f'<text x="48" y="56" font-family="{mono}" font-size="18" fill="#3E5468">'
        f'olang&gt; [1, 2, 3] |&gt; map((x) =&gt; x * x) |&gt; sum</text>',
    ] + mark + [
        f'<g transform="translate({fmt(ox)} {fmt(oy)}) scale({fmt(scale)})">'
        + "".join(text) + "</g>",
        f'<text x="{W / 2}" y="322" text-anchor="middle" font-family="{mono}" '
        f'font-size="23" letter-spacing="1" fill="{PAPER}">The Open Language</text>',
        f'<text x="{W / 2}" y="362" text-anchor="middle" font-family="{mono}" '
        f'font-size="17" letter-spacing="1.5" fill="{SLATE}">'
        f'open code &#183; open artifacts &#183; open execution</text>',
    ]
    write("banner.svg", svg(W, HGT, body, "olang - the Open Language"))


# ── PNG icons ──────────────────────────────────────────────────────────
# The mark is analytic geometry (an annulus and a rounded bar), so the
# icons are supersampled directly rather than shelling out to a
# rasterizer that may not be installed. 4x4 samples per pixel is past the
# point where more helps at these sizes.
def _hex(c):
    return tuple(int(c[i:i + 2], 16) for i in (1, 3, 5))


def _card_pixels(w, h, ss=3):
    """The social card: the mark centred on ink, with its through-line
    extended to both edges — the glyph's own gesture used as the card's
    spine. No lettering: the words come from og:title, and a wordmark
    rasterized without a real type engine looks worse than none."""
    ink = _hex(INK)
    teal = _hex(TEAL)
    amber = _hex(AMBER)
    rule = (0x1B, 0x2B, 0x3A)
    scale = 340.0 / 404.0        # mark outer diameter -> 340px
    cx, cy = w / 2.0, h / 2.0
    r_in = (RING_R - RING_W / 2) * scale
    r_out = (RING_R + RING_W / 2) * scale
    bar_r = (BAR_H / 2) * scale
    bx0 = cx - (MC - BAR_X0) * scale
    bx1 = cx + (BAR_X1 - MC) * scale
    rows = []
    for py in range(h):
        row = bytearray()
        for px in range(w):
            acc = [0.0, 0.0, 0.0]
            for sy in range(ss):
                for sx in range(ss):
                    x = px + (sx + 0.5) / ss
                    y = py + (sy + 0.5) / ss
                    col = ink
                    # a hairline rule across the whole card, at 6% teal
                    if abs(y - cy) <= 1.0:
                        col = rule
                    d = math.hypot(x - cx, y - cy)
                    if r_in <= d <= r_out:
                        col = teal
                    # The bar keeps the mark's own overhang: a rule spanning
                    # the whole card would read as a horizon, not a glyph.
                    if abs(y - cy) <= bar_r and bx0 <= x <= bx1:
                        if bx0 + bar_r <= x <= bx1 - bar_r:
                            col = amber
                    if abs(y - cy) <= bar_r:
                        ex = None
                        if bx0 <= x < bx0 + bar_r:
                            ex = bx0 + bar_r
                        elif bx1 - bar_r < x <= bx1:
                            ex = bx1 - bar_r
                        if ex is not None and math.hypot(x - ex, y - cy) <= bar_r:
                            col = amber
                    acc[0] += col[0]
                    acc[1] += col[1]
                    acc[2] += col[2]
            n = ss * ss
            row += bytes((round(acc[0] / n), round(acc[1] / n),
                          round(acc[2] / n), 255))
        rows.append(bytes(row))
    return rows


def _icon_pixels(size, ss=4):
    ink = _hex(INK)
    teal = _hex(TEAL)
    amber = _hex(AMBER)
    s = 512.0 / size          # canvas units per pixel
    r_in, r_out = RING_R - RING_W / 2, RING_R + RING_W / 2
    bar_r = BAR_H / 2
    # rounded-rect bar: the capsule between two centres
    cap_x0, cap_x1 = BAR_X0 + bar_r, BAR_X1 - bar_r
    corner = 112.0            # tile corner radius, in canvas units

    rows = []
    for py in range(size):
        row = bytearray()
        for px in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sy in range(ss):
                for sx in range(ss):
                    x = (px + (sx + 0.5) / ss) * s
                    y = (py + (sy + 0.5) / ss) * s
                    # rounded tile mask
                    qx = min(max(x, corner), 512 - corner)
                    qy = min(max(y, corner), 512 - corner)
                    if (x - qx) ** 2 + (y - qy) ** 2 > corner ** 2:
                        continue          # outside the tile: transparent
                    col = ink
                    d = math.hypot(x - MC, y - MC)
                    if r_in <= d <= r_out:
                        col = teal
                    cy_ = abs(y - MC)
                    if cap_x0 <= x <= cap_x1:
                        if cy_ <= bar_r:
                            col = amber
                    elif BAR_X0 <= x <= BAR_X1:
                        ex = cap_x0 if x < cap_x0 else cap_x1
                        if math.hypot(x - ex, y - MC) <= bar_r:
                            col = amber
                    acc[0] += col[0]
                    acc[1] += col[1]
                    acc[2] += col[2]
                    acc[3] += 255
            n = ss * ss
            a = acc[3] / n
            if a <= 0.5:
                row += bytes((0, 0, 0, 0))
            else:
                # un-premultiply so edge pixels keep their colour
                k = acc[3] / 255.0
                row += bytes((round(acc[0] / k), round(acc[1] / k),
                              round(acc[2] / k), round(a)))
        rows.append(bytes(row))
    return rows


def _write_png(path, rows, w, h):
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    png = (b"\x89PNG\r\n\x1a\n"
           + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 9))
           + chunk(b"IEND", b""))
    with open(path, "wb") as f:
        f.write(png)
    print(f"  {path}")


def _png(path, size):
    _write_png(path, _icon_pixels(size), size, size)


def gen_icons():
    os.makedirs("icons", exist_ok=True)
    for size in (16, 32, 180, 512):
        _png(f"icons/icon-{size}.png", size)


def gen_card():
    """1200x630 — the size every link unfurler crops to."""
    w, h = 1200, 630
    _write_png("og.png", _card_pixels(w, h), w, h)


if __name__ == "__main__":
    print("marks:")
    gen_marks()
    print("wordmarks:")
    gen_wordmarks()
    print("lockups:")
    gen_lockups()
    print("banner:")
    gen_banner()
    print("icons:")
    gen_icons()
    print("social card:")
    gen_card()

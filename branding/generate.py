#!/usr/bin/env python3
"""Generate every olang brand asset from one set of design tokens.

The letterforms are hand-drawn monoline geometry (no fonts — SVGs render
identically everywhere), sized to echo the mark: same stroke-to-body ratio,
same round caps as the chevron. Run from branding/:  python3 generate.py
"""

# ── design tokens ──────────────────────────────────────────────────────
TEAL = "#2DD4BF"        # primary on dark
TEAL_DEEP = "#0FA394"   # primary on light (holds contrast on white)
AMBER = "#FBBF24"       # accent on dark
AMBER_DEEP = "#F59E0B"  # accent on light
INK = "#0B1220"         # near-black (light-bg text, dark surfaces)
PAPER = "#E6EDF3"       # off-white (dark-bg text)
SLATE = "#7C93A8"       # secondary text

STROKE = 28             # wordmark monoline weight
XH = 132                # x-height
BASE = 178              # baseline y
TOP = BASE - XH         # 46: top of lowercase
ASC = 8                 # ascender top (l)
DESC = 248              # descender bottom (g)
R = 52                  # bowl radius (o, a, g)
GAP = 26                # inter-letter gap

H = STROKE / 2          # half stroke (caps padding)


def letter_o(x, color):
    cx = x + H + R
    return [f'<circle cx="{cx}" cy="{BASE - H - R}" r="{R}" fill="none" '
            f'stroke="{color}" stroke-width="{STROKE}"/>'], x + STROKE + 2 * R


def letter_l(x, color):
    lx = x + H
    return [f'<path d="M {lx} {ASC} L {lx} {BASE}" fill="none" stroke="{color}" '
            f'stroke-width="{STROKE}" stroke-linecap="round"/>'], x + STROKE


def letter_a(x, color):
    cx = x + H + R
    sx = cx + R
    return [
        f'<circle cx="{cx}" cy="{BASE - H - R}" r="{R}" fill="none" '
        f'stroke="{color}" stroke-width="{STROKE}"/>',
        f'<path d="M {sx} {TOP + H} L {sx} {BASE}" fill="none" stroke="{color}" '
        f'stroke-width="{STROKE}" stroke-linecap="round"/>',
    ], x + STROKE + 2 * R


def letter_n(x, color):
    lx = x + H
    rx = lx + 94
    mid = (lx + rx) / 2
    return [
        f'<path d="M {lx} {BASE} L {lx} {TOP + H}" fill="none" stroke="{color}" '
        f'stroke-width="{STROKE}" stroke-linecap="round"/>',
        f'<path d="M {lx} {TOP + 58} C {lx} {TOP + 14} {mid - 18} {TOP} {mid} {TOP} '
        f'C {mid + 18} {TOP} {rx} {TOP + 14} {rx} {TOP + 58} L {rx} {BASE}" '
        f'fill="none" stroke="{color}" stroke-width="{STROKE}" stroke-linecap="round"/>',
    ], x + STROKE + 94


def letter_g(x, color):
    cx = x + H + R
    sx = cx + R
    hook_end_x = cx - 6
    return [
        f'<circle cx="{cx}" cy="{BASE - H - R}" r="{R}" fill="none" '
        f'stroke="{color}" stroke-width="{STROKE}"/>',
        f'<path d="M {sx} {TOP + H} L {sx} {DESC - 52} '
        f'C {sx} {DESC - 16} {sx - 28} {DESC - 6} {hook_end_x} {DESC - 12}" '
        f'fill="none" stroke="{color}" stroke-width="{STROKE}" stroke-linecap="round"/>',
    ], x + STROKE + 2 * R


LETTERS = {"o": letter_o, "l": letter_l, "a": letter_a, "n": letter_n, "g": letter_g}


def wordmark_parts(text, x, o_color, rest_color):
    """Monoline letterform elements; the leading 'o' takes the brand teal."""
    parts, first = [], True
    for ch in text:
        color = o_color if (ch == "o" and first) else rest_color
        first = False
        elems, x = LETTERS[ch](x, color)
        parts += elems
        x += GAP
    return parts, x - GAP


def mark_parts(cx, cy, scale, teal, amber, ring_w=68, chev_w=58):
    """The o> mark: teal ring, amber chevron in the counter. Stroke widths are
    overridable so the lockup can match the ring to the letter weight."""
    return [
        f'<g transform="translate({cx} {cy}) scale({scale})">'
        f'<circle cx="0" cy="0" r="188" fill="none" stroke="{teal}" stroke-width="{ring_w}"/>'
        f'<path d="M -46 -82 L 62 0 L -46 82" fill="none" stroke="{amber}" '
        f'stroke-width="{chev_w}" stroke-linecap="round" stroke-linejoin="round"/></g>'
    ]


def svg(width, height, body, label):
    inner = "\n  ".join(body)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" '
            f'role="img" aria-label="{label}">\n  {inner}\n</svg>\n')


def write(name, content):
    with open(name, "w") as f:
        f.write(content)
    print(f"  {name}")


# ── mark (icon) ────────────────────────────────────────────────────────
def gen_marks():
    dark = mark_parts(256, 256, 1.0, TEAL, AMBER)
    write("mark.svg", svg(512, 512, dark, "olang mark"))
    light = mark_parts(256, 256, 1.0, TEAL_DEEP, AMBER_DEEP)
    write("mark-light.svg", svg(512, 512, light, "olang mark (light backgrounds)"))
    mono = mark_parts(256, 256, 1.0, "currentColor", "currentColor")
    write("mark-mono.svg", svg(512, 512, mono, "olang mark (monochrome)"))
    # favicon: identical geometry on a rounded dark tile so it reads on any tab
    tile = ['<rect width="512" height="512" rx="96" fill="#0B1220"/>'] + \
        mark_parts(256, 256, 0.82, TEAL, AMBER)
    write("favicon.svg", svg(512, 512, tile, "olang"))


# ── wordmark ───────────────────────────────────────────────────────────
def gen_wordmarks():
    for name, o_color, rest in [
        ("wordmark.svg", TEAL, PAPER),
        ("wordmark-light.svg", TEAL_DEEP, INK),
    ]:
        parts, w = wordmark_parts("olang", 8, o_color, rest)
        write(name, svg(int(w + 8), 262, parts, "olang"))


# ── lockup: mark + wordmark ────────────────────────────────────────────
def gen_lockups():
    # The mark IS the word's leading "o": ring outer edge spans exactly the
    # x-height (46..178), chevron nested inside — the logo reads "olang" with
    # the prompt living in its first letter.
    for name, teal, amber, rest in [
        ("logo.svg", TEAL, AMBER, PAPER),
        ("logo-light.svg", TEAL_DEEP, AMBER_DEEP, INK),
        ("logo-mono.svg", "currentColor", "currentColor", "currentColor"),
    ]:
        scale = XH / 444                      # mark outer diameter -> x-height
        cx, cy = 8 + XH / 2, TOP + XH / 2     # centered on the x-height band
        # Ring weight matches the letter stroke exactly (28 at word scale);
        # the chevron sits a step lighter for hierarchy.
        mark = mark_parts(cx, cy, scale, teal, amber,
                          ring_w=round(STROKE / scale), chev_w=76)
        text, w = wordmark_parts("lang", 8 + XH + GAP, rest, rest)
        body = mark + text
        write(name, svg(int(w + 8), 262, body, "olang"))


# ── README hero banner ─────────────────────────────────────────────────
def gen_banner():
    W, HGT = 1200, 320
    scale = 0.62
    # centered lockup: logo viewBox is ~(w x 262); word width ~ 690
    parts, word_w = wordmark_parts("lang", 8 + XH + GAP, PAPER, PAPER)
    lock_w = (word_w + 8) * scale
    ox = (W - lock_w) / 2
    oy = 62
    mark = mark_parts(ox + (8 + XH / 2) * scale, oy + (TOP + XH / 2) * scale,
                      (XH / 444) * scale, TEAL, AMBER,
                      ring_w=round(STROKE / (XH / 444)), chev_w=76)
    text, _ = wordmark_parts("lang", 8 + XH + GAP, PAPER, PAPER)
    body = [
        f'<rect width="{W}" height="{HGT}" rx="24" fill="{INK}"/>',
        # ghost mark bleeding off the right edge, whisper-quiet
        f'<g opacity="0.05">'
        + mark_parts(W - 30, HGT / 2, 0.62, TEAL, TEAL)[0] + "</g>",
        # faint prompt at top-left: the REPL greeting
        f'<text x="40" y="52" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" '
        f'font-size="17" fill="#3E5468">olang&gt; [1, 2, 3] |&gt; map((x) =&gt; x * x) |&gt; sum</text>',
    ] + mark + [
        f'<g transform="translate({ox} {oy}) scale({scale})">{"".join(text)}</g>',
        f'<text x="{W / 2}" y="262" text-anchor="middle" '
        f'font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" '
        f'font-size="21" letter-spacing="2" fill="{SLATE}">'
        f'pipelines &#183; pattern matching &#183; batteries included &#183; a fast lane</text>',
    ]
    write("banner.svg", svg(W, HGT, body, "olang - pipelines, pattern matching, batteries included"))


if __name__ == "__main__":
    gen_marks()
    gen_wordmarks()
    gen_lockups()
    gen_banner()

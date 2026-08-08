#!/usr/bin/env python3
"""Generate Ollie, the olang otter, in every pose — one set of shared parts
(head, pebble, palette) so the whole family stays consistent. Run from
branding/:  python3 mascot_gen.py

Emits mascot.svg (the floating hero) plus mascot/<pose>.svg variants.
"""
import os

# palette
FUR = "#B45309"
FUR_DARK = "#92400E"
CREAM = "#F3DFB6"
INK = "#0B1220"
TEAL = "#2DD4BF"
TEAL_MID = "#14B8A6"
TEAL_DEEP = "#0FA394"
AMBER = "#FBBF24"


def head(cx, cy, r=58, eyes="closed", smile=True):
    """Ollie's face, front view, centered on (cx, cy)."""
    e = []
    # ears
    e.append(f'<circle cx="{cx - 44}" cy="{cy - 42}" r="14" fill="{FUR_DARK}"/>')
    e.append(f'<circle cx="{cx + 44}" cy="{cy - 42}" r="14" fill="{FUR_DARK}"/>')
    e.append(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{FUR}"/>')
    # muzzle + nose
    e.append(f'<ellipse cx="{cx}" cy="{cy + 20}" rx="34" ry="26" fill="{CREAM}"/>')
    e.append(f'<ellipse cx="{cx}" cy="{cy + 8}" rx="11" ry="8" fill="{INK}"/>')
    # eyes
    if eyes == "closed":
        e.append(f'<path d="M {cx - 30} {cy - 12} Q {cx - 22} {cy - 20} {cx - 14} {cy - 12}" '
                 f'fill="none" stroke="{INK}" stroke-width="6" stroke-linecap="round"/>')
        e.append(f'<path d="M {cx + 14} {cy - 12} Q {cx + 22} {cy - 20} {cx + 30} {cy - 12}" '
                 f'fill="none" stroke="{INK}" stroke-width="6" stroke-linecap="round"/>')
    else:
        e.append(f'<circle cx="{cx - 22}" cy="{cy - 12}" r="7" fill="{INK}"/>')
        e.append(f'<circle cx="{cx + 22}" cy="{cy - 12}" r="7" fill="{INK}"/>')
    if smile:
        e.append(f'<path d="M {cx - 8} {cy + 28} Q {cx} {cy + 34} {cx + 8} {cy + 28}" '
                 f'fill="none" stroke="{INK}" stroke-width="5" stroke-linecap="round"/>')
    # whiskers
    e.append(f'<path d="M {cx - 44} {cy + 14} L {cx - 66} {cy + 10} M {cx - 44} {cy + 22} L {cx - 64} {cy + 24}" '
             f'stroke="{INK}" stroke-width="4" stroke-linecap="round"/>')
    e.append(f'<path d="M {cx + 44} {cy + 14} L {cx + 66} {cy + 10} M {cx + 44} {cy + 22} L {cx + 64} {cy + 24}" '
             f'stroke="{INK}" stroke-width="4" stroke-linecap="round"/>')
    return e


def pebble(cx, cy, s=0.155):
    """The o> stone Ollie carries everywhere."""
    return [
        f'<g transform="translate({cx} {cy}) scale({s})">'
        f'<circle cx="0" cy="0" r="252" fill="{INK}"/>'
        f'<circle cx="0" cy="0" r="168" fill="none" stroke="{TEAL}" stroke-width="76"/>'
        f'<path d="M -42 -74 L 56 0 L -42 74" fill="none" stroke="{AMBER}" stroke-width="64" '
        f'stroke-linecap="round" stroke-linejoin="round"/></g>'
    ]


def arm(x1, y1, x2, y2, w=26):
    return [f'<path d="M {x1} {y1} L {x2} {y2}" fill="none" stroke="{FUR}" '
            f'stroke-width="{w}" stroke-linecap="round"/>',
            f'<circle cx="{x2}" cy="{y2}" r="16" fill="{FUR_DARK}"/>']


def standing_body(cx=256, top=236):
    """Pear-shaped standing body with belly, feet, and side tail."""
    return [
        f'<path d="M {cx + 46} {top + 150} C {cx + 120} {top + 168} {cx + 158} {top + 120} {cx + 150} {top + 78}" '
        f'fill="none" stroke="{FUR_DARK}" stroke-width="30" stroke-linecap="round"/>',
        f'<ellipse cx="{cx}" cy="{top + 96}" rx="84" ry="98" fill="{FUR}"/>',
        f'<ellipse cx="{cx}" cy="{top + 108}" rx="52" ry="64" fill="{CREAM}"/>',
        f'<ellipse cx="{cx - 42}" cy="{top + 190}" rx="27" ry="14" fill="{FUR_DARK}"/>',
        f'<ellipse cx="{cx + 42}" cy="{top + 190}" rx="27" ry="14" fill="{FUR_DARK}"/>',
    ]


def svg(body, label, w=512, h=512):
    inner = "\n  ".join(body)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" '
            f'role="img" aria-label="{label}">\n  {inner}\n</svg>\n')


def write(path, content):
    with open(path, "w") as f:
        f.write(content)
    print(f"  {path}")


# ── poses ──────────────────────────────────────────────────────────────

def floating():
    """The hero: Ollie on its back in the water, pebble on its chest."""
    return [
        f'<rect x="0" y="330" width="512" height="182" fill="{TEAL_DEEP}"/>',
        f'<path d="M 396 344 C 444 340 472 318 484 288" fill="none" stroke="{FUR_DARK}" '
        f'stroke-width="32" stroke-linecap="round"/>',
        f'<ellipse cx="268" cy="312" rx="138" ry="66" fill="{FUR}"/>',
        f'<ellipse cx="282" cy="296" rx="92" ry="42" fill="{CREAM}"/>',
        f'<ellipse cx="374" cy="256" rx="17" ry="27" transform="rotate(18 374 256)" fill="{FUR_DARK}"/>',
        f'<ellipse cx="344" cy="250" rx="16" ry="25" transform="rotate(8 344 250)" fill="{FUR}"/>',
    ] + head(148, 248) + [
        f'<circle cx="228" cy="242" r="17" fill="{FUR_DARK}"/>',
        f'<circle cx="310" cy="242" r="17" fill="{FUR_DARK}"/>',
    ] + pebble(269, 216) + [
        f'<path d="M 0 358 C 60 340 110 372 170 358 C 230 344 280 376 340 360 C 400 344 456 372 512 356 '
        f'L 512 512 L 0 512 Z" fill="{TEAL_MID}" opacity="0.85"/>',
        f'<path d="M 36 388 C 66 380 92 394 122 388 M 300 402 C 330 394 356 408 386 402" fill="none" '
        f'stroke="{TEAL}" stroke-width="8" stroke-linecap="round" opacity="0.6"/>',
    ]


def waving():
    """Standing, one paw raised in greeting, pebble tucked in the other."""
    return standing_body() + head(256, 168, eyes="open") + \
        arm(196, 300, 128, 216) + arm(316, 300, 356, 352) + \
        pebble(376, 368, 0.12)


def juggling():
    """Three pebbles in the air — concurrency has never been fluffier."""
    return standing_body() + head(256, 178, eyes="open") + \
        arm(192, 306, 148, 238) + arm(320, 306, 364, 238) + \
        pebble(150, 130, 0.11) + pebble(256, 72, 0.11) + pebble(362, 130, 0.11)


def coding():
    """At the laptop; the screen shows the prompt."""
    return standing_body(top=210) + head(256, 148, eyes="open", smile=True) + [
        # laptop: screen leaning toward the viewer + base
        f'<rect x="146" y="330" width="220" height="132" rx="12" fill="{INK}"/>',
        f'<rect x="160" y="344" width="192" height="104" rx="6" fill="#122032"/>',
        f'<text x="182" y="384" font-family="ui-monospace, Menlo, monospace" font-size="21" '
        f'fill="{TEAL}">olang&gt;</text>',
        f'<rect x="278" y="364" width="12" height="22" fill="{AMBER}"/>',
        f'<rect x="126" y="458" width="260" height="20" rx="10" fill="#1E2D42"/>',
    ] + arm(186, 320, 172, 452, 24)[:1] + arm(326, 320, 340, 452, 24)[:1]


def surfing():
    """Riding the amber chevron across the water — the fast lane."""
    return [
        f'<rect x="0" y="360" width="512" height="152" fill="{TEAL_DEEP}"/>',
        # speed lines
        f'<path d="M 40 300 L 130 300 M 20 254 L 96 254 M 60 344 L 150 344" stroke="{TEAL}" '
        f'stroke-width="10" stroke-linecap="round" opacity="0.55"/>',
        # the chevron as surfboard: a big legible ">" skimming the crest
        f'<g transform="translate(258 366) rotate(-10)">'
        f'<path d="M -120 -60 L 44 0 L -120 60" fill="none" stroke="{AMBER}" stroke-width="44" '
        f'stroke-linecap="round" stroke-linejoin="round"/></g>',
        # crouched body above the board
        f'<ellipse cx="252" cy="250" rx="82" ry="68" fill="{FUR}"/>',
        f'<ellipse cx="252" cy="266" rx="50" ry="40" fill="{CREAM}"/>',
        f'<ellipse cx="204" cy="312" rx="23" ry="13" transform="rotate(-14 204 312)" fill="{FUR_DARK}"/>',
        f'<ellipse cx="298" cy="308" rx="23" ry="13" transform="rotate(-8 298 308)" fill="{FUR_DARK}"/>',
    ] + head(244, 166, eyes="open") + arm(186, 234, 126, 192) + arm(318, 234, 378, 198) + [
        # spray
        f'<path d="M 150 372 C 176 354 204 380 232 366 M 330 366 C 360 350 390 376 418 362" fill="none" '
        f'stroke="{TEAL}" stroke-width="9" stroke-linecap="round" opacity="0.7"/>',
    ]


def reading():
    """Nose in the book (the olang book, naturally). Open, attentive eyes."""
    return standing_body() + head(256, 168, eyes="open", smile=False) + [
        # open book held in both paws
        f'<path d="M 152 330 L 252 310 L 252 428 L 152 448 Z" fill="{CREAM}"/>',
        f'<path d="M 360 330 L 260 310 L 260 428 L 360 448 Z" fill="{CREAM}"/>',
        f'<path d="M 152 330 L 152 448 M 360 330 L 360 448" stroke="{TEAL_DEEP}" stroke-width="10" '
        f'stroke-linecap="round"/>',
        f'<path d="M 252 310 L 252 428 M 260 310 L 260 428" stroke="{FUR_DARK}" stroke-width="4"/>',
        # text lines
        f'<path d="M 172 348 L 236 336 M 172 370 L 236 358 M 172 392 L 236 380 '
        f'M 276 336 L 340 348 M 276 358 L 340 370 M 276 380 L 340 392" '
        f'stroke="{TEAL_DEEP}" stroke-width="6" stroke-linecap="round" opacity="0.5"/>',
        f'<circle cx="160" cy="336" r="15" fill="{FUR_DARK}"/>',
        f'<circle cx="352" cy="336" r="15" fill="{FUR_DARK}"/>',
    ]


def badge():
    """Head-only avatar on the dark tile — for stickers and profile images.
    The standard head is scaled up as a unit so every feature stays put."""
    inner = "".join(head(0, 0, eyes="closed"))
    return [
        f'<rect width="512" height="512" rx="256" fill="{INK}"/>',
        f'<circle cx="256" cy="256" r="228" fill="none" stroke="{TEAL}" stroke-width="18"/>',
        f'<g transform="translate(256 240) scale(1.85)">{inner}</g>',
    ] + pebble(256, 434, 0.14)


POSES = {
    "waving": (waving, "Ollie the olang otter, waving hello"),
    "juggling": (juggling, "Ollie juggling three o> pebbles"),
    "coding": (coding, "Ollie at a laptop with the olang prompt"),
    "surfing": (surfing, "Ollie surfing the chevron"),
    "reading": (reading, "Ollie reading the olang book"),
    "badge": (badge, "Ollie badge avatar"),
}

if __name__ == "__main__":
    write("mascot.svg", svg(floating(), "Ollie, the olang otter, floating with the o> pebble"))
    os.makedirs("mascot", exist_ok=True)
    for name, (fn, label) in POSES.items():
        write(f"mascot/{name}.svg", svg(fn(), label))

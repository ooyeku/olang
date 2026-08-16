#!/usr/bin/env python3
"""Generate Ollie, the olang otter — drawn with realistic sea-otter anatomy
in a restrained flat style: profile head with a true muzzle, pale head over
a dark body (as sea otters have), small open eye, long streamlined
proportions. The only brand colors are the water and the o> stone.

Run from branding/:  python3 mascot_gen.py
Emits mascot.svg (floating, the hero) plus mascot/<pose>.svg variants.
"""
import os

# otter palette — muted, naturalistic
BODY = "#6B4423"        # dark brown body
BODY_DK = "#523318"     # shadow / limbs / tail underside
HEAD = "#C9A876"        # pale head and throat, the sea-otter field mark
MUZZLE = "#B8936A"      # slightly deeper than the head
INK = "#0B1220"         # eye, nose, whiskers
# brand
TEAL = "#2DD4BF"
TEAL_MID = "#14B8A6"
TEAL_DEEP = "#0FA394"
AMBER = "#FBBF24"


def stone(cx, cy, s=0.14):
    """The stone Ollie carries: the mark itself, on its own ink disc.

    Geometry mirrors generate.py's mark exactly — ring at r=168 w=68, the
    through-line 52 tall overhanging to x=+-222 — so the otter is never
    holding a logo the project has stopped using.
    """
    return [
        f'<g transform="translate({cx} {cy}) scale({s})">'
        f'<circle cx="0" cy="0" r="252" fill="{INK}"/>'
        f'<circle cx="0" cy="0" r="168" fill="none" stroke="{TEAL}" stroke-width="68"/>'
        f'<rect x="-222" y="-26" width="444" height="52" rx="26" fill="{AMBER}"/></g>'
    ]


def profile_head(cx, cy, s=1.0, facing=1, tilt=0):
    """A sea-otter head in profile: rounded skull, defined muzzle, small ear,
    small open eye, nose, whiskers. `facing` 1 = right, -1 = left."""
    return [
        f'<g transform="translate({cx} {cy}) rotate({tilt}) scale({facing * s} {s})">'
        # throat: pale wedge running down-back, seating the head on the body
        f'<path d="M -30 8 C -34 26 -28 44 -12 54 C 4 62 20 58 28 46 C 18 34 6 24 -2 14 Z" fill="{HEAD}"/>'
        # skull (pale)
        f'<circle cx="0" cy="0" r="34" fill="{HEAD}"/>'
        # muzzle: blunt wedge forward
        f'<path d="M 8 -12 C 30 -14 46 -4 50 6 C 52 12 48 18 40 19 C 24 22 8 18 2 12 Z" fill="{MUZZLE}"/>'
        # ear: small, set back and low (true otter placement)
        f'<circle cx="-24" cy="-18" r="6.5" fill="{BODY_DK}"/>'
        # nose
        f'<path d="M 44 2 C 50 2 52 6 50 10 C 48 13 42 13 40 9 C 39 6 40 3 44 2 Z" fill="{INK}"/>'
        # eye: small, open, attentive
        f'<circle cx="16" cy="-8" r="4.2" fill="{INK}"/>'
        # mouth: a short neutral line under the muzzle
        f'<path d="M 40 16 C 34 20 26 20 20 17" fill="none" stroke="{INK}" stroke-width="2.4" stroke-linecap="round"/>'
        # whiskers: three fine lines off the muzzle
        f'<path d="M 34 8 L 58 4 M 34 11 L 58 12 M 33 14 L 55 20" fill="none" stroke="{INK}" '
        f'stroke-width="1.6" stroke-linecap="round" opacity="0.85"/>'
        f'</g>'
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
    """The hero: a sea otter on its back — long low body, head in profile,
    hind flippers raised, the stone held on the chest."""
    return [
        # deep water
        f'<rect x="0" y="322" width="512" height="190" fill="{TEAL_DEEP}"/>',
        # tail: thick at the hip, tapering into the water
        f'<path d="M 402 318 C 442 314 470 322 488 344 L 468 356 C 448 340 424 334 398 338 Z" fill="{BODY_DK}"/>',
        # body: long, low floating mass
        f'<path d="M 120 300 C 150 268 220 258 288 264 C 348 269 396 284 408 306 '
        f'C 414 322 402 338 376 344 C 300 360 180 358 132 338 C 110 328 108 314 120 300 Z" fill="{BODY}"/>',
        # chest/throat: pale wash flowing from the head
        f'<path d="M 138 296 C 160 276 208 268 252 272 C 268 274 276 284 270 296 '
        f'C 250 316 180 320 150 312 C 136 308 132 302 138 296 Z" fill="{HEAD}" opacity="0.35"/>',
        # hind flippers: webbed paddles angled up out of the water
        f'<path d="M 384 300 C 398 272 414 258 434 252 C 438 266 432 288 416 306 Z" fill="{BODY_DK}"/>',
        f'<path d="M 356 300 C 364 276 376 262 392 256 C 396 270 390 290 376 306 Z" fill="{BODY}"/>',
        # forepaws holding the stone on the chest
        f'<circle cx="238" cy="266" r="12" fill="{BODY_DK}"/>',
        f'<circle cx="292" cy="266" r="12" fill="{BODY_DK}"/>',
    ] + stone(265, 248, 0.13) + profile_head(158, 252, 1.06, facing=1, tilt=-4) + [
        # waterline lapping the body
        f'<path d="M 0 350 C 70 336 130 362 200 350 C 270 338 330 364 400 350 C 452 340 490 356 512 348 '
        f'L 512 512 L 0 512 Z" fill="{TEAL_MID}" opacity="0.85"/>',
        f'<path d="M 44 384 C 78 376 106 390 140 384 M 318 396 C 352 388 380 402 414 396" fill="none" '
        f'stroke="{TEAL}" stroke-width="7" stroke-linecap="round" opacity="0.5"/>',
    ]


def swimming():
    """Streamlined surface swim: body in a long arc, tail driving, wake behind."""
    return [
        f'<rect x="0" y="330" width="512" height="182" fill="{TEAL_DEEP}"/>',
        # wake
        f'<path d="M 60 342 C 100 332 140 348 180 340 M 30 366 C 66 358 98 370 134 364" fill="none" '
        f'stroke="{TEAL}" stroke-width="7" stroke-linecap="round" opacity="0.5"/>',
        # tail: driving stroke, half submerged
        f'<path d="M 120 330 C 92 320 70 300 64 274 L 86 268 C 96 292 114 308 140 316 Z" fill="{BODY_DK}"/>',
        # body: horizontal arc, chest forward
        f'<path d="M 130 322 C 160 292 250 278 330 286 C 388 292 428 306 440 322 '
        f'C 446 334 436 344 412 348 C 320 360 190 358 148 344 C 126 338 120 330 130 322 Z" fill="{BODY}"/>',
        # foreleg tucked, hinting motion
        f'<path d="M 336 330 C 348 340 352 352 348 362 L 328 358 C 328 348 330 338 336 330 Z" fill="{BODY_DK}"/>',
    ] + profile_head(446, 300, 1.0, facing=1, tilt=-8) + [
        # bow wave at the chest
        f'<path d="M 470 336 C 490 330 504 336 512 344" fill="none" stroke="{TEAL}" '
        f'stroke-width="7" stroke-linecap="round" opacity="0.6"/>',
        f'<path d="M 0 356 C 80 346 150 364 230 354 C 320 344 400 366 512 352 L 512 512 L 0 512 Z" '
        f'fill="{TEAL_MID}" opacity="0.85"/>',
    ]


def standing():
    """Upright and alert — the heraldic pose. Tail grounds the figure; the
    stone is held close at the chest."""
    return [
        # ground line
        f'<path d="M 96 448 L 416 448" stroke="{TEAL_DEEP}" stroke-width="6" stroke-linecap="round" opacity="0.5"/>',
        # tail: thick, resting on the ground behind
        f'<path d="M 282 430 C 330 436 368 428 392 404 L 376 388 C 352 406 322 412 288 408 Z" fill="{BODY_DK}"/>',
        # body: tall tapered column, chest lifted
        f'<path d="M 218 210 C 258 202 292 218 302 258 C 314 310 312 380 296 424 '
        f'C 288 444 268 452 244 450 C 216 448 198 434 194 408 C 188 350 190 274 202 234 C 206 220 210 212 218 210 Z" '
        f'fill="{BODY}"/>',
        # throat/chest: pale front
        f'<path d="M 232 232 C 252 226 268 238 272 262 C 278 300 276 348 266 380 '
        f'C 260 396 244 400 232 392 C 220 382 216 344 218 300 C 219 270 222 244 232 232 Z" fill="{HEAD}" opacity="0.35"/>',
        # hind feet: webbed, flat on the ground
        f'<path d="M 208 446 C 196 446 186 440 184 432 L 236 432 C 234 442 224 446 208 446 Z" fill="{BODY_DK}"/>',
        f'<path d="M 276 446 C 264 446 254 440 252 432 L 304 432 C 302 442 292 446 276 446 Z" fill="{BODY_DK}"/>',
        # forepaws holding the stone at the chest
        f'<circle cx="228" cy="296" r="11" fill="{BODY_DK}"/>',
        f'<circle cx="274" cy="296" r="11" fill="{BODY_DK}"/>',
    ] + stone(251, 282, 0.12) + profile_head(258, 182, 1.05, facing=1, tilt=-2)


def lookout():
    """The spyhop: head and shoulders periscoped above the waterline,
    scanning — alert, composed."""
    return [
        f'<rect x="0" y="332" width="512" height="180" fill="{TEAL_DEEP}"/>',
        # shoulders breaking the surface
        f'<path d="M 178 358 C 190 322 232 300 274 304 C 312 308 340 328 348 356 '
        f'C 320 372 240 374 202 366 C 188 363 180 360 178 358 Z" fill="{BODY}"/>',
        # pale chest above the water
        f'<path d="M 232 330 C 250 318 276 320 288 334 C 294 344 290 354 278 358 '
        f'C 258 362 238 358 230 348 C 226 342 227 335 232 330 Z" fill="{HEAD}" opacity="0.35"/>',
    ] + profile_head(262, 282, 1.1, facing=1, tilt=-6) + [
        # ripples radiating from the body
        f'<path d="M 120 372 C 160 362 200 376 240 368 M 300 374 C 340 364 380 378 420 370" fill="none" '
        f'stroke="{TEAL}" stroke-width="7" stroke-linecap="round" opacity="0.5"/>',
        f'<path d="M 0 360 C 80 350 160 368 250 358 C 340 348 420 368 512 356 L 512 512 L 0 512 Z" '
        f'fill="{TEAL_MID}" opacity="0.85"/>',
    ]


def inspecting():
    """Head bowed over the stone, turning it in both paws — for
    documentation and tooling contexts."""
    return [
        f'<path d="M 116 448 L 396 448" stroke="{TEAL_DEEP}" stroke-width="6" stroke-linecap="round" opacity="0.5"/>',
        # tail curled around the seated body
        f'<path d="M 300 428 C 352 430 388 414 402 384 L 384 370 C 370 394 340 406 304 406 Z" fill="{BODY_DK}"/>',
        # seated body: rounded, hunched forward
        f'<path d="M 196 260 C 232 226 296 228 322 270 C 344 306 344 372 322 410 '
        f'C 306 436 260 444 224 432 C 192 420 176 392 176 352 C 176 316 182 280 196 260 Z" fill="{BODY}"/>',
        # pale chest
        f'<path d="M 224 286 C 248 272 278 280 288 306 C 296 330 294 366 282 390 '
        f'C 272 406 248 408 234 396 C 220 382 214 344 216 316 C 217 302 218 292 224 286 Z" fill="{HEAD}" opacity="0.35"/>',
        # hind feet forward
        f'<path d="M 210 444 C 198 444 190 438 188 430 L 238 430 C 236 440 226 444 210 444 Z" fill="{BODY_DK}"/>',
        f'<path d="M 282 444 C 270 444 262 438 260 430 L 310 430 C 308 440 298 444 282 444 Z" fill="{BODY_DK}"/>',
        # paws turning the stone
        f'<circle cx="232" cy="352" r="11" fill="{BODY_DK}"/>',
        f'<circle cx="278" cy="352" r="11" fill="{BODY_DK}"/>',
    ] + stone(255, 340, 0.125) + profile_head(262, 240, 1.0, facing=1, tilt=26)


def badge():
    """Profile head within the brand ring — avatars and small placements."""
    return [
        f'<rect width="512" height="512" rx="256" fill="{INK}"/>',
        f'<circle cx="256" cy="256" r="226" fill="none" stroke="{TEAL}" stroke-width="16"/>',
        # neck/shoulder base so the head doesn't float
        f'<path d="M 130 388 C 160 340 220 316 288 322 C 344 328 386 352 398 388 '
        f'C 360 428 300 448 240 444 C 190 440 152 420 130 388 Z" fill="{BODY}"/>',
    ] + profile_head(250, 264, 2.1, facing=1, tilt=-2) + stone(256, 430, 0.1)


POSES = {
    "swimming": (swimming, "Ollie the olang otter swimming"),
    "standing": (standing, "Ollie standing with the o> stone"),
    "lookout": (lookout, "Ollie at lookout above the waterline"),
    "inspecting": (inspecting, "Ollie examining the o> stone"),
    "badge": (badge, "Ollie profile badge"),
}

if __name__ == "__main__":
    write("mascot.svg", svg(floating(), "Ollie, the olang otter, floating with the o> stone"))
    os.makedirs("mascot", exist_ok=True)
    # drop poses from the earlier cartoon set
    for stale in ("waving.svg", "juggling.svg", "coding.svg", "surfing.svg", "reading.svg", "diving.svg"):
        p = os.path.join("mascot", stale)
        if os.path.exists(p):
            os.remove(p)
            print(f"  removed mascot/{stale}")
    for name, (fn, label) in POSES.items():
        write(f"mascot/{name}.svg", svg(fn(), label))

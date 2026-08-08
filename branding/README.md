# olang brand

The visual identity for olang, used by the README and the website. Every
asset is hand-authored SVG generated from two small scripts — no fonts, no
external tools — so everything renders identically everywhere and can be
regenerated from one set of tokens.

## The mark

The mark is the REPL prompt distilled: the lowercase **o** as a bold teal
ring, with the amber **>** — the prompt, the pipeline arrowhead — nested in
its counter. It reads "o>" at every size, and in the logo it *is* the
word's first letter.

## Palette

| Token | Hex | Use |
|---|---|---|
| Teal | `#2DD4BF` | primary, on dark backgrounds |
| Deep teal | `#0FA394` | primary, on light backgrounds |
| Amber | `#FBBF24` | accent, on dark |
| Deep amber | `#F59E0B` | accent, on light |
| Ink | `#0B1220` | dark surfaces, light-bg text |
| Paper | `#E6EDF3` | dark-bg text |
| Slate | `#7C93A8` | secondary text |

## Assets

| File | What it is |
|---|---|
| `logo.svg` / `logo-light.svg` / `logo-mono.svg` | the full logo — the mark as the word's leading "o" + "lang" (dark / light / single-color) |
| `wordmark.svg` / `wordmark-light.svg` | plain-text "olang" for quiet contexts |
| `mark.svg` / `mark-light.svg` / `mark-mono.svg` | the o> mark alone |
| `favicon.svg`, `icons/icon-{16,32,180,512}.png` | the mark on a rounded ink tile |
| `banner.svg` | the README hero (1200×320, self-contained dark panel) |
| `mascot.svg` | **Ollie**, the olang otter — floating with the o> pebble |
| `mascot/*.svg` | Ollie's pose family: `waving`, `juggling`, `coding`, `surfing`, `reading`, `badge` |

## Ollie

Ollie is an otter: otters float on their backs, carry a favorite stone
everywhere (batteries included), and are relentlessly practical. Ollie's
stone is the o> mark. Use the floating hero for splashes, `badge` for
avatars, and the pose family wherever a page needs warmth — one Ollie per
page is plenty.

## Usage rules

- On dark backgrounds use `logo.svg`; on light, `logo-light.svg`; when it
  must be one color, `logo-mono.svg` (it inherits `currentColor`).
- Don't recolor, outline, rotate, or add effects to the mark.
- Keep clear space around the logo of at least the ring's stroke width.
- The banner is a self-contained dark panel — it sits on any page background.

## Regenerating

```bash
cd branding
python3 generate.py      # mark, wordmark, logo, favicon, banner
python3 mascot_gen.py    # Ollie and the pose family
resvg -w 512 -h 512 favicon.svg icons/icon-512.png   # PNG exports
```

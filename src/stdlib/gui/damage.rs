//! Damage: which parts of a window a frame changed, so a renderer draws
//! only those over the last frame it kept, and presents nothing when no
//! pixel changed.
//!
//! The window is cut into tiles. Each frame, every primitive gives its
//! bounds (every pixel it can touch, antialiased edge and all, cut by its
//! clip) and a hash of everything that decides its pixels. A tile's hash
//! is the ordered hashes of the primitives that touch it (and the clear
//! colour): a tile's pixels are a function of exactly those, in that
//! order, so a tile whose hash is last frame's is last frame's pixels and
//! needs no drawing. That holds whatever changed the scene — a patch, a
//! hover or pressed look, a caret's blink, a scroll, an animation's
//! frame — without any of them saying what they dirtied.
//!
//! The dirty tiles become a few disjoint rectangles; a renderer redraws
//! each, scissored, with the primitives that touch it. A primitive's
//! pixels never depend on the scissor, so a damaged frame is the full
//! frame, pixel for pixel (`tests/gui_damage_test.rs` holds both
//! renderers to that). `OLANG_GUI_FULL_REDRAW=1` draws every frame whole.

use super::window::Clip;
use std::sync::atomic::{AtomicU64, Ordering};

/// How far along each edge a continuous corner of radius `r` reaches:
/// `CONTINUOUS_EXTENT · r` (the renderers' curve; soft.rs, gpu.rs).
pub const CONTINUOUS_EXTENT: f32 = 1.528;

/// A tile's side, in device pixels.
pub const TILE: i32 = 64;

/// A rectangle of whole device pixels, `x0 ≤ x < x1`, `y0 ≤ y < y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IRect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl IRect {
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> IRect {
        IRect { x0, y0, x1, y1 }
    }
    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }
    pub fn intersects(&self, o: &IRect) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
    pub fn intersect(&self, o: &IRect) -> IRect {
        IRect {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }
    pub fn area(&self) -> i64 {
        if self.is_empty() {
            0
        } else {
            (self.x1 - self.x0) as i64 * (self.y1 - self.y0) as i64
        }
    }
}

/// What a frame must draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Damage {
    /// Nothing changed: draw and present nothing.
    None,
    /// Everything: the first frame, a new size, or a forced redraw.
    Full,
    /// These disjoint rectangles, inside the window.
    Rects(Vec<IRect>),
}

impl Damage {
    /// The pixels it covers in a `w`×`h` window.
    pub fn area(&self, w: u32, h: u32) -> i64 {
        match self {
            Damage::None => 0,
            Damage::Full => w as i64 * h as i64,
            Damage::Rects(r) => r.iter().map(|r| r.area()).sum(),
        }
    }
}

/// Every frame drawn whole (the reference damage is checked against).
pub fn full_redraw_forced() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| {
        std::env::var("OLANG_GUI_FULL_REDRAW")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false)
    })
}

/// The pixels a primitive can touch: its box grown by `pad` (an
/// antialiased edge), cut by its clip and the window, rounded out to
/// whole pixels. `None` when it touches nothing.
// A primitive's box, its pad and clip, and the window: every renderer's
// primitive calls it with these loose, so they stay arguments.
#[allow(clippy::too_many_arguments)]
pub fn bounds(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    pad: f32,
    clip: &Clip,
    ww: u32,
    wh: u32,
) -> Option<IRect> {
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
        // the renderers draw nothing sensible for these; damage all
        return Some(IRect::new(0, 0, ww as i32, wh as i32));
    }
    let x0 = (x - pad).max(clip.x0).max(0.0);
    let y0 = (y - pad).max(clip.y0).max(0.0);
    let x1 = (x + w + pad).min(clip.x1).min(ww as f32);
    let y1 = (y + h + pad).min(clip.y1).min(wh as f32);
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    // a pixel is drawn when its centre is inside; whole pixels around
    // every one that could be
    let r = IRect::new(
        x0.floor() as i32 - 1,
        y0.floor() as i32 - 1,
        x1.ceil() as i32 + 1,
        y1.ceil() as i32 + 1,
    );
    let r = r.intersect(&IRect::new(0, 0, ww as i32, wh as i32));
    (!r.is_empty()).then_some(r)
}

/// A fast, order-sensitive mix (FxHash's step).
#[inline]
pub fn mix(h: u64, v: u64) -> u64 {
    (h.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95)
}

/// Hash a slice of `f32`s by their bits (with `mix`).
#[inline]
pub fn mix_f32s(mut h: u64, vs: &[f32]) -> u64 {
    for v in vs {
        h = mix(h, v.to_bits() as u64);
    }
    h
}

/// What damage reads of a primitive: the pixels it can touch, the hash
/// of everything that decides them, and the same split for moving it:
/// `base` hashes all but its place (`x`, `y`) and its `clip`.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub bounds: IRect,
    pub hash: u64,
    pub base: u64,
    pub x: f32,
    pub y: f32,
    pub clip: Clip,
    /// Where it paints one colour, the same wherever it is (a fill's
    /// inside, clear of its border and corners): seen from inside it, a
    /// moved fill is the same fill. Empty when it paints none.
    pub interior: IRect,
    /// Its pixels are not the same wherever it is: it is never taken as
    /// moved (the software renderer cuts a path at the window's edges, and
    /// a shape cut there is antialiased otherwise than one inside).
    pub pinned: bool,
}

/// Whether a box grown by `pad` passes a `ww`×`wh` window's edges.
pub fn crosses_window(x: f32, y: f32, w: f32, h: f32, pad: f32, ww: u32, wh: u32) -> bool {
    !(x - pad >= 0.0 && y - pad >= 0.0 && x + w + pad <= ww as f32 && y + h + pad <= wh as f32)
}

/// The pixels inside a box `[x, y, w, h]` (device pixels) at least
/// `inset` from its edges.
pub fn inside(x: f32, y: f32, w: f32, h: f32, inset: f32) -> IRect {
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
        return IRect::new(0, 0, 0, 0);
    }
    IRect::new(
        (x + inset).ceil() as i32,
        (y + inset).ceil() as i32,
        (x + w - inset).floor() as i32,
        (y + h - inset).floor() as i32,
    )
}

impl Item {
    /// An item whose hash is its base, place and clip.
    pub fn new(bounds: IRect, base: u64, x: f32, y: f32, clip: Clip) -> Item {
        let hash = mix_f32s(
            mix_f32s(base, &[x, y, clip.x0, clip.y0, clip.x1, clip.y1]),
            &clip.radii,
        );
        Item {
            bounds,
            hash,
            base,
            x,
            y,
            clip,
            interior: IRect::new(0, 0, 0, 0),
            pinned: false,
        }
    }

    /// Never taken as moved (see `pinned`).
    pub fn pinned(mut self, on: bool) -> Item {
        self.pinned = on;
        self
    }

    /// With the pixels it paints one colour.
    pub fn with_interior(mut self, r: IRect) -> Item {
        self.interior = r;
        self
    }

    /// The hash of what it draws inside `r`, as seen from `r`'s corner:
    /// the same for the same drawing moved by whole pixels. A clip is cut
    /// to `r` (a plain one decides a pixel by its centre alone; a rounded
    /// one only when `r` is a pixel clear of its corners and edges, else
    /// it is taken whole).
    fn seen_from(&self, r: &IRect) -> u64 {
        let (ox, oy) = (r.x0 as f32, r.y0 as f32);
        if self.pinned {
            return mix_f32s(self.hash, &[ox, oy]);
        }
        let (rx0, ry0, rx1, ry1) = (ox, oy, r.x1 as f32, r.y1 as f32);
        let c = &self.clip;
        let cut = if !c.is_rounded() {
            Some([c.x0.max(rx0), c.y0.max(ry0), c.x1.min(rx1), c.y1.min(ry1)])
        } else {
            let k = if c.continuous { CONTINUOUS_EXTENT } else { 1.0 };
            let m = c.radii.iter().fold(0.0f32, |a, v| a.max(*v)) * k + 1.0;
            let clear =
                rx0 >= c.x0 + m && rx1 <= c.x1 - m && ry0 >= c.y0 + 1.0 && ry1 <= c.y1 - 1.0
                    || ry0 >= c.y0 + m && ry1 <= c.y1 - m && rx0 >= c.x0 + 1.0 && rx1 <= c.x1 - 1.0;
            clear.then_some([rx0, ry0, rx1, ry1])
        };
        // one colour all over `r` (its clip too): where it is is no matter
        if let Some([x0, y0, x1, y1]) = cut
            && x0 <= rx0
            && y0 <= ry0
            && x1 >= rx1
            && y1 >= ry1
            && self.interior.x0 <= r.x0
            && self.interior.y0 <= r.y0
            && self.interior.x1 >= r.x1
            && self.interior.y1 >= r.y1
        {
            return mix(self.base, 2);
        }
        // places relative to `r`, exactly (in f64: the same only for the
        // same place moved by whole pixels)
        let rel = |h: u64, vs: &[f32]| -> u64 {
            let mut h = h;
            for (i, v) in vs.iter().enumerate() {
                let o = if i % 2 == 0 { r.x0 } else { r.y0 } as f64;
                h = mix(h, (*v as f64 - o).to_bits());
            }
            h
        };
        let h = rel(self.base, &[self.x, self.y]);
        match cut {
            Some([x0, y0, x1, y1]) => {
                // empty cuts are all alike
                if x1 <= x0 || y1 <= y0 {
                    mix(h, 1)
                } else {
                    rel(h, &[x0, y0, x1, y1])
                }
            }
            None => mix(
                mix_f32s(rel(h, &[c.x0, c.y0, c.x1, c.y1]), &c.radii),
                c.continuous as u64,
            ),
        }
    }
}

/// A scrolled region as a frame drew it: what it shows moved by its
/// offset (device pixels), seen through `view`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scroll {
    pub id: u64,
    pub view: Clip,
    pub dx: f32,
    pub dy: f32,
}

/// Pixels the kept frame moves before a frame draws: `src` copied to
/// `dst` (the same size), a scrolled region's content following it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shift {
    pub src: IRect,
    pub dst: IRect,
}

/// A window's tiles from the last frame drawn.
#[derive(Default)]
pub struct Tracker {
    last: Option<Next>,
    /// The last frame was drawn and its tiles are known.
    valid: bool,
}

/// A frame's tiles, kept by [`Tracker::commit`] once drawn: each tile's
/// hash, the items, which items touch each tile, and its scrolls.
pub struct Next {
    size: (u32, u32),
    cols: i32,
    rows: i32,
    seed: u64,
    tiles: Vec<u64>,
    items: Vec<Item>,
    /// Items by tile, `at[t]..at[t + 1]` in `of`: made only when a
    /// scroll asks (most frames do not move anything).
    buckets: std::cell::OnceCell<(Vec<u32>, Vec<u32>)>,
    scrolls: Vec<Scroll>,
}

impl Next {
    fn tile_range(&self, r: &IRect) -> Option<(i32, i32, i32, i32)> {
        let c0 = (r.x0 / TILE).max(0);
        let c1 = ((r.x1 - 1) / TILE).min(self.cols - 1);
        let r0 = (r.y0 / TILE).max(0);
        let r1 = ((r.y1 - 1) / TILE).min(self.rows - 1);
        (c0 <= c1 && r0 <= r1 && !r.is_empty()).then_some((c0, c1, r0, r1))
    }

    /// The items by tile.
    fn buckets(&self) -> &(Vec<u32>, Vec<u32>) {
        self.buckets.get_or_init(|| {
            let n = self.tiles.len();
            let mut at = vec![0u32; n + 1];
            for it in &self.items {
                if let Some((c0, c1, r0, r1)) = self.tile_range(&it.bounds) {
                    for row in r0..=r1 {
                        let base = (row * self.cols) as usize;
                        for t in base + c0 as usize..=base + c1 as usize {
                            at[t + 1] += 1;
                        }
                    }
                }
            }
            for t in 0..n {
                at[t + 1] += at[t];
            }
            let mut of = vec![0u32; at[n] as usize];
            let mut fill = at.clone();
            for (i, it) in self.items.iter().enumerate() {
                if let Some((c0, c1, r0, r1)) = self.tile_range(&it.bounds) {
                    for row in r0..=r1 {
                        let base = (row * self.cols) as usize;
                        for t in base + c0 as usize..=base + c1 as usize {
                            of[fill[t] as usize] = i as u32;
                            fill[t] += 1;
                        }
                    }
                }
            }
            (at, of)
        })
    }

    /// The hash of what this frame draws in `r`, seen from its corner.
    fn seen(&self, r: &IRect) -> u64 {
        let Some((c0, c1, r0, r1)) = self.tile_range(r) else {
            return self.seed;
        };
        let (at, of) = self.buckets();
        let mut idx: Vec<u32> = Vec::new();
        for row in r0..=r1 {
            for col in c0..=c1 {
                let t = (row * self.cols + col) as usize;
                idx.extend_from_slice(&of[at[t] as usize..at[t + 1] as usize]);
            }
        }
        if (c0, r0) != (c1, r1) {
            idx.sort_unstable();
            idx.dedup();
        }
        let mut h = self.seed;
        for i in idx {
            let it = &self.items[i as usize];
            if it.bounds.intersects(r) {
                h = mix(h, it.seen_from(r));
            }
        }
        h
    }
}

impl Tracker {
    /// The next frame draws everything (a lost or reconfigured target).
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// The damage of a frame `w`×`h` cleared to `clear`, whose primitives,
    /// in painter's order, are `items`, and whose scrolled regions are
    /// `scrolls`: the pixels the kept frame moves first, and what is then
    /// drawn.
    pub fn damage(
        &self,
        w: u32,
        h: u32,
        clear: [u8; 4],
        items: Vec<Item>,
        scrolls: &[Scroll],
    ) -> (Damage, Vec<Shift>, Next) {
        let cols = (w as i32 + TILE - 1) / TILE;
        let rows = (h as i32 + TILE - 1) / TILE;
        let n = (cols.max(0) * rows.max(0)) as usize;
        let seed = mix(0x9e37_79b9_7f4a_7c15, u32::from_le_bytes(clear) as u64);
        let mut tiles = vec![seed; n];
        let mut next = Next {
            size: (w, h),
            cols,
            rows,
            seed,
            tiles: Vec::new(),
            items: Vec::new(),
            buckets: std::cell::OnceCell::new(),
            scrolls: scrolls.to_vec(),
        };
        // each item into its tiles' hashes
        for it in &items {
            let Some((c0, c1, r0, r1)) = next.tile_range(&it.bounds) else {
                continue;
            };
            for row in r0..=r1 {
                let base = (row * cols) as usize;
                for t in &mut tiles[base + c0 as usize..=base + c1 as usize] {
                    *t = mix(*t, it.hash);
                }
            }
        }
        next.tiles = tiles;
        next.items = items;
        let last = match &self.last {
            Some(l) if self.valid && l.size == (w, h) && !full_redraw_forced() => l,
            _ => return (Damage::Full, Vec::new(), next),
        };
        let mut dirty: Vec<bool> = next
            .tiles
            .iter()
            .zip(&last.tiles)
            .map(|(a, b)| a != b)
            .collect();
        let shifts = shifts_of(last, &next, &mut dirty);
        (rects_of(&dirty, cols, rows, w, h), shifts, next)
    }

    /// The frame `next` was drawn: it is what the next one is compared to.
    pub fn commit(&mut self, next: Next) {
        self.last = Some(next);
        self.valid = true;
    }
}

/// The least a shift must save: tiles it keeps that would be drawn.
const SHIFT_MIN_TILES: usize = 4;

/// For each region scrolled by whole pixels since `last`, the tiles
/// inside where its content still shows once the kept frame is moved
/// with it: the move, and those tiles no longer dirty (the others inside
/// it are). `dirty` is updated.
fn shifts_of(last: &Next, next: &Next, dirty: &mut [bool]) -> Vec<Shift> {
    let mut out: Vec<Shift> = Vec::new();
    let mut taken: Vec<IRect> = Vec::new();
    let screen = IRect::new(0, 0, next.size.0 as i32, next.size.1 as i32);
    for s in &next.scrolls {
        let Some(o) = last.scrolls.iter().find(|o| o.id == s.id) else {
            continue;
        };
        let (dx, dy) = (o.dx - s.dx, o.dy - s.dy);
        if (dx == 0.0 && dy == 0.0) || dx.fract() != 0.0 || dy.fract() != 0.0 || o.view != s.view {
            continue;
        }
        let (dx, dy) = (dx as i32, dy as i32);
        // the view, whole pixels inside it
        let v = IRect::new(
            s.view.x0.ceil() as i32,
            s.view.y0.ceil() as i32,
            s.view.x1.floor() as i32,
            s.view.y1.floor() as i32,
        )
        .intersect(&screen);
        let moved = IRect::new(v.x0 + dx, v.y0 + dy, v.x1 + dx, v.y1 + dy);
        let w = v.intersect(&moved);
        // the whole tiles inside it
        let wt = IRect::new(
            (w.x0 + TILE - 1).div_euclid(TILE) * TILE,
            (w.y0 + TILE - 1).div_euclid(TILE) * TILE,
            w.x1.div_euclid(TILE) * TILE,
            w.y1.div_euclid(TILE) * TILE,
        );
        // views that overlap move one at most: a second move would read
        // pixels the first wrote, not the last frame's
        if wt.is_empty() || taken.iter().any(|t| t.intersects(&v)) {
            continue;
        }
        let mut kept = Vec::new();
        let mut gained = 0usize;
        for row in wt.y0 / TILE..wt.y1 / TILE {
            for col in wt.x0 / TILE..wt.x1 / TILE {
                let t = IRect::new(col * TILE, row * TILE, (col + 1) * TILE, (row + 1) * TILE);
                let from = IRect::new(t.x0 - dx, t.y0 - dy, t.x1 - dx, t.y1 - dy);
                let i = (row * next.cols + col) as usize;
                let same = next.seen(&t) == last.seen(&from);
                if same && dirty[i] {
                    gained += 1;
                }
                kept.push((i, same));
            }
        }
        if gained < SHIFT_MIN_TILES {
            continue;
        }
        for (i, same) in kept {
            dirty[i] = !same;
        }
        taken.push(v);
        out.push(Shift {
            src: IRect::new(wt.x0 - dx, wt.y0 - dy, wt.x1 - dx, wt.y1 - dy),
            dst: wt,
        });
    }
    out
}

/// Past this many rectangles a frame draws their bounding box.
const MAX_RECTS: usize = 24;

/// Dirty tiles as disjoint rectangles: runs along each row, joined down
/// when a run below spans the same columns.
fn rects_of(dirty: &[bool], cols: i32, rows: i32, w: u32, h: u32) -> Damage {
    let mut open: Vec<(i32, i32, i32)> = Vec::new(); // c0, c1, first row
    let mut done: Vec<(i32, i32, i32, i32)> = Vec::new(); // c0, c1, r0, r1 (exclusive)
    let mut count = 0usize;
    for row in 0..rows {
        let mut runs = Vec::new();
        let mut c = 0;
        while c < cols {
            if dirty[(row * cols + c) as usize] {
                let s = c;
                while c < cols && dirty[(row * cols + c) as usize] {
                    c += 1;
                }
                runs.push((s, c));
                count += 1;
            } else {
                c += 1;
            }
        }
        let mut still = Vec::new();
        for (c0, c1, r0) in open.drain(..) {
            if let Some(i) = runs.iter().position(|&(a, b)| a == c0 && b == c1) {
                runs.remove(i);
                still.push((c0, c1, r0));
            } else {
                done.push((c0, c1, r0, row));
            }
        }
        still.extend(runs.into_iter().map(|(a, b)| (a, b, row)));
        open = still;
    }
    done.extend(open.into_iter().map(|(a, b, r0)| (a, b, r0, rows)));
    if count == 0 {
        return Damage::None;
    }
    let to_px = |(c0, c1, r0, r1): (i32, i32, i32, i32)| {
        IRect::new(
            c0 * TILE,
            r0 * TILE,
            (c1 * TILE).min(w as i32),
            (r1 * TILE).min(h as i32),
        )
    };
    let rects: Vec<IRect> = done.into_iter().map(to_px).collect();
    let area: i64 = rects.iter().map(|r| r.area()).sum();
    if area * 10 >= w as i64 * h as i64 * 7 {
        return Damage::Full;
    }
    if rects.len() > MAX_RECTS {
        let b = rects.iter().fold(rects[0], |a, r| IRect {
            x0: a.x0.min(r.x0),
            y0: a.y0.min(r.y0),
            x1: a.x1.max(r.x1),
            y1: a.y1.max(r.y1),
        });
        return Damage::Rects(vec![b]);
    }
    Damage::Rects(rects)
}

// ── counters, for a HUD and for measuring ───────────────────────────

/// Frames drawn (whole or in part) and presented.
pub static FRAMES_DRAWN: AtomicU64 = AtomicU64::new(0);
/// Frames whose pixels were all last frame's: nothing drawn or presented.
pub static FRAMES_UNCHANGED: AtomicU64 = AtomicU64::new(0);
/// Pixels drawn, all frames.
pub static PIXELS_DRAWN: AtomicU64 = AtomicU64::new(0);
/// Pixels of the windows those frames covered.
pub static PIXELS_SHOWN: AtomicU64 = AtomicU64::new(0);

/// Count a frame and what it drew.
/// The repaint rainbow's tint for frame `n` (an inspector's: every region
/// a frame drew, each frame a colour of its own), straight RGBA.
pub fn flash_color(n: u32) -> [u8; 4] {
    const HUES: [[u8; 3]; 7] = [
        [255, 59, 48],
        [255, 149, 0],
        [255, 204, 0],
        [52, 199, 89],
        [0, 199, 190],
        [0, 122, 255],
        [175, 82, 222],
    ];
    let c = HUES[n as usize % HUES.len()];
    [c[0], c[1], c[2], 64]
}

/// The rectangles a frame drew, whole or in parts.
pub fn drawn(d: &Damage, w: u32, h: u32) -> Vec<IRect> {
    match d {
        Damage::None => Vec::new(),
        Damage::Full => vec![IRect::new(0, 0, w as i32, h as i32)],
        Damage::Rects(r) => r.clone(),
    }
}

pub fn count(d: &Damage, w: u32, h: u32) {
    match d {
        Damage::None => {
            FRAMES_UNCHANGED.fetch_add(1, Ordering::Relaxed);
        }
        _ => {
            FRAMES_DRAWN.fetch_add(1, Ordering::Relaxed);
            PIXELS_DRAWN.fetch_add(d.area(w, h) as u64, Ordering::Relaxed);
            PIXELS_SHOWN.fetch_add(w as u64 * h as u64, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(x0: i32, y0: i32, x1: i32, y1: i32, h: u64) -> Item {
        Item::new(
            IRect::new(x0, y0, x1, y1),
            h,
            x0 as f32,
            y0 as f32,
            Clip::rect(0.0, 0.0, 4096.0, 4096.0),
        )
    }

    #[test]
    fn the_first_frame_is_whole_and_the_same_frame_again_is_nothing() {
        let mut t = Tracker::default();
        let items = vec![item(0, 0, 400, 300, 1), item(10, 10, 20, 20, 2)];
        let (d, _, n) = t.damage(400, 300, [255; 4], items.clone(), &[]);
        assert_eq!(d, Damage::Full);
        t.commit(n);
        let (d, _, _) = t.damage(400, 300, [255; 4], items.clone(), &[]);
        assert_eq!(d, Damage::None);
    }

    #[test]
    fn a_change_damages_only_its_tiles_and_order_matters() {
        let mut t = Tracker::default();
        let a = vec![
            item(0, 0, 640, 640, 1),
            item(100, 100, 110, 110, 2),
            item(105, 105, 120, 120, 3),
        ];
        let (_, _, n) = t.damage(640, 640, [255; 4], a.clone(), &[]);
        t.commit(n);
        // the caret moves within a tile
        let mut b = a.clone();
        b[1] = item(100, 100, 110, 110, 9);
        let (d, _, _) = t.damage(640, 640, [255; 4], b.clone(), &[]);
        assert_eq!(d, Damage::Rects(vec![IRect::new(64, 64, 128, 128)]));
        // the same primitives, two overlapping ones swapped
        let c = vec![a[0], a[2], a[1]];
        let (d, _, _) = t.damage(640, 640, [255; 4], c.clone(), &[]);
        assert_eq!(d, Damage::Rects(vec![IRect::new(64, 64, 128, 128)]));
        // a new size is whole; a new clear colour damages everything
        assert_eq!(t.damage(641, 640, [255; 4], a.clone(), &[]).0, Damage::Full);
        assert_eq!(t.damage(640, 640, [0; 4], a.clone(), &[]).0, Damage::Full);
    }

    #[test]
    fn a_scroll_by_whole_pixels_moves_the_kept_frame_and_draws_what_it_uncovered() {
        let view = Clip::rect(0.0, 0.0, 512.0, 512.0);
        // rows of 40 px inside the view, scrolled by `off`
        let frame = |off: i32| -> Vec<Item> {
            let mut v = vec![
                Item::new(
                    IRect::new(0, 0, 512, 512),
                    1,
                    0.0,
                    0.0,
                    Clip::rect(0.0, 0.0, 512.0, 512.0),
                )
                .with_interior(IRect::new(0, 0, 512, 512)),
            ];
            for r in 0..40 {
                let y = r * 40 - off;
                let b = IRect::new(0, y - 1, 512, y + 41).intersect(&IRect::new(0, 0, 512, 512));
                if !b.is_empty() {
                    v.push(Item::new(b, 100 + r as u64, 0.0, y as f32, view));
                }
            }
            v
        };
        let mut t = Tracker::default();
        let s = |off: i32| {
            vec![Scroll {
                id: 7,
                view,
                dx: 0.0,
                dy: off as f32,
            }]
        };
        let (_, _, n) = t.damage(512, 512, [255; 4], frame(0), &s(0));
        t.commit(n);
        let (d, shifts, _) = t.damage(512, 512, [255; 4], frame(64), &s(64));
        assert_eq!(
            shifts,
            vec![Shift {
                src: IRect::new(0, 64, 512, 512),
                dst: IRect::new(0, 0, 512, 448)
            }]
        );
        // only the uncovered band is drawn
        assert_eq!(d, Damage::Rects(vec![IRect::new(0, 448, 512, 512)]));
    }

    #[test]
    fn dirty_tiles_become_disjoint_rectangles() {
        // an L of dirty tiles in a 4×4 grid
        let mut dirty = vec![false; 16];
        for i in [0, 1, 4, 5, 8] {
            dirty[i] = true;
        }
        let Damage::Rects(r) = rects_of(&dirty, 4, 4, 256, 256) else {
            panic!("rects")
        };
        assert_eq!(
            r,
            vec![IRect::new(0, 0, 128, 128), IRect::new(0, 128, 64, 192)]
        );
        for (i, a) in r.iter().enumerate() {
            for b in &r[i + 1..] {
                assert!(!a.intersects(b));
            }
        }
    }

    #[test]
    fn bounds_hold_the_antialiased_edge_and_respect_the_clip() {
        let clip = Clip::rect(0.0, 0.0, 50.0, 50.0);
        let b = bounds(10.5, 10.5, 5.0, 5.0, 1.0, &clip, 100, 100).unwrap();
        assert!(b.x0 <= 9 && b.y0 <= 9 && b.x1 >= 17 && b.y1 >= 17);
        assert!(bounds(60.0, 60.0, 5.0, 5.0, 1.0, &clip, 100, 100).is_none());
        let b = bounds(40.0, 40.0, 30.0, 30.0, 1.0, &clip, 100, 100).unwrap();
        assert!(b.x1 <= 51 && b.y1 <= 51);
    }
}

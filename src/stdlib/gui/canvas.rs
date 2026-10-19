//! Pictures: a `canvas` node's drawing and an `image` node's file, as
//! what both renderers draw.
//!
//! A canvas is an immediate-mode draw list: each frame the program says
//! what is visible, and the engine draws it. Its operations become the
//! window's own primitives, so a canvas of a few hundred shapes costs what
//! as many boxes do and a pan redraws without a raster of the whole
//! canvas:
//!
//! - rectangles and axis-aligned lines are the renderers' rectangles
//!   (corners, borders, clips);
//! - text is shaped and drawn as any other text (sharp, measured alike),
//!   cut with an ellipsis to `max_w` when it says so;
//! - a gradient's stops are mixed in linear light into a strip: a linear
//!   one is the strip stretched over its rectangle, a radial one
//!   (`dir: "radial"`) the strip read by the distance from its centre
//!   (both renderers, the GPU in its shader), clipped to its corners;
//! - a circle is a box with round corners (drawn exactly by both);
//! - paths (lines and curves: `points` or `d`), dashed circles and slanted
//!   lines are vector paths: tiny-skia draws them in software, the GPU
//!   draws them itself (gpu.rs: coverage on the GPU, nothing rasterized
//!   on the CPU), with their caps, joins and dashes;
//! - an image is a picture file, decoded off the window's thread
//!   (picture.rs).
//!
//! `push` and `pop` keep a stack of clips and transforms (`translate`,
//! `scale`): a transform moves and sizes geometry, never a stroke's width,
//! a corner's radius, or a text's size, so a timeline can zoom its days
//! while its labels stay the size they are. An operation with a `hit`
//! value is a hit region: `hit` answers the topmost one under a point
//! (the window adds it to a pointer event over the canvas).

use super::text::Color;
use super::values::*;
use crate::ast::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tiny_skia::{LineCap, LineJoin, PathBuilder, Stroke};

/// A picture both renderers draw: premultiplied RGBA, `id` unique to its
/// content (the GPU keys its texture by it).
#[derive(Debug)]
pub struct Picture {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A fresh picture id (the GPU keys a picture's texture by it).
pub fn next_picture_id() -> u64 {
    NEXT.fetch_add(1, Ordering::SeqCst)
}

/// A clip in the canvas's own pixels: `[x0, y0, x1, y1]`.
pub type Area = [f32; 4];

/// A canvas's text, drawn by the window as text.
#[derive(Clone, Debug)]
pub struct CanvasText {
    pub x: f32,
    /// The top of the line, as everywhere else in the engine.
    pub y: f32,
    pub text: String,
    pub size: f32,
    pub weight: f32,
    pub color: Color,
    /// 0 start, 1 centre, 2 end: where `x` is on the line.
    pub align: u8,
    /// Cut with an ellipsis to this width, when given.
    pub max_w: Option<f32>,
    pub clip: Area,
    /// A family other than the node's (`"mono"`, `"sans"`).
    pub font: Option<String>,
    /// Slanted (a terminal's italic).
    pub italic: bool,
}

/// What a canvas draws, in its own logical pixels, in order.
#[derive(Clone, Debug)]
pub enum Item {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: Color,
        border: Color,
        border_width: f32,
        radii: [f32; 4],
        clip: Area,
    },
    /// A picture made here (a raster piece, a gradient) stretched over
    /// its box; `radii` round the box's corners.
    Pic {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        pic: Arc<Picture>,
        radii: [f32; 4],
        clip: Area,
    },
    /// A radial gradient over a box: its strip's colours from the box's
    /// centre (0) to its corners (1); `radii` round the box.
    Radial {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        strip: Arc<Picture>,
        radii: [f32; 4],
        clip: Area,
    },
    /// A picture file (picture.rs), fitted to its box.
    Image {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        src: Value,
        fit: String,
        clip: Area,
    },
    Text(CanvasText),
    /// A vector path (lines, curves), filled and or stroked: drawn by the
    /// renderers as geometry — tiny-skia in software, the GPU's own
    /// coverage on the GPU — never a picture.
    Path(PathItem),
}

/// How a stroke's open ends are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

/// How a stroke turns a corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

/// A path's stroke, in the canvas's logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeSpec {
    pub color: Color,
    pub width: f32,
    pub cap: Cap,
    pub join: Join,
    /// On, off, on, … lengths; `None`: solid.
    pub dash: Option<Arc<[f32]>>,
    pub dash_offset: f32,
}

impl StrokeSpec {
    /// The same stroke for tiny-skia, its width and dashes at `scale`
    /// times (the path is drawn through a transform that scales it).
    pub fn skia(&self) -> Stroke {
        let mut s = Stroke {
            width: self.width.max(0.0),
            line_cap: match self.cap {
                Cap::Butt => LineCap::Butt,
                Cap::Round => LineCap::Round,
                Cap::Square => LineCap::Square,
            },
            line_join: match self.join {
                Join::Miter => LineJoin::Miter,
                Join::Round => LineJoin::Round,
                Join::Bevel => LineJoin::Bevel,
            },
            ..Default::default()
        };
        if let Some(d) = &self.dash {
            s.dash = tiny_skia::StrokeDash::new(d.to_vec(), self.dash_offset);
        }
        s
    }

    /// How far past the path its stroke can reach.
    pub fn reach(&self) -> f32 {
        let h = self.width.max(0.0) / 2.0;
        match (self.join, self.cap) {
            (Join::Miter, _) => h * 4.0,
            (_, Cap::Square) => h * std::f32::consts::SQRT_2,
            _ => h,
        }
    }
}

/// A path in the canvas's logical pixels and how it is painted.
#[derive(Clone, Debug)]
pub struct PathItem {
    pub path: Arc<tiny_skia::Path>,
    pub fill: Option<Color>,
    /// The even-odd rule (else non-zero).
    pub even_odd: bool,
    pub stroke: Option<StrokeSpec>,
    pub clip: Area,
    /// A hash of the path and its paint (what damage compares).
    pub ident: u64,
}

/// A canvas's drawing, and its hit regions (topmost last).
#[derive(Clone, Debug, Default)]
pub struct Drawing {
    pub items: Vec<Item>,
    pub hits: Vec<(Area, Value)>,
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    kx: f32,
    ky: f32,
    tx: f32,
    ty: f32,
    clip: Area,
}

impl Frame {
    fn pt(&self, x: f32, y: f32) -> (f32, f32) {
        (self.tx + self.kx * x, self.ty + self.ky * y)
    }
    /// A box through the transform, as `(x, y, w, h)` with a positive size.
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32, f32, f32) {
        let (x0, y0) = self.pt(x, y);
        let (x1, y1) = self.pt(x + w, y + h);
        (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
    }
}

fn meet(a: Area, b: Area) -> Area {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    [x0, y0, a[2].min(b[2]).max(x0), a[3].min(b[3]).max(y0)]
}

fn empty(a: &Area) -> bool {
    a[2] <= a[0] || a[3] <= a[1]
}

const CLEAR: Color = [0, 0, 0, 0];

/// The pieces rasterized here, by their shape on their own box, with the
/// tick each was last drawn: past [`PIECES_MAX`] or [`PIECES_BYTES`] the
/// least recently drawn quarter goes (a piece kept keeps its picture id,
/// so its GPU texture and the window's damage stay as they were).
#[derive(Default)]
struct Pieces {
    map: HashMap<u64, (Arc<Picture>, u64)>,
    bytes: usize,
    tick: u64,
    evicted: u64,
}

const PIECES_MAX: usize = 4096;
const PIECES_BYTES: usize = 64 << 20;

fn pieces() -> &'static Mutex<Pieces> {
    static CACHE: OnceLock<Mutex<Pieces>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Pieces::default()))
}

/// What the piece cache holds: `(pieces, bytes, evicted ever)`.
pub fn pieces_stats() -> (usize, usize, u64) {
    match pieces().lock() {
        Ok(p) => (p.map.len(), p.bytes, p.evicted),
        Err(_) => (0, 0, 0),
    }
}

fn cached(key: u64, make: impl FnOnce() -> Option<Picture>) -> Option<Arc<Picture>> {
    if let Ok(mut c) = pieces().lock() {
        c.tick += 1;
        let now = c.tick;
        if let Some(p) = c.map.get_mut(&key) {
            p.1 = now;
            return Some(p.0.clone());
        }
    }
    let pic = Arc::new(make()?);
    if let Ok(mut c) = pieces().lock() {
        if c.map.len() >= PIECES_MAX || c.bytes + pic.rgba.len() > PIECES_BYTES {
            let mut ticks: Vec<u64> = c.map.values().map(|v| v.1).collect();
            let cut = ticks.len() / 4;
            if cut > 0 {
                let nth = *ticks.select_nth_unstable(cut).1;
                let before = c.map.len();
                let mut freed = 0;
                c.map.retain(|_, v| {
                    let keep = v.1 > nth;
                    if !keep {
                        freed += v.0.rgba.len();
                    }
                    keep
                });
                c.bytes -= freed;
                c.evicted += (before - c.map.len()) as u64;
            }
        }
        let now = c.tick;
        c.bytes += pic.rgba.len();
        if let Some((old, _)) = c.map.insert(key, (pic.clone(), now)) {
            c.bytes -= old.rgba.len();
        }
    }
    Some(pic)
}

/// Entries in a gradient's strip.
pub const STRIP: usize = 256;

/// An sRGB-encoded channel (0 … 1) in linear light.
pub fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// A linear-light channel (0 … 1) sRGB-encoded.
pub fn to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// A gradient's colour at `t` (0 … 1) along evenly spaced `stops`, mixed
/// in linear light, premultiplied (CSS Color 4's interpolation in
/// `srgb-linear`): no muddy band between two hues, and a fade to
/// `transparent` keeps its colour. Straight sRGB, 0 … 255.
pub fn gradient_at(stops: &[Color], t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
    let k = (t.floor() as usize).min(stops.len() - 2);
    let f = t - k as f32;
    let (a, b) = (stops[k], stops[k + 1]);
    let (aa, ba) = (a[3] as f32 / 255.0, b[3] as f32 / 255.0);
    let alpha = aa + (ba - aa) * f;
    let mut out = [0.0f32; 4];
    for j in 0..3 {
        let pa = to_linear(a[j] as f32 / 255.0) * aa;
        let pb = to_linear(b[j] as f32 / 255.0) * ba;
        let p = pa + (pb - pa) * f;
        out[j] = if alpha > 0.0 { to_srgb((p / alpha).clamp(0.0, 1.0)) * 255.0 } else { 0.0 };
    }
    out[3] = alpha * 255.0;
    out
}

/// A gradient's stops as a [`STRIP`]-pixel strip (across for `"x"` and a
/// radial one, down for `"y"`), mixed in linear light, premultiplied;
/// cached by its colours.
pub fn gradient_strip(stops: &[Color], down: bool) -> Option<Arc<Picture>> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("gradient-linear-light", stops, down).hash(&mut h);
    cached(h.finish(), || {
        let n = STRIP;
        let mut rgba = Vec::with_capacity(n * 4);
        for i in 0..n {
            let c = gradient_at(stops, i as f32 / (n - 1) as f32);
            let alpha = c[3] / 255.0;
            // premultiplied, as both renderers take pictures
            for v in &c[..3] {
                rgba.push((v * alpha).round().clamp(0.0, 255.0) as u8);
            }
            rgba.push(c[3].round().clamp(0.0, 255.0) as u8);
        }
        let (width, height) = if down { (1, n as u32) } else { (n as u32, 1) };
        Some(Picture {
            id: next_picture_id(),
            width,
            height,
            rgba,
        })
    })
}

fn colors_of(v: Option<&Value>, what: &str) -> Res<Vec<Color>> {
    match v {
        None | Some(Value::Unit) => Ok(vec![]),
        Some(Value::List(l)) => l
            .iter()
            .map(|c| {
                parse_color(c).ok_or_else(|| format!("{what}: \"gradient\" holds colours, got {c}"))
            })
            .collect(),
        Some(other) => Err(format!(
            "{what}: \"gradient\" is a list of colours, got {other}"
        )),
    }
}

fn radii_of(op: &Value, what: &str) -> Res<[f32; 4]> {
    match get(op, "radius") {
        None | Some(Value::Unit) => Ok([0.0; 4]),
        Some(v) => match num(v) {
            Some(r) => Ok([r; 4]),
            None => match nums(v).as_deref() {
                Some([a, b, c, d]) => Ok([*a, *b, *c, *d]),
                _ => Err(format!("{what}: \"radius\" is a number or four")),
            },
        },
    }
}

/// An op's stroke (`stroke`/`color`, `width`, `cap`, `join`, `dash`,
/// `dash_offset`), its geometry through the frame's transform only where
/// it is a path's (widths and dashes are never scaled by it).
fn stroke_spec(op: &Value, color: Color, width: f32, cap: Cap, what: &str) -> Res<StrokeSpec> {
    let cap = match get_str(op, "cap", what)? {
        None => cap,
        Some("butt") => Cap::Butt,
        Some("round") => Cap::Round,
        Some("square") => Cap::Square,
        Some(o) => return Err(format!("{what}: \"cap\" is butt, round or square, got \"{o}\"")),
    };
    let join = match get_str(op, "join", what)? {
        None | Some("round") => Join::Round,
        Some("miter") => Join::Miter,
        Some("bevel") => Join::Bevel,
        Some(o) => return Err(format!("{what}: \"join\" is round, miter or bevel, got \"{o}\"")),
    };
    let dash = match get(op, "dash") {
        None | Some(Value::Unit) => None,
        Some(v) => {
            let d = nums(v).ok_or_else(|| format!("{what}: \"dash\" is a list of lengths"))?;
            // tiny-skia wants an even count of positive lengths
            let mut d: Vec<f32> = d.into_iter().map(|x| x.max(0.0)).collect();
            if d.len() % 2 == 1 {
                d.extend(d.clone());
            }
            if d.len() < 2 || d.iter().sum::<f32>() <= 0.0 || d.iter().any(|x| !x.is_finite()) {
                None
            } else {
                Some(Arc::from(d.into_boxed_slice()))
            }
        }
    };
    Ok(StrokeSpec {
        color,
        width: if width.is_finite() { width.max(0.0) } else { 0.0 },
        cap,
        join,
        dash,
        dash_offset: get_num(op, "dash_offset", what)?.unwrap_or(0.0),
    })
}

/// A path's `d`: a list of commands, each a list or tuple — `("M", x, y)`,
/// `("L", x, y)`, `("Q", cx, cy, x, y)`, `("C", c1x, c1y, c2x, c2y, x, y)`,
/// `("Z")` — through the frame's transform.
fn path_of_d(d: &Value, f: &Frame, what: &str) -> Res<Option<tiny_skia::Path>> {
    let cmds: &[Value] = match d {
        Value::List(l) => l,
        Value::Tuple(t) => t,
        _ => return Err(format!("{what}: \"d\" is a list of commands")),
    };
    let mut pb = PathBuilder::new();
    let mut open = false;
    for c in cmds {
        let parts: &[Value] = match c {
            Value::List(l) => l,
            Value::Tuple(t) => t,
            _ => return Err(format!("{what}: a command of \"d\" is a list, got {c}")),
        };
        let Some(Value::String(name)) = parts.first() else {
            return Err(format!("{what}: a command of \"d\" starts with its letter (M, L, Q, C, Z)"));
        };
        let ns: Vec<f32> = parts[1..].iter().filter_map(num).collect();
        let need = |k: usize| -> Res<()> {
            if ns.len() < k || ns.iter().take(k).any(|v| !v.is_finite()) {
                Err(format!("{what}: \"{name}\" takes {k} numbers"))
            } else {
                Ok(())
            }
        };
        match name.as_str() {
            "M" => {
                need(2)?;
                let (x, y) = f.pt(ns[0], ns[1]);
                pb.move_to(x, y);
                open = true;
            }
            "L" => {
                need(2)?;
                let (x, y) = f.pt(ns[0], ns[1]);
                if open { pb.line_to(x, y) } else { pb.move_to(x, y) }
                open = true;
            }
            "Q" => {
                need(4)?;
                let (cx, cy) = f.pt(ns[0], ns[1]);
                let (x, y) = f.pt(ns[2], ns[3]);
                if !open {
                    pb.move_to(cx, cy);
                }
                pb.quad_to(cx, cy, x, y);
                open = true;
            }
            "C" => {
                need(6)?;
                let (ax, ay) = f.pt(ns[0], ns[1]);
                let (bx, by) = f.pt(ns[2], ns[3]);
                let (x, y) = f.pt(ns[4], ns[5]);
                if !open {
                    pb.move_to(ax, ay);
                }
                pb.cubic_to(ax, ay, bx, by, x, y);
                open = true;
            }
            "Z" => pb.close(),
            o => return Err(format!("{what}: unknown command \"{o}\" in \"d\" (M, L, Q, C, Z)")),
        }
    }
    Ok(pb.finish())
}

/// A path item: its paint, and a hash of all of it (damage's).
fn path_item(path: tiny_skia::Path, fill: Option<Color>, even_odd: bool, stroke: Option<StrokeSpec>, clip: Area) -> Item {
    let mut h = rustc_hash::FxHasher::default();
    for p in path.points() {
        p.x.to_bits().hash(&mut h);
        p.y.to_bits().hash(&mut h);
    }
    for v in path.verbs() {
        (*v as u8).hash(&mut h);
    }
    fill.hash(&mut h);
    even_odd.hash(&mut h);
    if let Some(s) = &stroke {
        (s.color, s.width.to_bits(), s.cap, s.join, s.dash_offset.to_bits()).hash(&mut h);
        if let Some(d) = &s.dash {
            for v in d.iter() {
                v.to_bits().hash(&mut h);
            }
        }
    }
    Item::Path(PathItem {
        path: Arc::new(path),
        fill,
        even_odd,
        stroke,
        clip,
        ident: h.finish(),
    })
}

/// The drawing of a canvas of `w × h` logical pixels at `scale`: its
/// items and hit regions. `raster` false skips the pictures (a hit test).
pub fn draw_at(ops: &Value, w: f32, h: f32, _scale: f32, raster: bool) -> Res<Drawing> {
    let items: &[Value] = match ops {
        Value::List(l) => l,
        _ => return Err("canvas: \"draw\" must be a list of operations".into()),
    };
    let mut out = Drawing::default();
    let mut stack = vec![Frame {
        kx: 1.0,
        ky: 1.0,
        tx: 0.0,
        ty: 0.0,
        clip: [0.0, 0.0, w, h],
    }];
    for (i, op) in items.iter().enumerate() {
        let what = format!("canvas: operation {}", i + 1);
        let kind = get_str(op, "op", &what)?.ok_or_else(|| format!("{what}: needs an \"op\""))?;
        let f = *stack.last().expect("the base frame stays");
        let n = |k: &str| -> Res<f32> { Ok(get_num(op, k, &what)?.unwrap_or(0.0)) };
        let fill = get_color(op, "fill", &what)?;
        let line_c = get_color(op, "stroke", &what)?.or(get_color(op, "color", &what)?);
        let stroked = get(op, "stroke").is_some_and(|v| *v != Value::Unit);
        let width = get_num(op, "width", &what)?.unwrap_or(1.0);
        let hit = get(op, "hit").filter(|v| **v != Value::Unit).cloned();
        // the op's box after the transform, for its hit region
        let bbox: Option<(f32, f32, f32, f32)>;
        match kind {
            "push" => {
                let mut g = f;
                if let Some(c) = get_rect(op, "clip", &what)? {
                    let (cx, cy, cw, ch) = f.rect(c[0], c[1], c[2], c[3]);
                    g.clip = meet(f.clip, [cx, cy, cx + cw, cy + ch]);
                }
                let (ttx, tty) = get_pair(op, "translate", &what)?.unwrap_or((0.0, 0.0));
                let (skx, sky) = match get(op, "scale") {
                    None | Some(Value::Unit) => (1.0, 1.0),
                    Some(v) => match num(v) {
                        Some(k) => (k, k),
                        None => get_pair(op, "scale", &what)?.unwrap_or((1.0, 1.0)),
                    },
                };
                // outer(translate + scale · p)
                let (ox, oy) = f.pt(ttx, tty);
                g.tx = ox;
                g.ty = oy;
                g.kx = f.kx * skx;
                g.ky = f.ky * sky;
                stack.push(g);
                continue;
            }
            "pop" => {
                if stack.len() > 1 {
                    stack.pop();
                }
                continue;
            }
            "rect" => {
                let (x, y, rw, rh) = f.rect(n("x")?, n("y")?, n("w")?, n("h")?);
                let radii = radii_of(op, &what)?;
                bbox = Some((x, y, rw, rh));
                let stops = colors_of(get(op, "gradient"), &what)?;
                if stops.len() >= 2 {
                    let dir = get_str(op, "dir", &what)?.unwrap_or("x");
                    let down = dir == "y";
                    if dir == "radial" {
                        if raster && let Some(pic) = gradient_strip(&stops, false) {
                            out.items.push(Item::Radial {
                                x,
                                y,
                                w: rw,
                                h: rh,
                                strip: pic,
                                radii,
                                clip: f.clip,
                            });
                        }
                    } else if raster && let Some(pic) = gradient_strip(&stops, down) {
                        out.items.push(Item::Pic {
                            x,
                            y,
                            w: rw,
                            h: rh,
                            pic,
                            radii,
                            clip: f.clip,
                        });
                    }
                } else if fill.is_some() || stroked {
                    // a stroke on the rectangle's edge, centred on it
                    let bw = if stroked { width.max(0.0) } else { 0.0 };
                    out.items.push(Item::Rect {
                        x: x - bw / 2.0,
                        y: y - bw / 2.0,
                        w: rw + bw,
                        h: rh + bw,
                        fill: fill.unwrap_or(CLEAR),
                        border: if stroked {
                            line_c.unwrap_or(CLEAR)
                        } else {
                            CLEAR
                        },
                        border_width: bw,
                        radii: radii.map(|r| if r > 0.0 { r + bw / 2.0 } else { 0.0 }),
                        clip: f.clip,
                    });
                }
                if stops.len() >= 2 && stroked {
                    out.items.push(Item::Rect {
                        x: x - width / 2.0,
                        y: y - width / 2.0,
                        w: rw + width,
                        h: rh + width,
                        fill: CLEAR,
                        border: line_c.unwrap_or(CLEAR),
                        border_width: width,
                        radii: radii.map(|r| if r > 0.0 { r + width / 2.0 } else { 0.0 }),
                        clip: f.clip,
                    });
                }
            }
            "line" => {
                let (x1, y1) = f.pt(n("x1")?, n("y1")?);
                let (x2, y2) = f.pt(n("x2")?, n("y2")?);
                let c = line_c.unwrap_or([24, 24, 27, 255]);
                bbox = Some((
                    x1.min(x2) - width / 2.0,
                    y1.min(y2) - width / 2.0,
                    (x2 - x1).abs() + width,
                    (y2 - y1).abs() + width,
                ));
                let plain = get(op, "cap").is_none() && get(op, "dash").is_none();
                if plain && (x1 == x2 || y1 == y2) {
                    // along an axis: a rectangle, its ends square
                    let (x, y, lw, lh) = if y1 == y2 {
                        (x1.min(x2), y1 - width / 2.0, (x2 - x1).abs(), width)
                    } else {
                        (x1 - width / 2.0, y1.min(y2), width, (y2 - y1).abs())
                    };
                    out.items.push(Item::Rect {
                        x,
                        y,
                        w: lw,
                        h: lh,
                        fill: c,
                        border: CLEAR,
                        border_width: 0.0,
                        radii: [0.0; 4],
                        clip: f.clip,
                    });
                } else if raster {
                    let mut pb = PathBuilder::new();
                    pb.move_to(x1, y1);
                    pb.line_to(x2, y2);
                    if let Some(p) = pb.finish() {
                        let st = stroke_spec(op, c, width, Cap::Round, &what)?;
                        out.items.push(path_item(p, None, false, Some(st), f.clip));
                    }
                }
            }
            "circle" => {
                let (cx, cy) = f.pt(n("cx")?, n("cy")?);
                // a circle through a transform that stretches stays round
                let r = n("r")?.max(0.0);
                bbox = Some((cx - r, cy - r, 2.0 * r, 2.0 * r));
                let s = if stroked { line_c.map(|c| (c, width)) } else { None };
                let dashed = get(op, "dash").is_some_and(|v| *v != Value::Unit);
                if r > 0.0 && !dashed && (fill.is_some() || s.is_some()) {
                    // a box with round corners: drawn exactly by both renderers
                    let bw = s.map(|(_, w)| w.max(0.0)).unwrap_or(0.0);
                    out.items.push(Item::Rect {
                        x: cx - r - bw / 2.0,
                        y: cy - r - bw / 2.0,
                        w: 2.0 * r + bw,
                        h: 2.0 * r + bw,
                        fill: fill.unwrap_or(CLEAR),
                        border: s.map(|(c, _)| c).unwrap_or(CLEAR),
                        border_width: bw,
                        radii: [r + bw / 2.0; 4],
                        clip: f.clip,
                    });
                } else if r > 0.0
                    && raster
                    && (fill.is_some() || s.is_some())
                    && let Some(path) = PathBuilder::from_circle(cx, cy, r)
                {
                    let st = match s {
                        Some((c, w)) => Some(stroke_spec(op, c, w, Cap::Butt, &what)?),
                        None => None,
                    };
                    out.items.push(path_item(path, fill, false, st, f.clip));
                }
            }
            "path" => {
                let path = if let Some(d) = get(op, "d").filter(|v| **v != Value::Unit) {
                    path_of_d(d, &f, &what)?
                } else {
                    let pts = match get(op, "points") {
                        Some(Value::List(l)) => l.iter().filter_map(nums).collect::<Vec<_>>(),
                        _ => vec![],
                    };
                    let mut pb = PathBuilder::new();
                    let mut first = true;
                    for p in pts.iter().filter(|p| p.len() >= 2) {
                        let (x, y) = f.pt(p[0], p[1]);
                        if !(x.is_finite() && y.is_finite()) {
                            continue;
                        }
                        if first {
                            pb.move_to(x, y);
                            first = false;
                        } else {
                            pb.line_to(x, y);
                        }
                    }
                    if get_bool(op, "close", &what)?.unwrap_or(false) {
                        pb.close();
                    }
                    pb.finish()
                };
                let Some(path) = path else { continue };
                let b = path.bounds();
                bbox = Some((b.left(), b.top(), b.width(), b.height()));
                let even_odd = get_str(op, "rule", &what)? == Some("evenodd");
                let st = if stroked {
                    match line_c {
                        Some(c) => Some(stroke_spec(op, c, width, Cap::Round, &what)?),
                        None => None,
                    }
                } else {
                    None
                };
                if raster && (fill.is_some() || st.is_some()) {
                    out.items.push(path_item(path, fill, even_odd, st, f.clip));
                }
            }
            "image" => {
                let (x, y, iw, ih) = f.rect(n("x")?, n("y")?, n("w")?, n("h")?);
                bbox = Some((x, y, iw, ih));
                if let Some(src) = get(op, "src").filter(|v| **v != Value::Unit) {
                    out.items.push(Item::Image {
                        x,
                        y,
                        w: iw,
                        h: ih,
                        src: src.clone(),
                        fit: get_str(op, "fit", &what)?.unwrap_or("contain").to_string(),
                        clip: f.clip,
                    });
                }
            }
            "text" => {
                let (x, y) = f.pt(n("x")?, n("y")?);
                let size = get_num(op, "size", &what)?.unwrap_or(12.0);
                let max_w = get_num(op, "max_w", &what)?.filter(|m| *m > 0.0);
                let align = match get_str(op, "align", &what)?.unwrap_or("start") {
                    "center" => 1,
                    "end" => 2,
                    _ => 0,
                };
                // a text's hit region is the box it says it takes (`w`,
                // `h`), else its line at `max_w`
                let tw = get_num(op, "w", &what)?.or(max_w).unwrap_or(0.0);
                let th = get_num(op, "h", &what)?.unwrap_or(size * 1.3);
                let tx0 = match align {
                    1 => x - tw / 2.0,
                    2 => x - tw,
                    _ => x,
                };
                bbox = Some((tx0, y, tw, th));
                out.items.push(Item::Text(CanvasText {
                    x,
                    y,
                    text: get_str(op, "text", &what)?.unwrap_or("").to_string(),
                    size,
                    weight: get_num(op, "weight", &what)?.unwrap_or(400.0),
                    color: line_c.or(fill).unwrap_or([24, 24, 27, 255]),
                    align,
                    max_w,
                    clip: f.clip,
                    font: get_str(op, "font", &what)?.map(|f| f.to_string()),
                    italic: matches!(get(op, "italic"), Some(Value::Boolean(true))),
                }));
            }
            other => {
                return Err(format!(
                    "{what}: unknown op \"{other}\" (rect, line, circle, path, text, image, push, pop)"
                ));
            }
        }
        if let (Some(v), Some((x, y, bw, bh))) = (hit, bbox) {
            let a = meet(f.clip, [x, y, x + bw, y + bh]);
            if !empty(&a) {
                out.hits.push((a, v));
            }
        }
    }
    Ok(out)
}

/// The drawing of a canvas of `w × h` logical pixels at `scale`.
pub fn draw(ops: &Value, w: f32, h: f32, scale: f32) -> Res<Drawing> {
    draw_at(ops, w, h, scale, true)
}

/// The hit region under `(x, y)` (the canvas's own pixels): the `hit`
/// value of the topmost operation that says one, or `()`.
pub fn hit(ops: &Value, w: f32, h: f32, x: f32, y: f32) -> Value {
    match draw_at(ops, w, h, 1.0, false) {
        Ok(d) => d
            .hits
            .iter()
            .rev()
            .find(|(a, _)| x >= a[0] && x < a[2] && y >= a[1] && y < a[3])
            .map(|(_, v)| v.clone())
            .unwrap_or(Value::Unit),
        Err(_) => Value::Unit,
    }
}

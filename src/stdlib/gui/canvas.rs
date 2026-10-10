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
//! - a gradient is a small picture of its stops stretched over its
//!   rectangle, clipped to its corners;
//! - paths, circles, and slanted lines are rasterized with tiny-skia, each
//!   on its own bounding box and cached by its shape there: the same
//!   arrowhead a hundred times, or a pan by whole pixels, is one raster;
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
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

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

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn stroke_of(w: f32) -> Stroke {
    Stroke {
        width: w.max(0.0),
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    }
}

const CLEAR: Color = [0, 0, 0, 0];

/// The pieces rasterized here, by their shape on their own box.
fn pieces() -> &'static Mutex<HashMap<u64, Arc<Picture>>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Arc<Picture>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached(key: u64, make: impl FnOnce() -> Option<Picture>) -> Option<Arc<Picture>> {
    if let Some(p) = pieces().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Some(p);
    }
    let pic = Arc::new(make()?);
    if let Ok(mut c) = pieces().lock() {
        if c.len() > 1024 {
            c.clear();
        }
        c.insert(key, pic.clone());
    }
    Some(pic)
}

/// A path (in the canvas's logical pixels) filled and stroked on its own
/// bounding box at `scale`: the piece's top left (logical, on a device
/// pixel) and its picture. The shape is cached relative to the piece, so
/// the same shape elsewhere by whole device pixels is the same picture.
fn raster_piece(
    path: &tiny_skia::Path,
    fill: Option<Color>,
    stroke: Option<(Color, f32)>,
    scale: f32,
) -> Option<(f32, f32, Arc<Picture>)> {
    let b = path.bounds();
    let pad = stroke.map(|(_, w)| w / 2.0).unwrap_or(0.0) + 1.0;
    let dx0 = ((b.left() - pad) * scale).floor();
    let dy0 = ((b.top() - pad) * scale).floor();
    let dw = (((b.right() + pad) * scale).ceil() - dx0).clamp(1.0, 4096.0) as u32;
    let dh = (((b.bottom() + pad) * scale).ceil() - dy0).clamp(1.0, 4096.0) as u32;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in path.points() {
        // relative to the piece, to an eighth of a device pixel
        (((p.x * scale - dx0) * 8.0).round() as i64).hash(&mut h);
        (((p.y * scale - dy0) * 8.0).round() as i64).hash(&mut h);
    }
    for v in path.verbs() {
        (*v as u8).hash(&mut h);
    }
    fill.hash(&mut h);
    stroke.map(|(c, w)| (c, (w * 100.0) as i64)).hash(&mut h);
    (dw, dh, (scale * 100.0) as i64).hash(&mut h);
    let key = h.finish();
    let pic = cached(key, || {
        let mut pm = Pixmap::new(dw, dh)?;
        let tf = Transform::from_row(scale, 0.0, 0.0, scale, -dx0, -dy0);
        if let Some(f) = fill {
            pm.fill_path(path, &paint(f), FillRule::Winding, tf, None);
        }
        if let Some((c, w)) = stroke {
            pm.stroke_path(path, &paint(c), &stroke_of(w), tf, None);
        }
        Some(Picture {
            id: next_picture_id(),
            width: pm.width(),
            height: pm.height(),
            rgba: pm.data().to_vec(),
        })
    })?;
    Some((dx0 / scale, dy0 / scale, pic))
}

/// A gradient's stops as a 256-pixel strip (across for `"x"`, down for
/// `"y"`), cached by its colours.
fn gradient_strip(stops: &[Color], down: bool) -> Option<Arc<Picture>> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ("gradient", stops, down).hash(&mut h);
    cached(h.finish(), || {
        let n = 256usize;
        let mut rgba = Vec::with_capacity(n * 4);
        for i in 0..n {
            let t = i as f32 / (n - 1) as f32 * (stops.len() - 1) as f32;
            let k = (t.floor() as usize).min(stops.len() - 2);
            let f = t - k as f32;
            let (a, b) = (stops[k], stops[k + 1]);
            let mix = |j: usize| a[j] as f32 + (b[j] as f32 - a[j] as f32) * f;
            let alpha = mix(3) / 255.0;
            // premultiplied, as both renderers take pictures
            for j in 0..3 {
                rgba.push((mix(j) * alpha).round().clamp(0.0, 255.0) as u8);
            }
            rgba.push(mix(3).round().clamp(0.0, 255.0) as u8);
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

/// The drawing of a canvas of `w × h` logical pixels at `scale`: its
/// items and hit regions. `raster` false skips the pictures (a hit test).
pub fn draw_at(ops: &Value, w: f32, h: f32, scale: f32, raster: bool) -> Res<Drawing> {
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
                    let down = get_str(op, "dir", &what)?.unwrap_or("x") == "y";
                    if raster && let Some(pic) = gradient_strip(&stops, down) {
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
                if x1 == x2 || y1 == y2 {
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
                    if let Some(p) = pb.finish()
                        && let Some((px, py, pic)) = raster_piece(&p, None, Some((c, width)), scale)
                    {
                        out.items.push(Item::Pic {
                            x: px,
                            y: py,
                            w: pic.width as f32 / scale,
                            h: pic.height as f32 / scale,
                            pic,
                            radii: [0.0; 4],
                            clip: f.clip,
                        });
                    }
                }
            }
            "circle" | "path" => {
                let path = if kind == "circle" {
                    let (cx, cy) = f.pt(n("cx")?, n("cy")?);
                    // a circle through a transform that stretches stays round
                    PathBuilder::from_circle(cx, cy, n("r")?.max(0.0))
                } else {
                    let pts = match get(op, "points") {
                        Some(Value::List(l)) => l.iter().filter_map(nums).collect::<Vec<_>>(),
                        _ => vec![],
                    };
                    let mut pb = PathBuilder::new();
                    let mut first = true;
                    for p in pts.iter().filter(|p| p.len() >= 2) {
                        let (x, y) = f.pt(p[0], p[1]);
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
                let s = if stroked {
                    line_c.map(|c| (c, width))
                } else {
                    None
                };
                if raster
                    && (fill.is_some() || s.is_some())
                    && let Some((px, py, pic)) = raster_piece(&path, fill, s, scale)
                {
                    out.items.push(Item::Pic {
                        x: px,
                        y: py,
                        w: pic.width as f32 / scale,
                        h: pic.height as f32 / scale,
                        pic,
                        radii: [0.0; 4],
                        clip: f.clip,
                    });
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

//! Pictures: a `canvas` node's drawing and an `image` node's file, as
//! images both renderers draw.
//!
//! A canvas is a list of operations (rectangles, lines, paths, circles)
//! rasterized with tiny-skia at the window's scale and cached by what it
//! draws; its text operations are drawn as text over it by the window, so
//! they stay sharp and are measured like any other text. An image's
//! file is decoded by picture.rs.

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

/// A canvas's text, drawn by the window over the picture.
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
}

fn color_of(op: &Value, key: &str, what: &str) -> Res<Option<Color>> {
    get_color(op, key, what)
}

fn paint(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn stroke(w: f32) -> Stroke {
    Stroke {
        width: w.max(0.0),
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    }
}

fn hash_value(v: &Value, extra: (u32, u32, u32)) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{v}").hash(&mut h);
    extra.hash(&mut h);
    h.finish()
}

/// A canvas's drawing and its text.
pub type Drawing = (Option<Arc<Picture>>, Vec<CanvasText>);

/// The drawing of a canvas of `w × h` logical pixels at `scale`, and its
/// text. Cached by content: the same operations at the same size answer
/// the same picture.
pub fn draw(ops: &Value, w: f32, h: f32, scale: f32) -> Res<Drawing> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Drawing>>> = OnceLock::new();
    let pw = (w * scale).round().max(1.0) as u32;
    let ph = (h * scale).round().max(1.0) as u32;
    let key = hash_value(ops, (pw, ph, (scale * 100.0) as u32));
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Ok(hit);
    }
    let items: &[Value] = match ops {
        Value::List(l) => l,
        _ => return Err("canvas: \"draw\" must be a list of operations".into()),
    };
    let mut pm = Pixmap::new(pw.min(8192), ph.min(8192)).ok_or("canvas: too large")?;
    let mut texts = Vec::new();
    let mut drew = false;
    let tf = Transform::from_scale(scale, scale);
    for (i, op) in items.iter().enumerate() {
        let what = format!("canvas: operation {}", i + 1);
        let kind = get_str(op, "op", &what)?.ok_or_else(|| format!("{what}: needs an \"op\""))?;
        let n = |k: &str| -> Res<f32> { Ok(get_num(op, k, &what)?.unwrap_or(0.0)) };
        let fill = color_of(op, "fill", &what)?;
        let stroke_c = color_of(op, "stroke", &what)?.or(color_of(op, "color", &what)?);
        let width = get_num(op, "width", &what)?.unwrap_or(1.0);
        let path = match kind {
            "rect" => {
                let r = n("radius")?;
                super::soft::rrect_path(n("x")?, n("y")?, n("w")?, n("h")?, [r; 4])
            }
            "line" => {
                let mut pb = PathBuilder::new();
                pb.move_to(n("x1")?, n("y1")?);
                pb.line_to(n("x2")?, n("y2")?);
                pb.finish()
            }
            "circle" => PathBuilder::from_circle(n("cx")?, n("cy")?, n("r")?.max(0.0)),
            "path" => {
                let pts = match get(op, "points") {
                    Some(Value::List(l)) => l.iter().filter_map(nums).collect::<Vec<_>>(),
                    _ => vec![],
                };
                let mut pb = PathBuilder::new();
                for (j, p) in pts.iter().enumerate() {
                    if p.len() < 2 {
                        continue;
                    }
                    if j == 0 {
                        pb.move_to(p[0], p[1]);
                    } else {
                        pb.line_to(p[0], p[1]);
                    }
                }
                if get_bool(op, "close", &what)?.unwrap_or(false) {
                    pb.close();
                }
                pb.finish()
            }
            "text" => {
                texts.push(CanvasText {
                    x: n("x")?,
                    y: n("y")?,
                    text: get_str(op, "text", &what)?.unwrap_or("").to_string(),
                    size: get_num(op, "size", &what)?.unwrap_or(12.0),
                    weight: get_num(op, "weight", &what)?.unwrap_or(400.0),
                    color: stroke_c.or(fill).unwrap_or([24, 24, 27, 255]),
                    align: match get_str(op, "align", &what)?.unwrap_or("start") {
                        "center" => 1,
                        "end" => 2,
                        _ => 0,
                    },
                });
                continue;
            }
            other => {
                return Err(format!(
                    "{what}: unknown op \"{other}\" (rect, line, circle, path, text)"
                ));
            }
        };
        let Some(path) = path else { continue };
        // A line has no inside: its colour is its stroke.
        if kind != "line"
            && let Some(f) = fill
        {
            pm.fill_path(&path, &paint(f), FillRule::Winding, tf, None);
            drew = true;
        }
        if let Some(c) = stroke_c.filter(|_| kind == "line" || get(op, "stroke").is_some()) {
            pm.stroke_path(&path, &paint(c), &stroke(width), tf, None);
            drew = true;
        }
    }
    let pic = drew.then(|| {
        Arc::new(Picture {
            id: NEXT.fetch_add(1, Ordering::SeqCst),
            width: pm.width(),
            height: pm.height(),
            rgba: pm.data().to_vec(),
        })
    });
    let out = (pic, texts);
    if let Ok(mut c) = cache.lock() {
        if c.len() > 256 {
            c.clear();
        }
        c.insert(key, out.clone());
    }
    Ok(out)
}

/// A fresh picture id (the GPU keys a picture's texture by it).
pub fn next_picture_id() -> u64 {
    NEXT.fetch_add(1, Ordering::SeqCst)
}

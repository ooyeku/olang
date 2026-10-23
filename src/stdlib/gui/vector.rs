//! A canvas's vector paths made ready for the GPU (loom/ROADMAP.md item
//! 11): what gpu.rs draws into its coverage atlas instead of a picture
//! rasterized on the CPU.
//!
//! - **Fills** are triangle fans per contour (curves flattened to within
//!   a tenth of a device pixel), counted into a stencil (non-zero or
//!   even-odd) and covered, multisampled.
//! - **Strokes with round joins** (a chart's lines: the canvas's default)
//!   are segments drawn by their distance: each a capsule, or a box at a
//!   butt or square end, its coverage exact across the line (a hairline's
//!   faint as tiny-skia's is), the segments' coverages joined by their
//!   maximum, so a join is never drawn twice.
//! - **Other strokes** (miter or bevel joins) are tiny-skia's own stroke
//!   outline, filled as above: the same joins, caps and dashes as the
//!   software renderer, by construction.
//!
//! Dashes are tiny-skia's (`Path::dash`) in both renderers. Everything
//! here is in device pixels relative to the path's own origin (its points
//! times the scale); gpu.rs moves it to the atlas.

use super::canvas::{Cap, Join, StrokeSpec};
use super::window::PathPrim;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tiny_skia::{PathSegment, Point};

/// How far a flattened curve may stray from the curve, in device pixels.
pub const TOLERANCE: f32 = 0.1;

/// A stroke's segment drawn by its distance: from `a` to `b`, half its
/// width `hw`, each end's cap (`0` butt, `1` round, `2` square, `3` a
/// join: round, as the next segment meets it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub hw: f32,
    pub caps: [u8; 2],
}

/// A path's geometry for the GPU: fan triangles (three points each) whose
/// stencil count is the fill, and segments drawn by their distance.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub fan: Vec<[f32; 2]>,
    pub even_odd: bool,
    pub segs: Vec<Seg>,
}

impl Mesh {
    fn bytes(&self) -> usize {
        self.fan.len() * 8 + self.segs.len() * std::mem::size_of::<Seg>()
    }
}

/// What of a path a mesh draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Fill,
    Stroke,
}

/// The polylines of a path at `scale` (curves flattened), each with
/// whether it closes.
pub fn flatten(path: &tiny_skia::Path, scale: f32) -> Vec<(Vec<[f32; 2]>, bool)> {
    let mut out: Vec<(Vec<[f32; 2]>, bool)> = Vec::new();
    let mut cur: Vec<[f32; 2]> = Vec::new();
    let mut last = Point::zero();
    let p = |q: Point| [q.x * scale, q.y * scale];
    let flush = |cur: &mut Vec<[f32; 2]>, out: &mut Vec<(Vec<[f32; 2]>, bool)>, closed: bool| {
        if cur.len() >= 2 {
            out.push((std::mem::take(cur), closed));
        } else {
            cur.clear();
        }
    };
    for seg in path.segments() {
        match seg {
            PathSegment::MoveTo(q) => {
                flush(&mut cur, &mut out, false);
                cur.push(p(q));
                last = q;
            }
            PathSegment::LineTo(q) => {
                if cur.is_empty() {
                    cur.push(p(last));
                }
                cur.push(p(q));
                last = q;
            }
            PathSegment::QuadTo(c, q) => {
                if cur.is_empty() {
                    cur.push(p(last));
                }
                let (a, c, b) = (p(last), p(c), p(q));
                let dd = len([a[0] - 2.0 * c[0] + b[0], a[1] - 2.0 * c[1] + b[1]]);
                let n = steps(dd * 2.0 / 8.0);
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    cur.push([
                        u * u * a[0] + 2.0 * u * t * c[0] + t * t * b[0],
                        u * u * a[1] + 2.0 * u * t * c[1] + t * t * b[1],
                    ]);
                }
                last = q;
            }
            PathSegment::CubicTo(c1, c2, q) => {
                if cur.is_empty() {
                    cur.push(p(last));
                }
                let (a, c, d, b) = (p(last), p(c1), p(c2), p(q));
                let dd = len([a[0] - 2.0 * c[0] + d[0], a[1] - 2.0 * c[1] + d[1]])
                    .max(len([c[0] - 2.0 * d[0] + b[0], c[1] - 2.0 * d[1] + b[1]]));
                let n = steps(dd * 6.0 / 8.0);
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    cur.push([
                        w0 * a[0] + w1 * c[0] + w2 * d[0] + w3 * b[0],
                        w0 * a[1] + w1 * c[1] + w2 * d[1] + w3 * b[1],
                    ]);
                }
                last = q;
            }
            PathSegment::Close => {
                if let Some(first) = cur.first().copied() {
                    last = Point::from_xy(first[0] / scale, first[1] / scale);
                }
                flush(&mut cur, &mut out, true);
            }
        }
    }
    flush(&mut cur, &mut out, false);
    out
}

/// Wang's formula: the steps that flatten a curve whose control polygon
/// bends by `k` (its degree's factor taken) to [`TOLERANCE`].
fn steps(k: f32) -> usize {
    if !k.is_finite() {
        return 1;
    }
    ((k / TOLERANCE).sqrt().ceil() as usize).clamp(1, 256)
}

fn len(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// A fill: fan triangles from each contour's first point (closed or not:
/// a fill closes every contour).
pub fn fill_mesh(path: &tiny_skia::Path, scale: f32, even_odd: bool) -> Mesh {
    let mut fan = Vec::new();
    for (pts, _) in flatten(path, scale) {
        if pts.len() < 3 {
            continue;
        }
        let o = pts[0];
        for w in pts[1..].windows(2) {
            fan.extend_from_slice(&[o, w[0], w[1]]);
        }
    }
    Mesh {
        fan,
        even_odd,
        segs: Vec::new(),
    }
}

/// A stroke at `scale`: segments drawn by their distance when its joins
/// are round, else tiny-skia's outline as a fill. Dashes are tiny-skia's.
pub fn stroke_mesh(path: &tiny_skia::Path, scale: f32, st: &StrokeSpec) -> Mesh {
    let res = scale.max(0.01);
    let dashed;
    let path = match st.skia().dash {
        Some(d) => match path.dash(&d, res) {
            Some(p) => {
                dashed = p;
                &dashed
            }
            None => return Mesh::default(),
        },
        None => path,
    };
    if st.join == Join::Round {
        let hw = st.width * scale / 2.0;
        let cap = match st.cap {
            Cap::Butt => 0,
            Cap::Round => 1,
            Cap::Square => 2,
        };
        let mut segs = Vec::new();
        for (mut pts, closed) in flatten(path, scale) {
            pts.dedup();
            // a flattened curve's last steps shorter than the stroke's half
            // width are one step: a join's round end so near a butt or
            // square end would reach past it (tiny-skia strokes the curve)
            if !closed && cap != 1 {
                while pts.len() > 2
                    && len([
                        pts[pts.len() - 1][0] - pts[pts.len() - 2][0],
                        pts[pts.len() - 1][1] - pts[pts.len() - 2][1],
                    ]) < hw
                {
                    let n = pts.len();
                    pts.remove(n - 2);
                }
                while pts.len() > 2 && len([pts[1][0] - pts[0][0], pts[1][1] - pts[0][1]]) < hw {
                    pts.remove(1);
                }
            }
            if closed && pts.len() >= 2 && pts.first() != pts.last() {
                let f = pts[0];
                pts.push(f);
            }
            if pts.len() < 2 {
                // a dot: a round or square cap draws it, a butt one nothing
                if cap != 0
                    && let Some(p) = pts.first()
                {
                    segs.push(Seg {
                        a: *p,
                        b: *p,
                        hw,
                        caps: [cap, cap],
                    });
                }
                continue;
            }
            let n = pts.len() - 1;
            for i in 0..n {
                let start = if i == 0 && !closed { cap } else { 3 };
                let end = if i == n - 1 && !closed { cap } else { 3 };
                segs.push(Seg {
                    a: pts[i],
                    b: pts[i + 1],
                    hw,
                    caps: [start, end],
                });
            }
        }
        return Mesh {
            fan: Vec::new(),
            even_odd: false,
            segs,
        };
    }
    let mut plain = st.skia();
    plain.dash = None;
    match path.stroke(&plain, res) {
        Some(outline) => fill_mesh(&outline, scale, false),
        None => Mesh::default(),
    }
}

/// Meshes made lately, by the path's identity, its scale and its part: a
/// path drawn again (its window's next frame, a damaged rectangle, a
/// whole-pixel move) is not flattened again. The least recent half goes
/// past a budget.
#[derive(Default)]
struct Meshes {
    map: HashMap<(u64, u32, Part), (Arc<Mesh>, u64)>,
    bytes: usize,
    tick: u64,
}

const MESHES_BYTES: usize = 32 << 20;

fn meshes() -> &'static Mutex<Meshes> {
    static M: OnceLock<Mutex<Meshes>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(Meshes::default()))
}

/// A path's mesh for `part`, made once.
pub fn mesh(pp: &PathPrim, part: Part) -> Arc<Mesh> {
    let key = (pp.ident, pp.scale.to_bits(), part);
    if let Ok(mut m) = meshes().lock() {
        m.tick += 1;
        let now = m.tick;
        if let Some(e) = m.map.get_mut(&key) {
            e.1 = now;
            return e.0.clone();
        }
    }
    let made = Arc::new(match (part, &pp.stroke) {
        (Part::Fill, _) => fill_mesh(&pp.path, pp.scale, pp.even_odd),
        (Part::Stroke, Some(st)) => stroke_mesh(&pp.path, pp.scale, st),
        (Part::Stroke, None) => Mesh::default(),
    });
    if let Ok(mut m) = meshes().lock() {
        let size = made.bytes();
        if m.bytes + size > MESHES_BYTES && !m.map.is_empty() {
            let mut ticks: Vec<u64> = m.map.values().map(|v| v.1).collect();
            let half = ticks.len() / 2;
            let cut = *ticks.select_nth_unstable(half).1;
            let mut freed = 0;
            m.map.retain(|_, v| {
                let keep = v.1 > cut;
                if !keep {
                    freed += v.0.bytes();
                }
                keep
            });
            m.bytes -= freed;
        }
        if size <= MESHES_BYTES {
            let now = m.tick;
            m.bytes += size;
            if let Some((old, _)) = m.map.insert(key, (made.clone(), now)) {
                m.bytes -= old.bytes();
            }
        }
    }
    made
}

/// Rectangles packed in shelves on pages of `side` × `side`.
pub struct Shelves {
    pub side: u32,
    /// Per page: shelves `(y, height, x)`, and the extent used.
    pages: Vec<(Vec<(u32, u32, u32)>, u32, u32)>,
}

impl Shelves {
    pub fn new(side: u32) -> Shelves {
        Shelves {
            side: side.max(16),
            pages: Vec::new(),
        }
    }

    /// Room for `w` × `h` (cut to a page): its page and place.
    pub fn place(&mut self, w: u32, h: u32) -> (usize, u32, u32) {
        let (w, h) = (w.clamp(1, self.side), h.clamp(1, self.side));
        for (i, (shelves, used_w, used_h)) in self.pages.iter_mut().enumerate() {
            for s in shelves.iter_mut() {
                if h <= s.1 && s.2 + w <= self.side {
                    let at = (i, s.2, s.0);
                    s.2 += w + 1;
                    *used_w = (*used_w).max(at.1 + w);
                    return at;
                }
            }
            let y = shelves.last().map(|s| s.0 + s.1 + 1).unwrap_or(0);
            if y + h <= self.side {
                shelves.push((y, h, w + 1));
                *used_w = (*used_w).max(w);
                *used_h = (*used_h).max(y + h);
                return (i, 0, y);
            }
        }
        self.pages.push((vec![(0, h, w + 1)], w, h));
        (self.pages.len() - 1, 0, 0)
    }

    /// Each page's extent used, `(w, h)`.
    pub fn extents(&self) -> Vec<(u32, u32)> {
        self.pages
            .iter()
            .map(|p| (p.1.max(1), p.2.max(1)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::PathBuilder;

    #[test]
    fn a_curve_flattens_within_the_tolerance() {
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, 0.0);
        pb.quad_to(50.0, 100.0, 100.0, 0.0);
        let path = pb.finish().unwrap();
        let lines = flatten(&path, 2.0);
        assert_eq!(lines.len(), 1);
        let pts = &lines[0].0;
        assert!(
            pts.len() > 8,
            "a bend this large takes steps: {}",
            pts.len()
        );
        // every step's middle within the tolerance of the curve
        for w in pts.windows(2) {
            let m = [(w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0];
            // the curve y(x) at scale 2: x = 200t, y = 2·(200 t (1 - t))
            let t = m[0] / 200.0;
            let y = 400.0 * t * (1.0 - t);
            assert!(
                (m[1] - y).abs() < 4.0 * TOLERANCE + 0.05,
                "{m:?} against {y}"
            );
        }
    }

    #[test]
    fn a_round_joined_stroke_is_segments_and_a_mitred_one_an_outline() {
        let mut pb = PathBuilder::new();
        pb.move_to(0.0, 0.0);
        pb.line_to(10.0, 0.0);
        pb.line_to(10.0, 10.0);
        let path = pb.finish().unwrap();
        let mut st = StrokeSpec {
            color: [0, 0, 0, 255],
            width: 2.0,
            cap: Cap::Butt,
            join: Join::Round,
            dash: None,
            dash_offset: 0.0,
        };
        let m = stroke_mesh(&path, 1.0, &st);
        assert_eq!(m.segs.len(), 2);
        assert_eq!(m.segs[0].caps, [0, 3]);
        assert_eq!(m.segs[1].caps, [3, 0]);
        assert!(m.fan.is_empty());
        st.join = Join::Miter;
        let m = stroke_mesh(&path, 1.0, &st);
        assert!(m.segs.is_empty() && m.fan.len() >= 6);
        st.join = Join::Round;
        st.dash = Some(Arc::from(vec![2.0f32, 2.0].into_boxed_slice()));
        let m = stroke_mesh(&path, 1.0, &st);
        assert!(m.segs.len() >= 5, "dashed into pieces: {}", m.segs.len());
    }

    #[test]
    fn shelves_pack_and_spill_onto_pages() {
        let mut s = Shelves::new(100);
        assert_eq!(s.place(60, 40), (0, 0, 0));
        assert_eq!(s.place(30, 20), (0, 61, 0));
        assert_eq!(s.place(60, 50), (0, 0, 41));
        assert_eq!(s.place(90, 30).0, 1);
        assert_eq!(s.extents()[0], (91, 91));
    }
}

//! Layout: the plot's box measured and divided — title, subtitle and
//! caption, legends, facet strips, axes — and every panel drawn: its
//! grid, its layers (clipped to it), its axes' labels.

use super::build::{Built, Vis, default_color};
use super::geom;
use super::scale::{NpMap, PosKind, Ramp, Trained};
use super::scene::{Anchor, AxisInfo, Item, LayerInfo, Paint, PanelInfo, Rgba, Role, Scene, Shape};
use super::text::{fit, width, wrap};
use super::theme::Theme;
use super::{Aes, p_num};
use std::sync::Arc;

/// A plot's height when the theme names none: taller with more rows of
/// panels.
pub fn default_height(f: &super::build::FacetLayout) -> f64 {
    if f.nrow <= 1 {
        if f.ncol >= 3 { 340.0 } else { 440.0 }
    } else {
        (150.0 * f.nrow as f64 + 130.0).min(1100.0)
    }
}

/// A panel's box and its two axes: data (transformed) to pixels.
#[derive(Clone, Debug)]
pub struct PanelCoord {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// the horizontal axis's trained scale (y's under flip)
    pub hs: Trained,
    pub vs: Trained,
    pub flip: bool,
}

impl PanelCoord {
    fn hx(&self, v: f64) -> f64 {
        self.x + (v - self.hs.lo) / (self.hs.hi - self.hs.lo).max(1e-300) * self.w
    }
    fn vy(&self, v: f64) -> f64 {
        self.y + self.h - (v - self.vs.lo) / (self.vs.hi - self.vs.lo).max(1e-300) * self.h
    }
    /// A data point (x, y) in pixels.
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        if self.flip {
            (self.hx(y), self.vy(x))
        } else {
            (self.hx(x), self.vy(y))
        }
    }
    /// A data rectangle in pixels: (left, top, w, h).
    pub fn rect(&self, x0: f64, x1: f64, y0: f64, y1: f64) -> (f64, f64, f64, f64) {
        let (a, b) = self.map(x0, y0);
        let (c, d) = self.map(x1, y1);
        (a.min(c), b.min(d), (c - a).abs(), (d - b).abs())
    }
    /// The x scale's ends (data) in this panel.
    pub fn x_range(&self) -> (f64, f64) {
        if self.flip {
            (self.vs.lo, self.vs.hi)
        } else {
            (self.hs.lo, self.hs.hi)
        }
    }
    pub fn y_range(&self) -> (f64, f64) {
        if self.flip {
            (self.hs.lo, self.hs.hi)
        } else {
            (self.vs.lo, self.vs.hi)
        }
    }
}

/// A point's radius in pixels for a ggplot size.
pub fn radius(size: f64) -> f64 {
    if size.is_finite() {
        (0.6 + size * 1.05).max(0.5)
    } else {
        2.2
    }
}

// ── legends ─────────────────────────────────────────────────────────
#[derive(Clone, Debug, Default)]
struct KeyVis {
    color: Option<Rgba>,
    fill: Option<Rgba>,
    size: Option<f64>,
    shape: Option<Shape>,
    alpha: Option<f64>,
    dash: Option<Option<Vec<f64>>>,
    /// a line's width in pixels (a linewidth scale's key)
    lw: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Glyph {
    Point,
    Line,
    Rect,
    Band,
    Text,
}

fn glyph_of(geom: &str) -> Glyph {
    match geom {
        "point" | "jitter" | "qq" | "pointrange" | "summary" => Glyph::Point,
        "line" | "path" | "step" | "freqpoly" | "ecdf" | "hline" | "vline" | "abline"
        | "qq_line" | "segment" | "errorbar" | "linerange" | "rug" => Glyph::Line,
        "smooth" => Glyph::Band,
        "text" | "label" => Glyph::Text,
        _ => Glyph::Rect,
    }
}

enum Guide {
    Keys {
        title: String,
        aes: Vec<Aes>,
        labels: Vec<String>,
        keys: Vec<KeyVis>,
        /// (glyph, the layer's own visuals for what the key leaves out)
        layers: Vec<(Glyph, KeyVis, f64)>,
    },
    Bar {
        title: String,
        ramp: Ramp,
        mid: Option<f64>,
        lo: f64,
        hi: f64,
        breaks: Vec<(f64, String)>,
    },
}

impl Guide {
    fn title(&self) -> &str {
        match self {
            Guide::Keys { title, .. } | Guide::Bar { title, .. } => title,
        }
    }
}

fn layer_defaults(vis: &Vis) -> KeyVis {
    // a colour another legend explains is no default for this one's keys
    KeyVis {
        color: vis.color.first().copied().filter(|_| !vis.color_mapped),
        fill: vis.fill.first().copied().filter(|_| !vis.fill_mapped),
        size: vis.size.first().copied(),
        shape: vis.shape.first().copied(),
        alpha: vis.alpha.first().copied().filter(|a| a.is_finite()),
        dash: vis.dash.first().cloned(),
        lw: None,
    }
}

fn guides(b: &Built) -> Vec<Guide> {
    let mut out: Vec<Guide> = Vec::new();
    for (aes, sc) in &b.np {
        if !sc.guide || b.theme.legend == "none" {
            continue;
        }
        let users: Vec<&super::build::LayerOut> = b
            .layers
            .iter()
            .filter(|l| l.legend && l.mapped.contains(aes) && l.frame.cols.contains_key(aes))
            .collect();
        if users.is_empty() {
            continue;
        }
        let title = b
            .labels
            .get(aes.name())
            .or_else(|| {
                if *aes == Aes::Color {
                    b.labels.get("colour")
                } else {
                    None
                }
            })
            .cloned()
            .or_else(|| sc.name.clone())
            .or_else(|| users.iter().find_map(|l| l.titles.get(aes).cloned()))
            .unwrap_or_default();
        let layers: Vec<(Glyph, KeyVis, f64)> = users
            .iter()
            .map(|l| {
                let mut g = glyph_of(&l.geom);
                if g == Glyph::Band && super::p_bool(&l.params, "se") == Some(false) {
                    g = Glyph::Line;
                }
                (g, layer_defaults(&l.vis), l.vis.linewidth)
            })
            .collect();
        match &sc.map {
            Some(NpMap::Discrete { labels, levels }) => {
                let keys: Vec<KeyVis> = (0..levels.len())
                    .map(|i| {
                        let c = Some(i as u32);
                        let mut k = KeyVis::default();
                        match aes {
                            Aes::Color => k.color = Some(sc.color_of(c, 0.0)),
                            Aes::Fill => k.fill = Some(sc.color_of(c, 0.0)),
                            Aes::Size => k.size = Some(sc.size_of(c, 0.0)),
                            Aes::Alpha => k.alpha = Some(sc.alpha_of(c, 0.0)),
                            Aes::Shape => k.shape = Some(sc.shape_of(c)),
                            Aes::Linetype => k.dash = Some(sc.dash_of(c)),
                            Aes::Linewidth => k.lw = Some(sc.linewidth_of(c, 0.0) * 2.8),
                            _ => {}
                        }
                        k
                    })
                    .collect();
                // merge with a guide of the same title and labels
                if let Some(Guide::Keys {
                    keys: ks,
                    aes: a2,
                    layers: ls,
                    ..
                }) = out.iter_mut().find(|g| match g {
                    Guide::Keys {
                        title: t,
                        labels: l,
                        ..
                    } => *t == title && l == labels,
                    _ => false,
                }) {
                    for (k, n) in ks.iter_mut().zip(keys) {
                        k.color = k.color.or(n.color);
                        k.fill = k.fill.or(n.fill);
                        k.size = k.size.or(n.size);
                        k.alpha = k.alpha.or(n.alpha);
                        k.shape = k.shape.or(n.shape);
                        if k.dash.is_none() {
                            k.dash = n.dash;
                        }
                        k.lw = k.lw.or(n.lw);
                    }
                    a2.push(*aes);
                    for l in layers {
                        if !ls.iter().any(|x| x.0 == l.0) {
                            ls.push(l);
                        }
                    }
                    continue;
                }
                out.push(Guide::Keys {
                    title,
                    aes: vec![*aes],
                    labels: labels.clone(),
                    keys,
                    layers,
                });
            }
            Some(NpMap::Continuous {
                lo,
                hi,
                mid,
                breaks,
                labels,
            }) => {
                if matches!(aes, Aes::Color | Aes::Fill) {
                    if let Some(r) = &sc.ramp {
                        out.push(Guide::Bar {
                            title,
                            ramp: r.clone(),
                            mid: *mid,
                            lo: *lo,
                            hi: *hi,
                            breaks: breaks.iter().cloned().zip(labels.iter().cloned()).collect(),
                        });
                    }
                } else {
                    let keys: Vec<KeyVis> = breaks
                        .iter()
                        .map(|v| {
                            let mut k = KeyVis::default();
                            match aes {
                                Aes::Size => k.size = Some(sc.size_of(None, *v)),
                                Aes::Alpha => k.alpha = Some(sc.alpha_of(None, *v)),
                                Aes::Linewidth => k.lw = Some(sc.linewidth_of(None, *v) * 2.8),
                                _ => {}
                            }
                            k
                        })
                        .collect();
                    out.push(Guide::Keys {
                        title,
                        aes: vec![*aes],
                        labels: labels.clone(),
                        keys,
                        layers,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

const KEY: f64 = 16.0;

/// A guide's size when laid out vertically (right) or flowing (bottom).
fn guide_size(g: &Guide, t: &Theme, horizontal: bool, max_w: f64) -> (f64, f64) {
    let ts = t.legend_title();
    let ls = t.legend_text();
    let title_w = width(g.title(), ts, true);
    let title_h = if g.title().is_empty() { 0.0 } else { ts * 1.6 };
    match g {
        Guide::Keys { labels, keys, .. } => {
            let row = key_box(keys).max(ls * 1.55);
            if horizontal {
                let items: f64 = labels
                    .iter()
                    .map(|l| key_box(keys) + 6.0 + width(l, ls, false) + 14.0)
                    .sum();
                let lines = (items / max_w.max(60.0)).ceil().max(1.0);
                (items.min(max_w).max(title_w), title_h + lines * row)
            } else {
                let lw = labels
                    .iter()
                    .map(|l| width(l, ls, false))
                    .fold(0.0, f64::max);
                (
                    (key_box(keys) + 6.0 + lw).max(title_w),
                    title_h + row * labels.len() as f64,
                )
            }
        }
        Guide::Bar { breaks, .. } => {
            let lw = breaks
                .iter()
                .map(|(_, l)| width(l, ls, false))
                .fold(0.0, f64::max);
            if horizontal {
                (160f64.max(title_w), title_h + 12.0 + ls * 1.6)
            } else {
                ((12.0 + 6.0 + lw).max(title_w), title_h + 128.0)
            }
        }
    }
}

/// A key's box: larger for large points.
fn key_box(keys: &[KeyVis]) -> f64 {
    keys.iter()
        .filter_map(|k| k.size)
        .map(|s| radius(s) * 2.0 + 4.0)
        .fold(KEY, f64::max)
        .min(30.0)
}

fn draw_key(
    items: &mut Vec<Item>,
    x: f64,
    y: f64,
    bx: f64,
    k: &KeyVis,
    layers: &[(Glyph, KeyVis, f64)],
    t: &Theme,
) {
    let cx = x + bx / 2.0;
    let cy = y + bx / 2.0;
    for (glyph, d, lw) in layers {
        let color = k.color.or(d.color).unwrap_or(t.app.role(Role::Ink2));
        let fill = k.fill.or(d.fill).or(k.color).unwrap_or(color);
        let alpha = k.alpha.or(d.alpha).unwrap_or(1.0);
        match glyph {
            Glyph::Point => {
                let r = radius(k.size.or(d.size).unwrap_or(1.5)).min(bx / 2.0);
                let shape = k.shape.or(d.shape).unwrap_or(Shape::Circle);
                items.push(Item::Points(super::scene::Points {
                    xs: vec![cx as f32],
                    ys: vec![cy as f32],
                    r: vec![r as f32],
                    fill: vec![0],
                    pal: vec![color.with_alpha(alpha)],
                    shape,
                    stroke_w: 1.2,
                    panel: u32::MAX,
                    tips: None,
                }));
            }
            Glyph::Line | Glyph::Band => {
                if *glyph == Glyph::Band {
                    items.push(Item::Rect {
                        x: x + 1.0,
                        y: y + 2.0,
                        w: bx - 2.0,
                        h: bx - 4.0,
                        fill: Some(Paint::Rgba(
                            k.fill
                                .or(k.color)
                                .unwrap_or(t.app.role(Role::Ink2))
                                .with_alpha(0.18),
                        )),
                        stroke: None,
                        sw: 0.0,
                        r: 2.0,
                        tip: None,
                    });
                }
                items.push(Item::Line {
                    x1: x + 1.0,
                    y1: cy,
                    x2: x + bx - 1.0,
                    y2: cy,
                    stroke: Paint::Rgba(color.with_alpha(alpha)),
                    sw: k.lw.unwrap_or(lw.clamp(1.2, 4.0)).min(bx / 2.0),
                    dash: k.dash.clone().or(d.dash.clone()).flatten(),
                });
            }
            Glyph::Rect => {
                let a = if alpha < 1.0 { alpha } else { 1.0 };
                items.push(Item::Rect {
                    x: x + 1.5,
                    y: y + 1.5,
                    w: bx - 3.0,
                    h: bx - 3.0,
                    fill: Some(Paint::Rgba(fill.with_alpha(a))),
                    stroke: if k.color.is_some() {
                        Some(Paint::Rgba(color))
                    } else {
                        None
                    },
                    sw: 1.0,
                    r: 3.0,
                    tip: None,
                });
            }
            Glyph::Text => items.push(Item::Text {
                x: cx,
                y: cy,
                text: "a".into(),
                size: t.legend_text() * 1.1,
                paint: Paint::Rgba(color.with_alpha(alpha)),
                anchor: Anchor::Middle,
                weight: 600,
            }),
        }
    }
}

fn draw_guide(
    items: &mut Vec<Item>,
    g: &Guide,
    x: f64,
    y: f64,
    t: &Theme,
    horizontal: bool,
    max_w: f64,
) {
    let ts = t.legend_title();
    let ls = t.legend_text();
    let mut yy = y;
    if !g.title().is_empty() {
        items.push(Item::Text {
            x,
            y: yy + ts * 0.6,
            text: g.title().to_string(),
            size: ts,
            paint: Paint::Role(Role::Ink),
            anchor: Anchor::Start,
            weight: 600,
        });
        yy += ts * 1.6;
    }
    match g {
        Guide::Keys {
            labels,
            keys,
            layers,
            ..
        } => {
            let bx = key_box(keys);
            let row = bx.max(ls * 1.55);
            let mut xx = x;
            for (i, l) in labels.iter().enumerate() {
                let iw = bx + 6.0 + width(l, ls, false) + 14.0;
                if horizontal && xx + iw > x + max_w && xx > x {
                    xx = x;
                    yy += row;
                }
                draw_key(items, xx, yy + (row - bx) / 2.0, bx, &keys[i], layers, t);
                items.push(Item::Text {
                    x: xx + bx + 6.0,
                    y: yy + row / 2.0,
                    text: l.clone(),
                    size: ls,
                    paint: Paint::Role(Role::Ink2),
                    anchor: Anchor::Start,
                    weight: 400,
                });
                if horizontal {
                    xx += iw;
                } else {
                    yy += row;
                }
            }
        }
        Guide::Bar {
            ramp,
            mid,
            lo,
            hi,
            breaks,
            ..
        } => {
            let unit = |v: f64| -> f64 {
                match mid {
                    Some(m) => {
                        let r = (hi - m).abs().max((m - lo).abs()).max(1e-12);
                        0.5 + (v - m) / r / 2.0
                    }
                    None => (v - lo) / (hi - lo).max(1e-12),
                }
            };
            let steps = 32;
            if horizontal {
                let (bw, bh) = (160.0, 10.0);
                for s in 0..steps {
                    let t0 = s as f64 / steps as f64;
                    let v = lo + (hi - lo) * (t0 + 0.5 / steps as f64);
                    items.push(Item::Rect {
                        x: x + t0 * bw,
                        y: yy,
                        w: bw / steps as f64 + 0.4,
                        h: bh,
                        fill: Some(Paint::Rgba(ramp.at(unit(v)))),
                        stroke: None,
                        sw: 0.0,
                        r: 0.0,
                        tip: None,
                    });
                }
                for (v, l) in breaks {
                    let px = x + (v - lo) / (hi - lo).max(1e-12) * bw;
                    items.push(Item::Line {
                        x1: px,
                        y1: yy + bh - 3.0,
                        x2: px,
                        y2: yy + bh,
                        stroke: Paint::Role(Role::Surface),
                        sw: 1.0,
                        dash: None,
                    });
                    items.push(Item::Text {
                        x: px,
                        y: yy + bh + ls * 0.9,
                        text: l.clone(),
                        size: ls,
                        paint: Paint::Role(Role::Ink2),
                        anchor: Anchor::Middle,
                        weight: 400,
                    });
                }
            } else {
                let (bw, bh) = (12.0, 120.0);
                for s in 0..steps {
                    let t0 = s as f64 / steps as f64;
                    let v = hi - (hi - lo) * (t0 + 0.5 / steps as f64);
                    items.push(Item::Rect {
                        x,
                        y: yy + t0 * bh,
                        w: bw,
                        h: bh / steps as f64 + 0.4,
                        fill: Some(Paint::Rgba(ramp.at(unit(v)))),
                        stroke: None,
                        sw: 0.0,
                        r: 0.0,
                        tip: None,
                    });
                }
                for (v, l) in breaks {
                    let py = yy + (hi - v) / (hi - lo).max(1e-12) * bh;
                    items.push(Item::Line {
                        x1: x + bw - 3.0,
                        y1: py,
                        x2: x + bw,
                        y2: py,
                        stroke: Paint::Role(Role::Surface),
                        sw: 1.0,
                        dash: None,
                    });
                    items.push(Item::Text {
                        x: x + bw + 6.0,
                        y: py,
                        text: l.clone(),
                        size: ls,
                        paint: Paint::Role(Role::Ink2),
                        anchor: Anchor::Start,
                        weight: 400,
                    });
                }
            }
        }
    }
}

// ── axes ────────────────────────────────────────────────────────────
/// Which of a facet's labels to keep so none overlap: every k-th.
fn thin_labels(pos: &[f64], labels: &[String], size: f64, horizontal: bool) -> usize {
    if !horizontal || labels.len() < 2 {
        return 1;
    }
    for k in 1..labels.len() {
        let mut ok = true;
        let mut last_right = f64::NEG_INFINITY;
        for i in (0..labels.len()).step_by(k) {
            let w = width(&labels[i], size, false);
            let left = pos[i] - w / 2.0;
            if left < last_right + 8.0 {
                ok = false;
                break;
            }
            last_right = pos[i] + w / 2.0;
        }
        if ok {
            return k;
        }
    }
    labels.len()
}

fn axis_info(s: &super::scale::PosScale, t: &Trained, title: &str) -> AxisInfo {
    AxisInfo {
        kind: s.kind_name(),
        trans: s.trans.name(),
        lo: t.lo,
        hi: t.hi,
        levels: if s.kind == PosKind::Discrete {
            s.levels.clone()
        } else {
            vec![]
        },
        title: title.to_string(),
    }
}

// ── the plot ────────────────────────────────────────────────────────
pub fn draw(b: &Built) -> super::Result<Scene> {
    let t = &b.theme;
    let fct = &b.facet;
    let w_total = t.width.unwrap_or(720.0).clamp(120.0, 8000.0);
    let h_total = t.height.unwrap_or(default_height(fct)).clamp(100.0, 8000.0);
    let m = 14.0;
    let void = t.void();
    let mut items: Vec<Item> = Vec::new();

    // ── the frame's words ───────────────────────────────────────────
    let title = b.labels.get("title").cloned().unwrap_or_default();
    let subtitle = b.labels.get("subtitle").cloned().unwrap_or_default();
    let caption = b.labels.get("caption").cloned().unwrap_or_default();
    let tl = if title.is_empty() {
        vec![]
    } else {
        wrap(&title, t.title(), true, w_total - 2.0 * m)
    };
    let sl = if subtitle.is_empty() {
        vec![]
    } else {
        wrap(&subtitle, t.subtitle(), false, w_total - 2.0 * m)
    };
    let cl = if caption.is_empty() {
        vec![]
    } else {
        wrap(&caption, t.caption(), false, w_total - 2.0 * m)
    };
    let title_h = tl.len() as f64 * t.title() * 1.3
        + sl.len() as f64 * t.subtitle() * 1.45
        + if tl.is_empty() && sl.is_empty() {
            0.0
        } else {
            10.0
        };
    let caption_h = if cl.is_empty() {
        0.0
    } else {
        cl.len() as f64 * t.caption() * 1.45 + 6.0
    };

    // ── the horizontal and vertical axes' scales ────────────────────
    let (hscale, vscale) = if b.flip {
        (&b.sy, &b.sx)
    } else {
        (&b.sx, &b.sy)
    };
    let (htrained, vtrained) = if b.flip {
        (&b.ty, &b.tx)
    } else {
        (&b.tx, &b.ty)
    };
    let (htitle, vtitle) = if b.flip {
        (b.y_title.clone(), b.x_title.clone())
    } else {
        (b.x_title.clone(), b.y_title.clone())
    };

    // ── legends ─────────────────────────────────────────────────────
    let gs = guides(b);
    let pos = t.legend.as_str();
    let horizontal = pos == "bottom" || pos == "top";
    let mut legend_w = 0.0;
    let mut legend_h = 0.0;
    let gsizes: Vec<(f64, f64)> = gs
        .iter()
        .map(|g| guide_size(g, t, horizontal, w_total - 2.0 * m))
        .collect();
    if !gs.is_empty() && pos != "none" {
        if horizontal {
            legend_h = gsizes.iter().map(|s| s.1 + 10.0).sum::<f64>() + 6.0;
        } else {
            legend_w = gsizes.iter().map(|s| s.0).fold(0.0, f64::max) + 18.0;
        }
    }

    // ── axes' room ──────────────────────────────────────────────────
    let ts = t.tick();
    let any_h_breaks = !void && htrained.iter().any(|tr| !tr.breaks.is_empty());
    let any_v_breaks = !void && vtrained.iter().any(|tr| !tr.breaks.is_empty());
    let xtick_h = if any_h_breaks {
        ts * 1.5 + if t.ticks { 4.0 } else { 3.0 }
    } else {
        0.0
    };
    let ytick_w = if any_v_breaks {
        vtrained
            .iter()
            .flat_map(|tr| tr.labels.iter())
            .map(|l| width(&fit(l, ts, false, 160.0), ts, false))
            .fold(0.0, f64::max)
            + if t.ticks { 10.0 } else { 7.0 }
    } else {
        0.0
    };
    let xtitle_h = if htitle.is_empty() || void {
        0.0
    } else {
        t.axis_title() * 1.7
    };
    let ytitle_h = if vtitle.is_empty() || void {
        0.0
    } else {
        t.axis_title() * 1.65
    };
    let strip_h = if fct.kind == "wrap" || !fct.col_labels.is_empty() {
        t.strip() * 1.85
    } else {
        0.0
    };
    let rowstrip_w = if fct.kind == "grid" && !fct.row_labels.is_empty() {
        fct.row_labels
            .iter()
            .map(|l| width(l, t.strip(), true))
            .fold(0.0, f64::max)
            .min(140.0)
            + 14.0
    } else {
        0.0
    };
    let per_panel_y = fct.kind == "wrap" && (if b.flip { fct.free_x } else { fct.free_y });
    let per_panel_x = fct.kind == "wrap" && (if b.flip { fct.free_y } else { fct.free_x });
    let gap = if fct.n > 1 { 12.0 } else { 0.0 };
    let hgap = gap + if per_panel_y { ytick_w } else { 0.0 };
    let vgap = gap + if per_panel_x { xtick_h } else { 0.0 };

    let left0 = m + ytick_w + if pos == "left" { legend_w } else { 0.0 };
    let right0 = w_total - m - rowstrip_w - if pos == "right" { legend_w } else { 0.0 };
    let top0 = m + title_h + ytitle_h + if pos == "top" { legend_h } else { 0.0 };
    let bottom0 =
        h_total - m - caption_h - xtitle_h - xtick_h - if pos == "bottom" { legend_h } else { 0.0 };
    let (nr, nc) = (fct.nrow as f64, fct.ncol as f64);
    let mut pw = ((right0 - left0 - (nc - 1.0) * hgap) / nc).max(10.0);
    let mut ph = ((bottom0 - top0 - nr * strip_h - (nr - 1.0) * vgap) / nr).max(10.0);
    let (mut ox, mut oy) = (0.0, 0.0);
    if let Some(r) = b.ratio {
        // a fixed aspect: the data's units the same length on both axes (× ratio)
        let hs = (htrained[0].hi - htrained[0].lo).max(1e-300);
        let vs = (vtrained[0].hi - vtrained[0].lo).max(1e-300);
        let want = r * vs / hs; // ph / pw
        if ph > pw * want {
            let nph = pw * want;
            oy = (ph - nph) * nr / 2.0;
            ph = nph;
        } else {
            let npw = ph / want;
            ox = (pw - npw) * nc / 2.0;
            pw = npw;
        }
    }

    // the panels' boxes
    let mut coords: Vec<PanelCoord> = Vec::with_capacity(fct.n);
    for p in 0..fct.n {
        let (r, c) = fct.pos[p];
        let x = left0 + ox + c as f64 * (pw + hgap);
        let y = top0 + oy + strip_h + r as f64 * (ph + strip_h + vgap);
        coords.push(PanelCoord {
            x,
            y,
            w: pw,
            h: ph,
            hs: htrained[p].clone(),
            vs: vtrained[p].clone(),
            flip: b.flip,
        });
    }
    // which panels show an axis: the bottom of each column, the first column
    let shows_h = |p: usize| -> bool {
        if per_panel_x {
            return true;
        }
        let (r, c) = fct.pos[p];
        !(0..fct.n).any(|q| fct.pos[q].1 == c && fct.pos[q].0 > r)
    };
    let shows_v = |p: usize| -> bool { per_panel_y || fct.pos[p].1 == 0 };

    let mut panels_info = Vec::with_capacity(fct.n);
    let mut layers_info: Vec<LayerInfo> = b
        .layers
        .iter()
        .map(|l| LayerInfo {
            geom: l.geom.clone(),
            fields: Arc::new(l.fields.iter().map(|f| f.0.clone()).collect()),
        })
        .collect();
    let hdisc = hscale.kind == PosKind::Discrete;
    let vdisc = vscale.kind == PosKind::Discrete;

    for (p, pc) in coords.iter().enumerate() {
        // background, grid
        if t.panel_fill {
            items.push(Item::Rect {
                x: pc.x,
                y: pc.y,
                w: pc.w,
                h: pc.h,
                fill: Some(Paint::Role(Role::Panel)),
                stroke: None,
                sw: 0.0,
                r: 0.0,
                tip: None,
            });
        }
        if !void {
            if t.grid_x() && !hdisc {
                for v in &pc.hs.breaks {
                    let x = (pc.hx(*v)).round() + 0.5;
                    items.push(Item::Line {
                        x1: x,
                        y1: pc.y,
                        x2: x,
                        y2: pc.y + pc.h,
                        stroke: Paint::Role(Role::Grid),
                        sw: 1.0,
                        dash: None,
                    });
                }
            }
            if t.grid_y() && !vdisc {
                for v in &pc.vs.breaks {
                    let y = (pc.vy(*v)).round() + 0.5;
                    items.push(Item::Line {
                        x1: pc.x,
                        y1: y,
                        x2: pc.x + pc.w,
                        y2: y,
                        stroke: Paint::Role(Role::Grid),
                        sw: 1.0,
                        dash: None,
                    });
                }
            }
        }
        // the layers, clipped to the panel
        items.push(Item::Clip {
            x: pc.x - 1.0,
            y: pc.y - 1.0,
            w: pc.w + 2.0,
            h: pc.h + 2.0,
        });
        for (li, l) in b.layers.iter().enumerate() {
            let rows: Vec<usize> = (0..l.frame.n)
                .filter(|&i| l.frame.panel[i] as usize == p)
                .collect();
            if rows.is_empty() {
                continue;
            }
            geom::draw(b, l, li as u32, p as u32, &rows, pc, &mut items)?;
        }
        items.push(Item::Unclip);
        // frame lines
        if t.panel_border {
            items.push(Item::Rect {
                x: pc.x,
                y: pc.y,
                w: pc.w,
                h: pc.h,
                fill: None,
                stroke: Some(Paint::Role(Role::Axis)),
                sw: 1.0,
                r: 0.0,
                tip: None,
            });
        }
        if t.axis_lines {
            let yb = pc.y + pc.h;
            items.push(Item::Line {
                x1: pc.x,
                y1: yb,
                x2: pc.x + pc.w,
                y2: yb,
                stroke: Paint::Role(Role::Axis),
                sw: 1.0,
                dash: None,
            });
            items.push(Item::Line {
                x1: pc.x,
                y1: pc.y,
                x2: pc.x,
                y2: yb,
                stroke: Paint::Role(Role::Axis),
                sw: 1.0,
                dash: None,
            });
        }
        // the horizontal axis's labels
        if !void && shows_h(p) && !pc.hs.breaks.is_empty() {
            let xs: Vec<f64> = pc.hs.breaks.iter().map(|v| pc.hx(*v)).collect();
            let every = thin_labels(&xs, &pc.hs.labels, ts, true);
            let slot = if hdisc && !xs.is_empty() {
                pc.w / xs.len() as f64 * every as f64 - 4.0
            } else {
                200.0
            };
            for (i, x) in xs.iter().enumerate() {
                if t.ticks {
                    items.push(Item::Line {
                        x1: *x,
                        y1: pc.y + pc.h,
                        x2: *x,
                        y2: pc.y + pc.h + 4.0,
                        stroke: Paint::Role(Role::Axis),
                        sw: 1.0,
                        dash: None,
                    });
                }
                if i % every != 0 {
                    continue;
                }
                // side by side, a label past its panel's half of the gap would meet its neighbour's
                let half = width(&pc.hs.labels[i], ts, false) / 2.0;
                if fct.ncol > 1
                    && !hdisc
                    && (x - half < pc.x - hgap / 2.0 || x + half > pc.x + pc.w + hgap / 2.0)
                {
                    continue;
                }
                items.push(Item::Text {
                    x: *x,
                    y: pc.y + pc.h + (if t.ticks { 4.0 } else { 3.0 }) + ts * 0.75,
                    text: fit(&pc.hs.labels[i], ts, false, slot.max(24.0)),
                    size: ts,
                    paint: Paint::Role(Role::Ink2),
                    anchor: Anchor::Middle,
                    weight: 400,
                });
            }
        }
        // the vertical axis's labels
        if !void && shows_v(p) {
            let ys: Vec<f64> = pc.vs.breaks.iter().map(|v| pc.vy(*v)).collect();
            // a crowded discrete axis says every k-th
            let every = if ys.len() > 1 {
                let gap = (ys[0] - ys[1]).abs();
                ((ts * 1.2) / gap.max(1e-9)).ceil().max(1.0) as usize
            } else {
                1
            };
            for (i, y) in ys.iter().enumerate() {
                if t.ticks {
                    items.push(Item::Line {
                        x1: pc.x - 4.0,
                        y1: *y,
                        x2: pc.x,
                        y2: *y,
                        stroke: Paint::Role(Role::Axis),
                        sw: 1.0,
                        dash: None,
                    });
                }
                if i % every != 0 {
                    continue;
                }
                items.push(Item::Text {
                    x: pc.x - if t.ticks { 7.0 } else { 6.0 },
                    y: *y,
                    text: fit(&pc.vs.labels[i], ts, false, 160.0),
                    size: ts,
                    paint: Paint::Role(Role::Ink2),
                    anchor: Anchor::End,
                    weight: 400,
                });
            }
        }
        // strips
        let label = if fct.kind == "wrap" {
            fct.labels[p].clone()
        } else {
            String::new()
        };
        if fct.kind == "wrap" {
            strip(&mut items, t, pc.x, pc.y - strip_h, pc.w, strip_h, &label);
        } else if fct.kind == "grid" {
            let (r, c) = fct.pos[p];
            if r == 0 && !fct.col_labels.is_empty() {
                strip(
                    &mut items,
                    t,
                    pc.x,
                    pc.y - strip_h,
                    pc.w,
                    strip_h,
                    &fct.col_labels[c],
                );
            }
            if c + 1 == fct.ncol && !fct.row_labels.is_empty() {
                let lx = pc.x + pc.w + 8.0;
                if t.strip_fill {
                    items.push(Item::Rect {
                        x: pc.x + pc.w + 4.0,
                        y: pc.y,
                        w: rowstrip_w - 4.0,
                        h: pc.h,
                        fill: Some(Paint::Role(Role::Strip)),
                        stroke: None,
                        sw: 0.0,
                        r: 0.0,
                        tip: None,
                    });
                }
                items.push(Item::Text {
                    x: lx,
                    y: pc.y + pc.h / 2.0,
                    text: fit(&fct.row_labels[r], t.strip(), true, rowstrip_w - 12.0),
                    size: t.strip(),
                    paint: Paint::Role(Role::Ink),
                    anchor: Anchor::Start,
                    weight: 600,
                });
            }
        }
        let panel_label = match fct.kind.as_str() {
            "wrap" => label,
            "grid" => {
                let (r, c) = fct.pos[p];
                [
                    fct.row_labels.get(r).cloned(),
                    fct.col_labels.get(c).cloned(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ")
            }
            _ => String::new(),
        };
        panels_info.push(PanelInfo {
            x: pc.x,
            y: pc.y,
            w: pc.w,
            h: pc.h,
            h_axis: axis_info(hscale, &pc.hs, &htitle),
            v_axis: axis_info(vscale, &pc.vs, &vtitle),
            flip: b.flip,
            row: fct.pos[p].0,
            col: fct.pos[p].1,
            label: panel_label,
        });
    }

    // ── axis titles ─────────────────────────────────────────────────
    let plot_left = coords.iter().map(|c| c.x).fold(f64::INFINITY, f64::min);
    let plot_right = coords
        .iter()
        .map(|c| c.x + c.w)
        .fold(f64::NEG_INFINITY, f64::max);
    let plot_top = coords.iter().map(|c| c.y).fold(f64::INFINITY, f64::min) - strip_h;
    let plot_bottom = coords
        .iter()
        .map(|c| c.y + c.h)
        .fold(f64::NEG_INFINITY, f64::max);
    if xtitle_h > 0.0 {
        items.push(Item::Text {
            x: (plot_left + plot_right) / 2.0,
            y: plot_bottom + xtick_h + xtitle_h * 0.55,
            text: fit(&htitle, t.axis_title(), false, plot_right - plot_left),
            size: t.axis_title(),
            paint: Paint::Role(Role::Ink2),
            anchor: Anchor::Middle,
            weight: 400,
        });
    }
    if ytitle_h > 0.0 {
        // the vertical axis's name over its labels, read across
        items.push(Item::Text {
            x: (plot_left - ytick_w).max(m),
            y: plot_top - ytitle_h * 0.55,
            text: fit(&vtitle, t.axis_title(), false, w_total - 2.0 * m),
            size: t.axis_title(),
            paint: Paint::Role(Role::Ink2),
            anchor: Anchor::Start,
            weight: 400,
        });
    }

    // ── title, subtitle, caption ────────────────────────────────────
    let mut ty = m;
    for l in &tl {
        items.push(Item::Text {
            x: m,
            y: ty + t.title() * 0.62,
            text: l.clone(),
            size: t.title(),
            paint: Paint::Role(Role::Ink),
            anchor: Anchor::Start,
            weight: 600,
        });
        ty += t.title() * 1.3;
    }
    for l in &sl {
        items.push(Item::Text {
            x: m,
            y: ty + t.subtitle() * 0.7,
            text: l.clone(),
            size: t.subtitle(),
            paint: Paint::Role(Role::Ink2),
            anchor: Anchor::Start,
            weight: 400,
        });
        ty += t.subtitle() * 1.45;
    }
    let mut cy = h_total - m - caption_h + 6.0;
    for l in &cl {
        items.push(Item::Text {
            x: m,
            y: cy + t.caption() * 0.7,
            text: l.clone(),
            size: t.caption(),
            paint: Paint::Role(Role::Ink2),
            anchor: Anchor::Start,
            weight: 400,
        });
        cy += t.caption() * 1.45;
    }

    // ── legends ─────────────────────────────────────────────────────
    if pos != "none" && !gs.is_empty() {
        if horizontal {
            let mut y = if pos == "top" {
                m + title_h
            } else {
                h_total - m - caption_h - legend_h + 6.0
            };
            for (g, s) in gs.iter().zip(&gsizes) {
                draw_guide(
                    &mut items,
                    g,
                    plot_left.min(m + ytick_w),
                    y,
                    t,
                    true,
                    w_total - 2.0 * m,
                );
                y += s.1 + 10.0;
            }
        } else {
            let x = if pos == "left" {
                m
            } else {
                w_total - m - legend_w + 18.0
            };
            let total: f64 = gsizes.iter().map(|s| s.1 + 14.0).sum();
            let mut y = (plot_top + (plot_bottom - plot_top - total) / 2.0).max(plot_top);
            for (g, s) in gs.iter().zip(&gsizes) {
                draw_guide(&mut items, g, x, y, t, false, legend_w);
                y += s.1 + 14.0;
            }
        }
    }

    // a layer drawn nowhere still says its fields
    for l in layers_info.iter_mut() {
        if l.fields.is_empty() {
            l.fields = Arc::new(vec![]);
        }
    }
    let zoomable = hscale.kind != PosKind::Discrete;
    Ok(Scene {
        width: w_total,
        height: h_total,
        app: t.app,
        font: t.font.clone(),
        items,
        panels: panels_info,
        layers: layers_info,
        title: title.clone(),
        alt: alt_text(b, &title),
        zoomable,
        zoom: b.zoom,
        paper: t.paper.clone(),
        ink: t.ink.clone(),
        id: t.id.clone(),
    })
}

fn strip(items: &mut Vec<Item>, t: &Theme, x: f64, y: f64, w: f64, h: f64, label: &str) {
    if t.strip_fill {
        items.push(Item::Rect {
            x,
            y: y + 1.0,
            w,
            h: h - 3.0,
            fill: Some(Paint::Role(Role::Strip)),
            stroke: None,
            sw: 0.0,
            r: 0.0,
            tip: None,
        });
    }
    items.push(Item::Text {
        x: x + if t.strip_fill { 6.0 } else { 0.0 },
        y: y + h * 0.5,
        text: fit(label, t.strip(), true, w - 6.0),
        size: t.strip(),
        paint: Paint::Role(Role::Ink),
        anchor: Anchor::Start,
        weight: 600,
    });
}

fn alt_text(b: &Built, title: &str) -> String {
    let geoms: Vec<&str> = b.layers.iter().map(|l| l.geom.as_str()).collect();
    let kind = match geoms.first().copied().unwrap_or("") {
        "point" | "jitter" => "A scatter plot",
        "line" | "path" | "step" => "A line chart",
        "bar" | "col" => "A bar chart",
        "histogram" => "A histogram",
        "density" => "A density plot",
        "boxplot" => "A box plot",
        "violin" => "A violin plot",
        "tile" | "raster" => "A heatmap",
        "area" => "An area chart",
        "hex" | "bin2d" | "bin_2d" => "A 2-D histogram",
        "smooth" => "A smoothed trend",
        "ecdf" => "An empirical cdf",
        "qq" => "A Q-Q plot",
        _ => "A plot",
    };
    let mut s = kind.to_string();
    if !title.is_empty() {
        s.push_str(&format!(": {}", title));
    }
    if !b.y_title.is_empty() && !b.x_title.is_empty() {
        s.push_str(&format!(", {} by {}", b.y_title, b.x_title));
    }
    for (a, word) in [(Aes::Color, "coloured"), (Aes::Fill, "filled")] {
        if let Some(t) = b.layers.iter().find_map(|l| {
            if l.mapped.contains(&a) {
                l.titles.get(&a)
            } else {
                None
            }
        }) {
            s.push_str(&format!(", {} by {}", word, t));
            break;
        }
    }
    if b.facet.n > 1 {
        s.push_str(&format!(", in {} panels", b.facet.n));
    }
    let _ = default_color;
    let _ = p_num;
    s
}

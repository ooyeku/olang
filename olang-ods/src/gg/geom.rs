//! Geoms: a layer's rows in one panel drawn as scene items — points,
//! lines, areas, bars, boxes, violins, ranges, text, tiles, hexagons,
//! reference lines — each mark carrying its tooltip's values.

use super::build::{Built, LayerOut, tip_value};
use super::data::Col;
use super::layout::{PanelCoord, radius};
use super::scene::{Anchor, Item, Paint, Points, Rgba, Role, Shape, Tips};
use super::text::width;
use super::theme::Appearance;
use super::{Aes, p_bool, p_num, p_nums, p_str};
use std::collections::{BTreeMap, HashMap};

struct G<'a> {
    b: &'a Built,
    l: &'a LayerOut,
    layer: u32,
    panel: u32,
    pc: &'a PanelCoord,
}

impl G<'_> {
    fn num(&self, a: Aes, i: usize) -> f64 {
        self.l.frame.num(a).map(|v| v[i]).unwrap_or(f64::NAN)
    }
    fn has(&self, a: Aes) -> bool {
        self.l.frame.num(a).is_some()
    }
    fn alpha(&self, i: usize, dflt: f64) -> f64 {
        let a = self.l.vis.alpha[i];
        if a.is_finite() { a } else { dflt }
    }
    fn color(&self, i: usize) -> Rgba {
        self.l.vis.color[i]
    }
    fn fill(&self, i: usize) -> Rgba {
        self.l.vis.fill[i]
    }
    fn tip(&self, i: usize) -> Vec<String> {
        self.l
            .fields
            .iter()
            .map(|(_, s)| tip_value(self.b, self.l, s, i))
            .collect()
    }
    fn tips(&self, rows: &[usize]) -> Option<Tips> {
        if self.l.fields.is_empty() {
            return None;
        }
        let cols = self
            .l
            .fields
            .iter()
            .map(|(_, s)| {
                rows.iter()
                    .map(|&i| tip_value(self.b, self.l, s, i))
                    .collect()
            })
            .collect();
        Some(Tips {
            layer: self.layer,
            cols,
        })
    }
    /// A row's line width in pixels (a scale's, else the layer's).
    fn lw(&self, i: usize) -> f64 {
        match &self.l.vis.lws {
            Some(v) => v.get(i).copied().unwrap_or(self.l.vis.linewidth),
            None => self.l.vis.linewidth,
        }
    }
    fn param(&self, k: &str) -> Option<f64> {
        p_num(&self.l.params, k)
    }
    fn rect(
        &self,
        i: usize,
        x0: f64,
        x1: f64,
        y0: f64,
        y1: f64,
        fill: Option<Paint>,
        stroke: Option<Paint>,
        sw: f64,
    ) -> Option<Item> {
        // an infinite edge is the panel's: -inf its low end, inf its high
        let edge = |v: f64, (a, b): (f64, f64)| {
            if v == f64::NEG_INFINITY {
                a.min(b)
            } else if v == f64::INFINITY {
                a.max(b)
            } else {
                v
            }
        };
        let (x0, x1) = (edge(x0, self.pc.x_range()), edge(x1, self.pc.x_range()));
        let (y0, y1) = (edge(y0, self.pc.y_range()), edge(y1, self.pc.y_range()));
        if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (x, y, w, h) = self.pc.rect(x0, x1, y0, y1);
        Some(Item::Rect {
            x,
            y,
            w,
            h,
            fill,
            stroke,
            sw,
            r: 0.0,
            tip: if self.l.fields.is_empty() {
                None
            } else {
                Some((self.layer, self.tip(i)))
            },
        })
    }
    fn line(
        &self,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        c: Rgba,
        sw: f64,
        dash: Option<Vec<f64>>,
    ) -> Option<Item> {
        if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
            return None;
        }
        let (a, b) = self.pc.map(x0, y0);
        let (cx, d) = self.pc.map(x1, y1);
        Some(Item::Line {
            x1: a,
            y1: b,
            x2: cx,
            y2: d,
            stroke: Paint::Rgba(c),
            sw,
            dash,
        })
    }
}

/// Rows split by group, each group's rows in their order.
fn by_group(f: &super::data::LFrame, rows: &[usize]) -> Vec<Vec<usize>> {
    let mut m: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for &i in rows {
        m.entry(f.group[i]).or_default().push(i);
    }
    m.into_values().collect()
}

pub fn draw(
    b: &Built,
    l: &LayerOut,
    layer: u32,
    panel: u32,
    rows: &[usize],
    pc: &PanelCoord,
    items: &mut Vec<Item>,
) -> super::Result<()> {
    let g = G {
        b,
        l,
        layer,
        panel,
        pc,
    };
    match l.geom.as_str() {
        "point" | "jitter" | "qq" => points(&g, rows, items, None),
        "line" | "path" | "step" | "freqpoly" | "ecdf" | "qq_line" => lines(&g, rows, items),
        "area" => area(&g, rows, items, false),
        "density" => area(&g, rows, items, true),
        "ribbon" => ribbon(&g, rows, items),
        "smooth" => smooth(&g, rows, items),
        "bar" | "col" | "histogram" => bars(&g, rows, items),
        "boxplot" => boxplot(&g, rows, items),
        "violin" => violin(&g, rows, items),
        "errorbar" | "linerange" | "pointrange" | "crossbar" | "summary" => ranges(&g, rows, items),
        "text" | "label" => text(&g, rows, items),
        "tile" | "raster" | "rect" | "bin2d" | "bin_2d" => tiles(&g, rows, items),
        "hex" => hex(&g, rows, items),
        "hline" | "vline" | "abline" => reference(&g, rows, items),
        "rug" => rug(&g, rows, items),
        "segment" => segments(&g, rows, items),
        other => return super::bad(format!("geom \"{}\" cannot draw", other)),
    }
    Ok(())
}

/// Points of the rows (their x, y), one batch per glyph; `r_px` overrides
/// the size's radius.
fn points(g: &G, rows: &[usize], items: &mut Vec<Item>, r_px: Option<f64>) {
    let vis = &g.l.vis;
    let mut by_shape: Vec<(Shape, Vec<usize>)> = Vec::new();
    for &i in rows {
        let (x, y) = (g.num(Aes::X, i), g.num(Aes::Y, i));
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let s = vis.shape[i];
        match by_shape.iter_mut().find(|(t, _)| *t == s) {
            Some((_, v)) => v.push(i),
            None => by_shape.push((s, vec![i])),
        }
    }
    let stroke = g.param("stroke").unwrap_or(1.0) * 1.2;
    for (shape, idx) in by_shape {
        let mut pal: Vec<Rgba> = Vec::new();
        let mut pidx: HashMap<(u8, u8, u8, u32), u32> = HashMap::new();
        let mut fill = Vec::with_capacity(idx.len());
        let mut xs = Vec::with_capacity(idx.len());
        let mut ys = Vec::with_capacity(idx.len());
        let mut rs = Vec::with_capacity(idx.len());
        for &i in &idx {
            let (px, py) = g.pc.map(g.num(Aes::X, i), g.num(Aes::Y, i));
            xs.push(px as f32);
            ys.push(py as f32);
            rs.push(r_px.unwrap_or_else(|| radius(vis.size[i])) as f32);
            let c = g.color(i).with_alpha(g.alpha(i, 1.0));
            let key = (c.r, c.g, c.b, (c.a * 1000.0) as u32);
            let k = *pidx.entry(key).or_insert_with(|| {
                pal.push(c);
                (pal.len() - 1) as u32
            });
            fill.push(k);
        }
        let one_r = rs.iter().all(|r| *r == rs[0]);
        let one_c = fill.iter().all(|c| *c == fill[0]);
        items.push(Item::Points(Points {
            xs,
            ys,
            r: if one_r { vec![rs[0]] } else { rs },
            fill: if one_c { vec![fill[0]] } else { fill },
            pal,
            shape,
            stroke_w: stroke as f32,
            panel: g.panel,
            tips: g.tips(&idx),
        }));
    }
}

/// Invisible points that carry tooltips for marks a pointer reads by a
/// centre (hexagons, violins).
fn hover_points(g: &G, rows: &[usize], at: &[(f64, f64)], items: &mut Vec<Item>) {
    if rows.is_empty() || g.l.fields.is_empty() {
        return;
    }
    items.push(Item::Points(Points {
        xs: at.iter().map(|p| p.0 as f32).collect(),
        ys: at.iter().map(|p| p.1 as f32).collect(),
        r: vec![0.0],
        fill: vec![0],
        pal: vec![Rgba {
            a: 0.0,
            ..Rgba::new(0, 0, 0)
        }],
        shape: Shape::Circle,
        stroke_w: 0.0,
        panel: g.panel,
        tips: g.tips(rows),
    }));
}

fn lines(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let f = &g.l.frame;
    let geom = g.l.geom.as_str();
    for mut grp in by_group(f, rows) {
        grp.retain(|&i| g.num(Aes::X, i).is_finite() && g.num(Aes::Y, i).is_finite());
        if grp.len() < 2 {
            continue;
        }
        if matches!(geom, "line" | "step" | "freqpoly" | "ecdf" | "qq_line") {
            grp.sort_by(|&a, &b| g.num(Aes::X, a).partial_cmp(&g.num(Aes::X, b)).unwrap());
        }
        let step = matches!(geom, "step" | "ecdf");
        let dir = p_str(&g.l.params, "direction").unwrap_or("hv");
        let mut pts: Vec<(f64, f64)> = Vec::with_capacity(grp.len() * if step { 2 } else { 1 });
        let mut tip_rows: Vec<usize> = Vec::with_capacity(pts.capacity());
        for (k, &i) in grp.iter().enumerate() {
            let (x, y) = (g.num(Aes::X, i), g.num(Aes::Y, i));
            if step && k > 0 {
                let j = grp[k - 1];
                let (px, py) = (g.num(Aes::X, j), g.num(Aes::Y, j));
                let mid = if dir == "vh" { (px, y) } else { (x, py) };
                pts.push(g.pc.map(mid.0, mid.1));
                tip_rows.push(j);
            }
            pts.push(g.pc.map(x, y));
            tip_rows.push(i);
        }
        let first = grp[0];
        let varies = grp.iter().any(|&i| {
            g.color(i) != g.color(first)
                || g.l.vis.dash[i] != g.l.vis.dash[first]
                || g.lw(i) != g.lw(first)
        });
        if !varies {
            items.push(Item::Path {
                pts,
                stroke: Some(Paint::Rgba(g.color(first).with_alpha(g.alpha(first, 1.0)))),
                sw: g.lw(first),
                dash: g.l.vis.dash[first].clone(),
                fill: None,
                close: false,
                tips: g.tips(&tip_rows),
            });
        } else {
            // a colour along the line: each segment its start's colour
            for k in 1..pts.len() {
                let i = tip_rows[k - 1];
                items.push(Item::Path {
                    pts: vec![pts[k - 1], pts[k]],
                    stroke: Some(Paint::Rgba(g.color(i).with_alpha(g.alpha(i, 1.0)))),
                    sw: g.lw(i),
                    dash: g.l.vis.dash[i].clone(),
                    fill: None,
                    close: false,
                    tips: g.tips(&[tip_rows[k - 1], tip_rows[k]]),
                });
            }
        }
    }
}

fn area(g: &G, rows: &[usize], items: &mut Vec<Item>, density: bool) {
    let f = &g.l.frame;
    let groups = by_group(f, rows);
    let multi = groups.len() > 1;
    for mut grp in groups {
        grp.retain(|&i| g.num(Aes::X, i).is_finite() && g.num(Aes::Y, i).is_finite());
        if grp.len() < 2 {
            continue;
        }
        grp.sort_by(|&a, &b| g.num(Aes::X, a).partial_cmp(&g.num(Aes::X, b)).unwrap());
        let hi = |i: usize| {
            if g.has(Aes::Ymax) && g.num(Aes::Ymax, i).is_finite() {
                g.num(Aes::Ymax, i)
            } else {
                g.num(Aes::Y, i)
            }
        };
        let lo = |i: usize| {
            if g.has(Aes::Ymin) && g.num(Aes::Ymin, i).is_finite() {
                g.num(Aes::Ymin, i)
            } else {
                0.0
            }
        };
        let upper: Vec<(f64, f64)> = grp
            .iter()
            .map(|&i| g.pc.map(g.num(Aes::X, i), hi(i)))
            .collect();
        let mut poly = upper.clone();
        for &i in grp.iter().rev() {
            poly.push(g.pc.map(g.num(Aes::X, i), lo(i)));
        }
        let first = grp[0];
        let dflt = if density {
            0.2
        } else if multi || g.l.vis.fill_mapped {
            0.85
        } else {
            0.3
        };
        items.push(Item::Path {
            pts: poly,
            stroke: None,
            sw: 0.0,
            dash: None,
            fill: Some(Paint::Rgba(g.fill(first).with_alpha(g.alpha(first, dflt)))),
            close: true,
            tips: None,
        });
        let edge = if g.l.vis.color_mapped
            || p_str(&g.l.params, "color").is_some()
            || p_str(&g.l.params, "colour").is_some()
        {
            g.color(first)
        } else {
            g.fill(first)
        };
        items.push(Item::Path {
            pts: upper,
            stroke: Some(Paint::Rgba(edge)),
            sw: g.lw(first).max(1.2),
            dash: g.l.vis.dash[first].clone(),
            fill: None,
            close: false,
            tips: g.tips(&grp),
        });
    }
}

fn ribbon(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    for mut grp in by_group(&g.l.frame, rows) {
        grp.retain(|&i| {
            g.num(Aes::X, i).is_finite()
                && g.num(Aes::Ymin, i).is_finite()
                && g.num(Aes::Ymax, i).is_finite()
        });
        if grp.len() < 2 {
            continue;
        }
        grp.sort_by(|&a, &b| g.num(Aes::X, a).partial_cmp(&g.num(Aes::X, b)).unwrap());
        let mut poly: Vec<(f64, f64)> = grp
            .iter()
            .map(|&i| g.pc.map(g.num(Aes::X, i), g.num(Aes::Ymax, i)))
            .collect();
        for &i in grp.iter().rev() {
            poly.push(g.pc.map(g.num(Aes::X, i), g.num(Aes::Ymin, i)));
        }
        let first = grp[0];
        items.push(Item::Path {
            pts: poly,
            stroke: if p_str(&g.l.params, "color").is_some() || g.l.vis.color_mapped {
                Some(Paint::Rgba(g.color(first)))
            } else {
                None
            },
            sw: 1.0,
            dash: None,
            fill: Some(Paint::Rgba(g.fill(first).with_alpha(g.alpha(first, 0.25)))),
            close: true,
            tips: None,
        });
    }
}

fn smooth(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let se = p_bool(&g.l.params, "se").unwrap_or(true);
    for mut grp in by_group(&g.l.frame, rows) {
        grp.retain(|&i| g.num(Aes::X, i).is_finite() && g.num(Aes::Y, i).is_finite());
        if grp.len() < 2 {
            continue;
        }
        grp.sort_by(|&a, &b| g.num(Aes::X, a).partial_cmp(&g.num(Aes::X, b)).unwrap());
        let first = grp[0];
        let c = g.color(first);
        if se && g.has(Aes::Ymin) {
            let mut poly: Vec<(f64, f64)> = grp
                .iter()
                .map(|&i| g.pc.map(g.num(Aes::X, i), g.num(Aes::Ymax, i)))
                .collect();
            for &i in grp.iter().rev() {
                poly.push(g.pc.map(g.num(Aes::X, i), g.num(Aes::Ymin, i)));
            }
            let band = if g.l.vis.fill_mapped || p_str(&g.l.params, "fill").is_some() {
                g.fill(first)
            } else if g.l.vis.color_mapped
                || p_str(&g.l.params, "color").is_some()
                || p_str(&g.l.params, "colour").is_some()
            {
                c
            } else {
                g.b.theme.app.role(Role::Ink2)
            };
            items.push(Item::Path {
                pts: poly,
                stroke: None,
                sw: 0.0,
                dash: None,
                fill: Some(Paint::Rgba(band.with_alpha(g.alpha(first, 0.16)))),
                close: true,
                tips: None,
            });
        }
        let pts: Vec<(f64, f64)> = grp
            .iter()
            .map(|&i| g.pc.map(g.num(Aes::X, i), g.num(Aes::Y, i)))
            .collect();
        items.push(Item::Path {
            pts,
            stroke: Some(Paint::Rgba(c)),
            sw: g.lw(first),
            dash: g.l.vis.dash[first].clone(),
            fill: None,
            close: false,
            tips: g.tips(&grp),
        });
    }
}

fn bars(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let hist = g.l.geom == "histogram";
    let outline = p_str(&g.l.params, "color").is_some()
        || p_str(&g.l.params, "colour").is_some()
        || g.l.vis.color_mapped && g.l.vis.fill_mapped;
    for &i in rows {
        let (x0, x1, y0, y1) = (
            g.num(Aes::Xmin, i),
            g.num(Aes::Xmax, i),
            g.num(Aes::Ymin, i),
            g.num(Aes::Ymax, i),
        );
        let fill = Paint::Rgba(g.fill(i).with_alpha(g.alpha(i, 1.0)));
        let stroke = if outline {
            Some(Paint::Rgba(g.color(i)))
        } else if hist {
            // adjacent bins kept apart by a hairline of the surface
            Some(Paint::Role(Role::Surface))
        } else {
            None
        };
        if let Some(it) = g.rect(
            i,
            x0,
            x1,
            y0,
            y1,
            Some(fill),
            stroke,
            if outline { 1.0 } else { 0.7 },
        ) {
            items.push(it);
        }
    }
}

fn boxplot(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let f = &g.l.frame;
    let filled = g.l.vis.fill_mapped || p_str(&g.l.params, "fill").is_some();
    let outlier_r = g.param("outlier_size").map(radius).unwrap_or(1.8);
    let show_out = p_bool(&g.l.params, "outliers").unwrap_or(true);
    for &i in rows {
        let (x0, x1, x) = (g.num(Aes::Xmin, i), g.num(Aes::Xmax, i), g.num(Aes::X, i));
        let (lo, mid, hi) = (
            g.num(Aes::Lower, i),
            g.num(Aes::Middle, i),
            g.num(Aes::Upper, i),
        );
        let (wlo, whi) = (g.num(Aes::Ymin, i), g.num(Aes::Ymax, i));
        let c = g.color(i);
        let lw = g.lw(i);
        // whiskers first, under the box
        for it in [
            g.line(x, hi, x, whi, c, lw, None),
            g.line(x, lo, x, wlo, c, lw, None),
        ]
        .into_iter()
        .flatten()
        {
            items.push(it);
        }
        let fill = if filled {
            Paint::Rgba(g.fill(i).with_alpha(g.alpha(i, 0.45)))
        } else {
            Paint::Role(Role::Surface)
        };
        if let Some(it) = g.rect(i, x0, x1, lo, hi, Some(fill), Some(Paint::Rgba(c)), lw) {
            items.push(it);
        }
        if let Some(it) = g.line(x0, mid, x1, mid, c, lw * 1.8, None) {
            items.push(it);
        }
        if show_out
            && let Some(o) = &f.outliers
            && !o[i].is_empty()
        {
            let xs: Vec<f32> = o[i].iter().map(|v| g.pc.map(x, *v).0 as f32).collect();
            let ys: Vec<f32> = o[i].iter().map(|v| g.pc.map(x, *v).1 as f32).collect();
            let tipcol = o[i]
                .iter()
                .map(|v| super::build::axis_value(if g.pc.flip { &g.b.sy } else { &g.b.sy }, *v))
                .collect();
            items.push(Item::Points(Points {
                xs,
                ys,
                r: vec![outlier_r as f32],
                fill: vec![0],
                pal: vec![c.with_alpha(0.8)],
                shape: Shape::Circle,
                stroke_w: 1.0,
                panel: g.panel,
                tips: Some(Tips {
                    layer: g.layer,
                    cols: vec![
                        vec![super::build::axis_value(&g.b.sx, x); o[i].len()],
                        tipcol,
                    ],
                }),
            }));
        }
    }
}

fn violin(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let quantiles = p_nums(&g.l.params, "draw_quantiles").unwrap_or_default();
    let mut centres = Vec::new();
    let mut centre_rows = Vec::new();
    for mut grp in by_group(&g.l.frame, rows) {
        grp.retain(|&i| g.num(Aes::Y, i).is_finite());
        if grp.len() < 2 {
            continue;
        }
        grp.sort_by(|&a, &b| g.num(Aes::Y, a).partial_cmp(&g.num(Aes::Y, b)).unwrap());
        let vw = g.l.frame.stat.get("violinwidth");
        let half = |i: usize| {
            (g.num(Aes::Xmax, i) - g.num(Aes::Xmin, i)) / 2.0 * vw.map(|v| v[i]).unwrap_or(0.0)
        };
        let mut poly: Vec<(f64, f64)> = grp
            .iter()
            .map(|&i| g.pc.map(g.num(Aes::X, i) + half(i), g.num(Aes::Y, i)))
            .collect();
        for &i in grp.iter().rev() {
            poly.push(g.pc.map(g.num(Aes::X, i) - half(i), g.num(Aes::Y, i)));
        }
        let first = grp[0];
        items.push(Item::Path {
            pts: poly,
            stroke: Some(Paint::Rgba(g.color(first))),
            sw: g.lw(first),
            dash: None,
            fill: Some(Paint::Rgba(g.fill(first).with_alpha(g.alpha(first, 0.35)))),
            close: true,
            tips: None,
        });
        // quantile lines across
        let ys: Vec<f64> = grp.iter().map(|&i| g.num(Aes::Y, i)).collect();
        let stat = &g.l.frame.stat;
        for q in &quantiles {
            let yq = match *q {
                v if (v - 0.5).abs() < 1e-9 => stat.get("median").map(|m| m[first]),
                v if (v - 0.25).abs() < 1e-9 => stat.get("q1").map(|m| m[first]),
                v if (v - 0.75).abs() < 1e-9 => stat.get("q3").map(|m| m[first]),
                _ => None,
            };
            if let Some(yq) = yq {
                let k = ys.partition_point(|y| *y < yq).min(grp.len() - 1);
                let h = half(grp[k]);
                let x = g.num(Aes::X, first);
                if let Some(it) = g.line(
                    x - h,
                    yq,
                    x + h,
                    yq,
                    g.color(first),
                    g.lw(first),
                    if (q - 0.5).abs() < 1e-9 {
                        None
                    } else {
                        Some(vec![3.0, 3.0])
                    },
                ) {
                    items.push(it);
                }
            }
        }
        let med = stat
            .get("median")
            .map(|m| m[first])
            .unwrap_or(ys[ys.len() / 2]);
        centres.push(g.pc.map(g.num(Aes::X, first), med));
        centre_rows.push(first);
    }
    hover_points(g, &centre_rows, &centres, items);
}

fn ranges(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let geom = g.l.geom.as_str();
    let mut pr_rows = Vec::new();
    for &i in rows {
        let x = g.num(Aes::X, i);
        let (lo, hi) = (g.num(Aes::Ymin, i), g.num(Aes::Ymax, i));
        let c = g.color(i).with_alpha(g.alpha(i, 1.0));
        let lw = g.lw(i);
        match geom {
            "crossbar" => {
                let (x0, x1) = (g.num(Aes::Xmin, i), g.num(Aes::Xmax, i));
                if let Some(it) = g.rect(
                    i,
                    x0,
                    x1,
                    lo,
                    hi,
                    Some(Paint::Role(Role::Surface)),
                    Some(Paint::Rgba(c)),
                    lw,
                ) {
                    items.push(it);
                }
                if let Some(it) = g.line(
                    x0,
                    g.num(Aes::Y, i),
                    x1,
                    g.num(Aes::Y, i),
                    c,
                    lw * 1.8,
                    None,
                ) {
                    items.push(it);
                }
            }
            _ => {
                if let Some(it) = g.line(x, lo, x, hi, c, lw, None) {
                    items.push(it);
                }
                if geom == "errorbar" {
                    let (x0, x1) = (g.num(Aes::Xmin, i), g.num(Aes::Xmax, i));
                    for y in [lo, hi] {
                        if let Some(it) = g.line(x0, y, x1, y, c, lw, None) {
                            items.push(it);
                        }
                    }
                }
                if matches!(geom, "pointrange" | "summary") {
                    pr_rows.push(i);
                }
            }
        }
    }
    if !pr_rows.is_empty() {
        points(g, &pr_rows, items, None);
    } else if geom == "errorbar" || geom == "linerange" || geom == "crossbar" {
        let at: Vec<(f64, f64)> = rows
            .iter()
            .map(|&i| {
                g.pc.map(
                    g.num(Aes::X, i),
                    (g.num(Aes::Ymin, i) + g.num(Aes::Ymax, i)) / 2.0,
                )
            })
            .collect();
        hover_points(g, rows, &at, items);
    }
}

fn text(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let label_col = g.l.frame.cols.get(&Aes::Label);
    let fixed = p_str(&g.l.params, "label").map(|s| s.to_string());
    let hjust = g.param("hjust").unwrap_or(0.5);
    let vjust = g.param("vjust").unwrap_or(0.5);
    let anchor = if hjust <= 0.25 {
        Anchor::Start
    } else if hjust >= 0.75 {
        Anchor::End
    } else {
        Anchor::Middle
    };
    let overlap = p_bool(&g.l.params, "check_overlap").unwrap_or(false);
    let boxed = g.l.geom == "label";
    let weight = match p_str(&g.l.params, "fontface") {
        Some("bold") => 600,
        _ => 400,
    };
    let mut taken: Vec<(f64, f64, f64, f64)> = Vec::new();
    // the default ink reads against what is under it: a tile's, a bar's or
    // its own box's fill takes the dark ink or the light by contrast
    let auto_ink = !g.l.vis.color_mapped
        && p_str(&g.l.params, "color").is_none()
        && p_str(&g.l.params, "colour").is_none();
    let own_fill = boxed && (g.l.vis.fill_mapped || p_str(&g.l.params, "fill").is_some());
    let under = if auto_ink && !boxed {
        fills_under(g, rows.len())
    } else {
        Vec::new()
    };
    for &i in rows {
        let (x, y) = (g.num(Aes::X, i), g.num(Aes::Y, i));
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let ink = if !auto_ink {
            None
        } else if own_fill {
            Some(ink_on(g.fill(i), g.b.theme.app.role(Role::Surface)))
        } else {
            under
                .iter()
                .rev()
                .find(|u| {
                    x >= u.0.min(u.1) && x <= u.0.max(u.1) && y >= u.2.min(u.3) && y <= u.2.max(u.3)
                })
                .map(|u| ink_on(u.4, g.b.theme.app.role(Role::Surface)))
        };
        let s = match (&fixed, label_col) {
            (Some(s), _) => s.clone(),
            (None, Some(c @ Col::Cat(..))) => {
                if c.is_na(i) {
                    continue;
                }
                c.text(i)
            }
            (None, Some(Col::Num(v))) => super::data::fmt_value(v[i]),
            (None, None) => continue,
        };
        let size = g.l.vis.size[i];
        let (px, py0) = g.pc.map(x, y);
        let py = py0 + (vjust - 0.5) * size * 1.2;
        let w = width(&s, size, weight > 400);
        let left = match anchor {
            Anchor::Start => px,
            Anchor::Middle => px - w / 2.0,
            Anchor::End => px - w,
        };
        let bbox = (left - 2.0, py - size * 0.7, left + w + 2.0, py + size * 0.7);
        if overlap
            && taken
                .iter()
                .any(|t| bbox.0 < t.2 && bbox.2 > t.0 && bbox.1 < t.3 && bbox.3 > t.1)
        {
            continue;
        }
        taken.push(bbox);
        if boxed {
            items.push(Item::Rect {
                x: left - 4.0,
                y: py - size * 0.8,
                w: w + 8.0,
                h: size * 1.6,
                fill: Some(
                    if g.l.vis.fill_mapped || p_str(&g.l.params, "fill").is_some() {
                        Paint::Rgba(g.fill(i))
                    } else {
                        Paint::Role(Role::Surface)
                    },
                ),
                stroke: Some(Paint::Role(Role::Axis)),
                sw: 0.8,
                r: 3.0,
                tip: None,
            });
        }
        items.push(Item::Text {
            x: px,
            y: py,
            text: s,
            size,
            paint: Paint::Rgba(ink.unwrap_or(g.color(i)).with_alpha(g.alpha(i, 1.0))),
            anchor,
            weight,
        });
    }
}

/// The filled boxes (data units) drawn under a text layer in its panel,
/// bottom first: each `(x0, x1, y0, y1, fill)` of a tile, raster, rect,
/// bar or 2-D bin, its alpha kept. Empty when there are too many to
/// search for the labels at hand.
fn fills_under(g: &G, labels: usize) -> Vec<(f64, f64, f64, f64, Rgba)> {
    let mut out = Vec::new();
    for l in g.b.layers.iter().take(g.layer as usize) {
        if !matches!(
            l.geom.as_str(),
            "tile" | "raster" | "rect" | "bar" | "col" | "histogram" | "bin2d" | "bin_2d"
        ) {
            continue;
        }
        let f = &l.frame;
        let (Some(x0), Some(x1), Some(y0), Some(y1)) = (
            f.num(Aes::Xmin),
            f.num(Aes::Xmax),
            f.num(Aes::Ymin),
            f.num(Aes::Ymax),
        ) else {
            continue;
        };
        for i in 0..f.n {
            if f.panel[i] != g.panel || ![x0[i], x1[i], y0[i], y1[i]].iter().all(|v| v.is_finite())
            {
                continue;
            }
            let a = l
                .vis
                .alpha
                .get(i)
                .copied()
                .filter(|a| a.is_finite())
                .unwrap_or(1.0);
            out.push((x0[i], x1[i], y0[i], y1[i], l.vis.fill[i].with_alpha(a)));
        }
    }
    if out.len().saturating_mul(labels) > 8_000_000 {
        Vec::new()
    } else {
        out
    }
}

/// The ink that reads on `fill` (composited over `surface`): the darker
/// or the lighter of the two appearances' inks, whichever contrasts more.
pub(crate) fn ink_on(fill: Rgba, surface: Rgba) -> Rgba {
    let c = surface.mix(Rgba { a: 1.0, ..fill }, fill.a.clamp(0.0, 1.0) as f64);
    let dark = Appearance::Light.role(Role::Ink);
    let light = Appearance::Dark.role(Role::Ink);
    let ratio = |a: Rgba, b: Rgba| {
        let (x, y) = (a.luminance() + 0.05, b.luminance() + 0.05);
        x.max(y) / x.min(y)
    };
    if ratio(c, dark) >= ratio(c, light) {
        dark
    } else {
        light
    }
}

fn tiles(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let outline = p_str(&g.l.params, "color")
        .or_else(|| p_str(&g.l.params, "colour"))
        .is_some();
    let raster = g.l.geom == "raster";
    for &i in rows {
        let (x0, x1, y0, y1) = (
            g.num(Aes::Xmin, i),
            g.num(Aes::Xmax, i),
            g.num(Aes::Ymin, i),
            g.num(Aes::Ymax, i),
        );
        let fill = Paint::Rgba(g.fill(i).with_alpha(g.alpha(i, 1.0)));
        let stroke = if outline {
            Some(Paint::Rgba(g.color(i)))
        } else if raster {
            // a raster's cells abut: their own colour closes the seam
            Some(fill)
        } else {
            None
        };
        if let Some(it) = g.rect(
            i,
            x0,
            x1,
            y0,
            y1,
            Some(fill),
            stroke,
            if outline {
                g.param("linewidth").map(|v| v * 2.8).unwrap_or(1.0)
            } else {
                0.5
            },
        ) {
            items.push(it);
        }
    }
}

fn hex(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let st = &g.l.frame.stat;
    let mut centres = Vec::with_capacity(rows.len());
    for &i in rows {
        let (cx, cy) = (g.num(Aes::X, i), g.num(Aes::Y, i));
        let w = st.get("width").map(|v| v[i]).unwrap_or(1.0);
        let h = st.get("height").map(|v| v[i]).unwrap_or(1.0);
        let verts = [
            (cx, cy + h * 2.0 / 3.0),
            (cx + w / 2.0, cy + h / 3.0),
            (cx + w / 2.0, cy - h / 3.0),
            (cx, cy - h * 2.0 / 3.0),
            (cx - w / 2.0, cy - h / 3.0),
            (cx - w / 2.0, cy + h / 3.0),
        ];
        let fill = Paint::Rgba(g.fill(i).with_alpha(g.alpha(i, 1.0)));
        items.push(Item::Path {
            pts: verts.iter().map(|(x, y)| g.pc.map(*x, *y)).collect(),
            stroke: Some(fill),
            sw: 0.5,
            dash: None,
            fill: Some(fill),
            close: true,
            tips: None,
        });
        centres.push(g.pc.map(cx, cy));
    }
    hover_points(g, rows, &centres, items);
}

fn reference(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let (xl, xh) = g.pc.x_range();
    let (yl, yh) = g.pc.y_range();
    for &i in rows {
        let c = g.color(i).with_alpha(g.alpha(i, 1.0));
        let dash = g.l.vis.dash[i].clone();
        let it = match g.l.geom.as_str() {
            "hline" => {
                let y = g.num(Aes::Yintercept, i);
                g.line(xl, y, xh, y, c, g.lw(i), dash)
            }
            "vline" => {
                let x = g.num(Aes::Xintercept, i);
                g.line(x, yl, x, yh, c, g.lw(i), dash)
            }
            _ => {
                // the line's part inside the panel (a coord's limits or a
                // fixed ratio leave most of a steep line outside it)
                let (s, b) = (g.num(Aes::Slope, i), g.num(Aes::Intercept, i));
                match clip_line(s, b, (xl, xh), (yl, yh)) {
                    Some((x0, x1)) => g.line(x0, b + s * x0, x1, b + s * x1, c, g.lw(i), dash),
                    None => None,
                }
            }
        };
        if let Some(it) = it {
            items.push(it);
        }
    }
}

/// The x span of y = b + s·x inside the box `xr` × `yr`, or None when the
/// line misses it.
pub(crate) fn clip_line(s: f64, b: f64, xr: (f64, f64), yr: (f64, f64)) -> Option<(f64, f64)> {
    let (xl, xh) = (xr.0.min(xr.1), xr.0.max(xr.1));
    let (yl, yh) = (yr.0.min(yr.1), yr.0.max(yr.1));
    if !(s.is_finite() && b.is_finite()) {
        return None;
    }
    if s == 0.0 {
        return (b >= yl && b <= yh).then_some((xl, xh));
    }
    let (xa, xb) = ((yl - b) / s, (yh - b) / s);
    let lo = xl.max(xa.min(xb));
    let hi = xh.min(xa.max(xb));
    (lo < hi).then_some((lo, hi))
}

fn rug(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    let sides = p_str(&g.l.params, "sides").unwrap_or("bl");
    let len = g.param("length").unwrap_or(6.0);
    let pc = g.pc;
    for &i in rows {
        let c = g.color(i).with_alpha(g.alpha(i, 0.6));
        let lw = g.lw(i).min(1.2);
        let (x, y) = (g.num(Aes::X, i), g.num(Aes::Y, i));
        // x's ticks stand on the axis x maps to (the left one under flip)
        if x.is_finite() && (sides.contains('b') || sides.contains('t')) {
            let (px, py) = pc.map(x, pc.y_range().0);
            let (a, b, cc, d) = if pc.flip {
                (pc.x, py, pc.x + len, py)
            } else if sides.contains('b') {
                (px, pc.y + pc.h, px, pc.y + pc.h - len)
            } else {
                (px, pc.y, px, pc.y + len)
            };
            items.push(Item::Line {
                x1: a,
                y1: b,
                x2: cc,
                y2: d,
                stroke: Paint::Rgba(c),
                sw: lw,
                dash: None,
            });
        }
        if y.is_finite() && (sides.contains('l') || sides.contains('r')) {
            let (px, py) = pc.map(pc.x_range().0, y);
            let (a, b, cc, d) = if pc.flip {
                (px, pc.y + pc.h, px, pc.y + pc.h - len)
            } else if sides.contains('l') {
                (pc.x, py, pc.x + len, py)
            } else {
                (pc.x + pc.w, py, pc.x + pc.w - len, py)
            };
            items.push(Item::Line {
                x1: a,
                y1: b,
                x2: cc,
                y2: d,
                stroke: Paint::Rgba(c),
                sw: lw,
                dash: None,
            });
        }
    }
}

fn segments(g: &G, rows: &[usize], items: &mut Vec<Item>) {
    for &i in rows {
        let c = g.color(i).with_alpha(g.alpha(i, 1.0));
        if let Some(it) = g.line(
            g.num(Aes::X, i),
            g.num(Aes::Y, i),
            g.num(Aes::Xend, i),
            g.num(Aes::Yend, i),
            c,
            g.lw(i),
            g.l.vis.dash[i].clone(),
        ) {
            items.push(it);
        }
    }
}

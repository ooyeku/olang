//! The pipeline: a spec's layers read, faceted, grouped, transformed,
//! computed, positioned and scaled; then laid out and drawn (layout.rs,
//! geom.rs) into a scene.

use super::data::{Col, LFrame, NA, fmt_value, parse_iso};
use super::layout;
use super::position;
use super::scale::{NpMap, NpScale, PosKind, PosScale, Trans, Trained, natural_cmp, strftime};
use super::scene::{Rgba, Scene, Shape};
use super::stat::{self, StatCtx};
use super::theme::{Appearance, Theme, quiet};
use super::{Aes, Column, Data, Layer, Param, Params, RenderOptions, Source, Spec, bad, p_bool, p_num, p_str};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// A geom's defaults: its stat and position.
pub fn geom_defaults(geom: &str) -> Option<(&'static str, &'static str)> {
    Some(match geom {
        "point" => ("identity", "identity"),
        "jitter" => ("identity", "jitter"),
        "line" | "path" | "step" => ("identity", "identity"),
        "area" => ("identity", "stack"),
        "ribbon" => ("identity", "identity"),
        "bar" => ("count", "stack"),
        "col" => ("identity", "stack"),
        "histogram" => ("bin", "stack"),
        "freqpoly" => ("bin", "identity"),
        "density" => ("density", "identity"),
        "boxplot" => ("boxplot", "dodge2"),
        "violin" => ("ydensity", "dodge"),
        "errorbar" | "pointrange" | "linerange" | "crossbar" => ("identity", "identity"),
        "summary" => ("summary", "identity"),
        "smooth" => ("smooth", "identity"),
        "text" | "label" => ("identity", "identity"),
        "tile" | "raster" | "rect" => ("identity", "identity"),
        "hline" | "vline" | "abline" => ("identity", "identity"),
        "rug" | "segment" => ("identity", "identity"),
        "hex" => ("binhex", "identity"),
        "bin2d" | "bin_2d" => ("bin2d", "identity"),
        "ecdf" => ("ecdf", "identity"),
        "qq" => ("qq", "identity"),
        "qq_line" => ("qq_line", "identity"),
        _ => return None,
    })
}

/// Geoms whose bars stand on zero (no expansion under the floor).
fn floored(geom: &str) -> bool {
    matches!(geom, "bar" | "col" | "histogram" | "area" | "density" | "freqpoly")
}

/// The visual values of a layer's rows after the scales map them.
#[derive(Clone, Debug, Default)]
pub struct Vis {
    pub color: Vec<Rgba>,
    pub fill: Vec<Rgba>,
    /// ggplot size units (points: radius by `layout::radius`)
    pub size: Vec<f64>,
    pub alpha: Vec<f64>,
    pub shape: Vec<Shape>,
    pub dash: Vec<Option<Vec<f64>>>,
    /// line width in pixels
    pub linewidth: f64,
    /// each row's line width in pixels, when a scale maps linewidth
    pub lws: Option<Vec<f64>>,
    /// whether colour / fill came from a mapping (else the layer's default)
    pub color_mapped: bool,
    pub fill_mapped: bool,
}

/// A layer as the pipeline leaves it.
pub struct LayerOut {
    pub geom: String,
    pub stat: String,
    pub params: Params,
    pub frame: LFrame,
    pub vis: Vis,
    /// each aesthetic's title (its column, or the stat's variable)
    pub titles: BTreeMap<Aes, String>,
    /// the aesthetics mapped (scaled ones a legend may show)
    pub mapped: BTreeSet<Aes>,
    pub legend: bool,
    /// the tooltip's fields: (name, aes or stat variable)
    pub fields: Vec<(String, TipSrc)>,
    /// whether x and y were whole numbers before positions moved them
    pub whole: (bool, bool),
    /// x and y as the data said them, when a position moved them (a
    /// jittered point's tooltip says its category, a stacked bar its own
    /// height)
    pub said: BTreeMap<Aes, Vec<f64>>,
}

#[derive(Clone, Debug)]
pub enum TipSrc {
    Aes(Aes),
    Stat(&'static str),
    Range,
}

/// What the layout and the geoms read: the trained scales and panels.
pub struct Built {
    pub theme: Theme,
    pub layers: Vec<LayerOut>,
    pub sx: PosScale,
    pub sy: PosScale,
    pub np: BTreeMap<Aes, NpScale>,
    pub facet: FacetLayout,
    /// trained axes per panel: (x, y)
    pub tx: Vec<Trained>,
    pub ty: Vec<Trained>,
    pub flip: bool,
    pub ratio: Option<f64>,
    pub labels: BTreeMap<String, String>,
    pub x_title: String,
    pub y_title: String,
    pub zoom: Option<(f64, f64)>,
}

/// The facet's panels: their labels and grid places.
#[derive(Clone, Debug, Default)]
pub struct FacetLayout {
    pub kind: String,
    pub n: usize,
    pub nrow: usize,
    pub ncol: usize,
    /// (row, col) of each panel
    pub pos: Vec<(usize, usize)>,
    /// wrap: each panel's strip; grid: column strips and row strips
    pub labels: Vec<String>,
    pub col_labels: Vec<String>,
    pub row_labels: Vec<String>,
    pub free_x: bool,
    pub free_y: bool,
}

impl FacetLayout {
    /// The key of a panel's x scale (panels sharing one train together).
    pub fn x_key(&self, p: usize) -> usize {
        if !self.free_x {
            0
        } else if self.kind == "grid" {
            self.pos[p].1
        } else {
            p
        }
    }
    pub fn y_key(&self, p: usize) -> usize {
        if !self.free_y {
            0
        } else if self.kind == "grid" {
            self.pos[p].0
        } else {
            p
        }
    }
}

// ── reading a layer's columns ────────────────────────────────────────

fn sorted_levels(vals: impl Iterator<Item = String>, order: Option<&Vec<String>>) -> Vec<String> {
    let set: BTreeSet<String> = vals.collect();
    match order {
        Some(o) => {
            let mut out: Vec<String> = o.iter().filter(|l| set.contains(*l)).cloned().collect();
            let mut rest: Vec<String> = set.into_iter().filter(|l| !o.contains(l)).collect();
            rest.sort_by(|a, b| natural_cmp(a, b));
            out.extend(rest);
            out
        }
        None => {
            let mut v: Vec<String> = set.into_iter().collect();
            v.sort_by(|a, b| natural_cmp(a, b));
            v
        }
    }
}

fn to_cat(c: &Column, order: Option<&Vec<String>>) -> Col {
    match c {
        Column::Str(v) => {
            let levels = sorted_levels(v.iter().flatten().cloned(), order);
            let index: HashMap<&str, u32> = levels.iter().enumerate().map(|(i, l)| (l.as_str(), i as u32)).collect();
            let codes = v
                .iter()
                .map(|s| s.as_deref().and_then(|s| index.get(s).copied()).unwrap_or(NA))
                .collect();
            Col::Cat(codes, Arc::new(levels))
        }
        Column::Num(v) => {
            // numbers as levels, in numeric order
            let texts: Vec<Option<String>> = v.iter().map(|x| if x.is_finite() { Some(fmt_value(*x)) } else { None }).collect();
            to_cat(&Column::Str(texts), order)
        }
    }
}

/// Whether a text column is all ISO dates; their seconds if so.
fn as_time(c: &Column) -> Option<(Vec<f64>, bool)> {
    let Column::Str(v) = c else { return None };
    let mut any = false;
    let mut datetime = false;
    let mut out = Vec::with_capacity(v.len());
    for s in v {
        match s {
            None => out.push(f64::NAN),
            Some(s) if s.is_empty() => out.push(f64::NAN),
            Some(s) => {
                let (t, dt) = parse_iso(s)?;
                any = true;
                datetime |= dt;
                out.push(t);
            }
        }
    }
    if any { Some((out, datetime)) } else { None }
}

fn time_text(t: f64, datetime: bool) -> String {
    if !t.is_finite() {
        return "NA".into();
    }
    if datetime || t % 86400.0 != 0.0 {
        strftime(t, "%Y-%m-%d %H:%M")
    } else {
        strftime(t, "%Y-%m-%d")
    }
}

/// The mapping a layer uses: the plot's (unless it does not inherit)
/// under its own.
fn effective_mapping(spec: &Spec, layer: &Layer) -> Vec<(Aes, Source)> {
    let mut m: Vec<(Aes, Source)> = if layer.inherit { spec.mapping.clone() } else { vec![] };
    for (a, s) in &layer.mapping {
        m.retain(|(b, _)| b != a);
        m.push((*a, s.clone()));
    }
    // a constant given as a parameter overrides the mapping
    for k in ["color", "colour", "fill", "size", "alpha", "shape", "linetype", "linewidth"] {
        if layer.params.contains_key(k)
            && let Some(a) = Aes::parse(k)
        {
            m.retain(|(b, _)| *b != a);
        }
    }
    m
}

/// The aesthetics a geom needs from a stat that are fixed for it.
fn constant_params_aes(geom: &str) -> &'static [(&'static str, Aes)] {
    match geom {
        "hline" => &[("yintercept", Aes::Yintercept)],
        "vline" => &[("xintercept", Aes::Xintercept)],
        "abline" => &[("slope", Aes::Slope), ("intercept", Aes::Intercept)],
        _ => &[],
    }
}

// ── the pipeline ─────────────────────────────────────────────────────

struct RawLayer {
    layer: Layer,
    geom: String,
    stat: String,
    position: String,
    mapping: Vec<(Aes, Source)>,
    frame: LFrame,
    titles: BTreeMap<Aes, String>,
    /// rows of the layer's data (before facetting copies)
    data_n: usize,
    /// panel per row, None for "every panel"
    every_panel: bool,
}

pub fn render(spec: &Spec, opts: &RenderOptions) -> super::Result<Scene> {
    let built = build(spec, opts)?;
    layout::draw(&built)
}

pub fn build(spec: &Spec, opts: &RenderOptions) -> super::Result<Built> {
    let theme = Theme::of(&spec.theme, opts.appearance.unwrap_or(Appearance::Light));
    let app = theme.app;
    let flip = spec.coord.kind == "flip";
    let ratio = if spec.coord.kind == "fixed" || spec.coord.kind == "equal" {
        Some(spec.coord.ratio.unwrap_or(1.0))
    } else {
        None
    };
    if spec.layers.is_empty() {
        return bad("a plot needs a layer (gg.point, gg.line, gg.bar, …)");
    }

    // scales as specified
    let mut sx = PosScale::new('x');
    let mut sy = PosScale::new('y');
    let mut np: BTreeMap<Aes, NpScale> = BTreeMap::new();
    for s in &spec.scales {
        match s.aes.as_str() {
            "x" => sx.configure(s)?,
            "y" => sy.configure(s)?,
            other => {
                let a = Aes::parse(other).filter(|a| a.scaled()).ok_or_else(|| {
                    crate::OdsError::InvalidArgument(format!("gg: a scale for \"{}\" (x, y, color, fill, size, alpha, shape, linetype)", other))
                })?;
                np.entry(a).or_insert_with(|| NpScale::new(a)).configure(s)?;
            }
        }
    }

    // ── read each layer's aesthetics ──
    let mut raws: Vec<RawLayer> = Vec::new();
    for layer in &spec.layers {
        let (dstat, dpos) = geom_defaults(&layer.geom).ok_or_else(|| {
            crate::OdsError::InvalidArgument(format!("gg: unknown geom \"{}\"", layer.geom))
        })?;
        let stat_name = layer.stat.clone().unwrap_or_else(|| dstat.to_string());
        let position = layer.position.clone().unwrap_or_else(|| dpos.to_string());
        let mapping = effective_mapping(spec, layer);
        let data = layer.data.as_ref().or(spec.data.as_ref());
        let empty = Data::default();
        let data = data.unwrap_or(&empty);
        let mut frame = LFrame::default();
        let mut titles = BTreeMap::new();
        // a reference line's constants make its rows
        let consts = constant_params_aes(&layer.geom);
        let const_rows = consts.iter().any(|(k, a)| layer.params.contains_key(*k) && !mapping.iter().any(|(b, _)| b == a));
        let n = if const_rows {
            consts
                .iter()
                .filter_map(|(k, _)| super::p_nums(&layer.params, k).map(|v| v.len()))
                .max()
                .unwrap_or(1)
        } else if mapping.iter().all(|(_, s)| matches!(s, Source::Stat(_))) && data.n == 0 {
            0
        } else {
            data.n
        };
        frame.n = n;
        if const_rows {
            for (k, a) in consts {
                if let Some(v) = super::p_nums(&layer.params, k) {
                    frame.set_num(*a, (0..n).map(|i| v[i % v.len()]).collect());
                } else if *a == Aes::Intercept {
                    frame.set_num(*a, vec![0.0; n]);
                } else if *a == Aes::Slope {
                    frame.set_num(*a, vec![1.0; n]);
                }
            }
        } else {
            for (a, src) in &mapping {
                let col = match src {
                    Source::Stat(_) => continue,
                    Source::Col(name) => {
                        if !consts.iter().any(|(_, ca)| ca == a) && data.n == 0 && data.names.is_empty() {
                            return bad(format!("{} = \"{}\": the plot has no data (gg.plot(frame, …))", a.name(), name));
                        }
                        match data.get(name) {
                            Some(c) => c.clone(),
                            None => {
                                // a column missing from an annotation layer's own data is fine
                                if layer.data.is_some() && !layer.data.as_ref().unwrap().names.contains(name) && *a != Aes::X && *a != Aes::Y {
                                    continue;
                                }
                                return bad(format!(
                                    "{} = \"{}\": no such column (the frame has {}; one value for every row: gg.constant(\"{}\"))",
                                    a.name(),
                                    name,
                                    data.names.join(", "),
                                    name
                                ));
                            }
                        }
                    }
                    Source::Values(_, c) => c.clone(),
                    // a constant: its one value on every row
                    Source::Const(_, c) => match c {
                        Column::Num(v) => Column::Num(vec![v.first().copied().unwrap_or(f64::NAN); n]),
                        Column::Str(v) => Column::Str(vec![v.first().cloned().flatten(); n]),
                    },
                };
                if col.len() != n {
                    return bad(format!(
                        "{} = \"{}\" has {} values, the data {}",
                        a.name(),
                        src.title(),
                        col.len(),
                        n
                    ));
                }
                titles.insert(*a, src.title().to_string());
                let scale_wants_disc = match a {
                    Aes::Shape | Aes::Linetype | Aes::Group | Aes::Label => true,
                    // a constant colour or the like is one level (ggplot's aes(colour = "all"))
                    a if a.scaled() && matches!(src, Source::Const(..)) => true,
                    a if a.scaled() => np.get(a).is_some_and(|s| s.wants_discrete(false)),
                    _ => false,
                };
                let c = match (&col, a.axis()) {
                    (Column::Num(v), None) if *a == Aes::Label => {
                        Col::Cat((0..v.len()).map(|i| i as u32).collect(), Arc::new(v.iter().map(|x| fmt_value(*x)).collect()))
                    }
                    (Column::Str(v), None) if *a == Aes::Label => {
                        Col::Cat((0..v.len()).map(|i| if v[i].is_some() { i as u32 } else { NA }).collect(), Arc::new(v.iter().map(|s| s.clone().unwrap_or_default()).collect()))
                    }
                    (Column::Num(v), _) if !scale_wants_disc => Col::Num(v.clone()),
                    (Column::Num(_), _) => {
                        let order = np.get(a).and_then(|s| super::p_strs(&s.params, "limits"));
                        to_cat(&col, order.as_ref())
                    }
                    (Column::Str(_), Some(axis)) => {
                        let ps = if axis == 'x' { &sx } else { &sy };
                        let want_time = matches!(ps.kind, PosKind::Time { .. });
                        let want_disc = ps.kind == PosKind::Discrete;
                        match (!want_disc).then(|| as_time(&col)).flatten() {
                            Some((t, _)) => Col::Num(t),
                            None if want_time => {
                                return bad(format!("{} = \"{}\": not ISO dates (YYYY-MM-DD[ HH:MM[:SS]])", a.name(), src.title()));
                            }
                            None => to_cat(&col, ps.dlimits.as_ref()),
                        }
                    }
                    (Column::Str(_), None) => {
                        let order = np.get(a).and_then(|s| super::p_strs(&s.params, "limits"));
                        to_cat(&col, order.as_ref())
                    }
                };
                // time on an axis: the scale learns it is one
                if let (Column::Str(_), Some(axis), Col::Num(_)) = (&col, a.axis(), &c) {
                    let datetime = as_time(&col).map(|t| t.1).unwrap_or(false);
                    let ps = if axis == 'x' { &mut sx } else { &mut sy };
                    match ps.kind {
                        PosKind::Time { datetime: d } => ps.kind = PosKind::Time { datetime: d || datetime },
                        _ => ps.kind = PosKind::Time { datetime },
                    }
                }
                frame.cols.insert(*a, c);
            }
        }
        frame.panel = vec![0; n];
        frame.group = vec![0; n];
        raws.push(RawLayer {
            layer: layer.clone(),
            geom: layer.geom.clone(),
            stat: stat_name,
            position,
            mapping,
            frame,
            titles,
            data_n: n,
            every_panel: false,
        });
    }

    // ── facets ──
    let facet = facet_panels(spec, &mut raws)?;

    // ── discrete positions: levels across layers, then codes to 1..k ──
    for (axis, ps) in [('x', &mut sx), ('y', &mut sy)] {
        let mut levels: BTreeSet<String> = BTreeSet::new();
        let mut any = false;
        for r in &raws {
            for (a, c) in &r.frame.cols {
                if a.axis() == Some(axis)
                    && let Col::Cat(_, l) = c
                {
                    any = true;
                    levels.extend(l.iter().cloned());
                }
            }
        }
        if any {
            if matches!(ps.kind, PosKind::Time { .. }) {
                return bad(format!("the {} axis mixes dates and categories", axis));
            }
            ps.kind = PosKind::Discrete;
            ps.levels = sorted_levels(levels.into_iter(), ps.dlimits.as_ref());
        } else if ps.kind == PosKind::Discrete && ps.levels.is_empty() {
            ps.levels = ps.dlimits.clone().unwrap_or_default();
        }
    }

    // ── groups: the interaction of the categorical aesthetics ──
    for r in raws.iter_mut() {
        let f = &mut r.frame;
        // a group mapped is the grouping (ggplot): alone, not crossed with
        // the other categories, so `group: 1` joins a line across a
        // categorical x
        let explicit = f.cols.get(&Aes::Group);
        let keys: Vec<&Col> = match explicit {
            Some(g @ Col::Cat(..)) => vec![g],
            _ => f
                .cols
                .iter()
                .filter(|(a, c)| matches!(c, Col::Cat(..)) && **a != Aes::Label)
                .map(|(_, c)| c)
                .collect(),
        };
        if keys.is_empty() && explicit.is_none() {
            continue;
        }
        let mut combos: BTreeMap<Vec<u32>, u32> = BTreeMap::new();
        let mut rows: Vec<Vec<u32>> = Vec::with_capacity(f.n);
        for i in 0..f.n {
            let mut k: Vec<u32> = keys.iter().map(|c| if let Col::Cat(v, _) = c { v[i] } else { 0 }).collect();
            if let Some(Col::Num(v)) = explicit {
                k.push(v[i].to_bits() as u32);
            }
            combos.entry(k.clone()).or_insert(0);
            rows.push(k);
        }
        for (i, v) in combos.values_mut().enumerate() {
            *v = i as u32;
        }
        f.group = rows.iter().map(|k| combos[k]).collect();
    }

    // codes on a discrete axis become 1..k (by the scale's levels)
    for r in raws.iter_mut() {
        let f = &mut r.frame;
        let aes: Vec<Aes> = f.cols.keys().copied().collect();
        for a in aes {
            let Some(axis) = a.axis() else { continue };
            let ps = if axis == 'x' { &sx } else { &sy };
            if let Some(Col::Cat(codes, levels)) = f.cols.get(&a) {
                let index: HashMap<&str, usize> = ps.levels.iter().enumerate().map(|(i, l)| (l.as_str(), i)).collect();
                let v: Vec<f64> = codes
                    .iter()
                    .map(|&c| {
                        if c == NA {
                            f64::NAN
                        } else {
                            index.get(levels[c as usize].as_str()).map(|i| (*i + 1) as f64).unwrap_or(f64::NAN)
                        }
                    })
                    .collect();
                f.cols.insert(a, Col::Num(v));
            }
        }
    }

    // ── transform continuous positions ──
    for r in raws.iter_mut() {
        for (axis, ps) in [('x', &sx), ('y', &sy)] {
            if ps.trans == Trans::Identity || ps.kind != PosKind::Continuous {
                continue;
            }
            let aes: Vec<Aes> = r.frame.cols.keys().copied().filter(|a| a.axis() == Some(axis)).collect();
            for a in aes {
                if let Some(v) = r.frame.num(a).cloned() {
                    r.frame.set_num(a, v.iter().map(|x| ps.trans.apply(*x)).collect());
                }
            }
        }
    }

    // ── stats ──
    let range_of = |axis: char| -> (f64, f64) {
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for r in raws.iter() {
            for (a, c) in &r.frame.cols {
                if a.axis() == Some(axis)
                    && let Col::Num(v) = c
                {
                    for x in v {
                        if x.is_finite() {
                            lo = lo.min(*x);
                            hi = hi.max(*x);
                        }
                    }
                }
            }
        }
        if lo.is_finite() { (lo, hi) } else { (0.0, 1.0) }
    };
    let (xr, yr) = (range_of('x'), range_of('y'));
    // a free scale's panels each bin (and grid a density) over their own
    // range: each panel's, joined over the panels that share its scale
    let panel_range = |axis: char| -> Option<Vec<(f64, f64)>> {
        let free = if axis == 'x' { facet.free_x } else { facet.free_y };
        if !free {
            return None;
        }
        let mut per = vec![(f64::INFINITY, f64::NEG_INFINITY); facet.n];
        for r in raws.iter() {
            for (a, c) in &r.frame.cols {
                if a.axis() == Some(axis)
                    && let Col::Num(v) = c
                {
                    for (i, x) in v.iter().enumerate() {
                        let p = r.frame.panel[i] as usize;
                        if x.is_finite() && p < per.len() {
                            per[p].0 = per[p].0.min(*x);
                            per[p].1 = per[p].1.max(*x);
                        }
                    }
                }
            }
        }
        let key = |p: usize| if axis == 'x' { facet.x_key(p) } else { facet.y_key(p) };
        let joined = (0..facet.n)
            .map(|p| {
                let (lo, hi) = (0..facet.n)
                    .filter(|q| key(*q) == key(p))
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, q| (acc.0.min(per[q].0), acc.1.max(per[q].1)));
                if lo.is_finite() { (lo, hi) } else if axis == 'x' { xr } else { yr }
            })
            .collect();
        Some(joined)
    };
    let (pxr, pyr) = (panel_range('x'), panel_range('y'));
    let mut layers: Vec<LayerOut> = Vec::new();
    for r in raws {
        let RawLayer {
            layer,
            geom,
            stat: stat_name,
            position,
            mapping,
            frame,
            mut titles,
            ..
        } = r;
        let params = layer.params.clone();
        let ctx = |panel: u32| StatCtx {
            x_range: pxr.as_ref().and_then(|v| v.get(panel as usize).copied()).unwrap_or(xr),
            y_range: pyr.as_ref().and_then(|v| v.get(panel as usize).copied()).unwrap_or(yr),
        };
        if matches!(stat_name.as_str(), "bin" | "density" | "ecdf") && sx.kind == PosKind::Discrete {
            return bad(format!(
                "stat_{} needs a continuous x, and x is categorical here (gg.bar counts categories)",
                stat_name
            ));
        }
        let mut f = if stat_name == "identity" {
            frame
        } else {
            run_stat(&stat_name, &frame, &params, &ctx)?
        };
        if geom == "violin" {
            stat::violin_widths(&mut f, p_str(&params, "scale").unwrap_or("area"));
        }
        // after_stat mappings, and the stats' own defaults
        let mut mapped: BTreeSet<Aes> = mapping.iter().map(|(a, _)| *a).collect();
        for (a, src) in &mapping {
            if let Source::Stat(name) = src {
                let v = f.stat.get(name.as_str()).cloned().ok_or_else(|| {
                    crate::OdsError::InvalidArgument(format!(
                        "gg: after_stat({}): stat_{} computes {}",
                        name,
                        stat_name,
                        f.stat.keys().cloned().collect::<Vec<_>>().join(", ")
                    ))
                })?;
                f.set_num(*a, v);
                titles.insert(*a, name.clone());
            }
        }
        if matches!(geom.as_str(), "hex" | "bin2d" | "bin_2d")
            && !mapped.contains(&Aes::Fill)
            && !params.contains_key("fill")
            && let Some(c) = f.stat.get("count").cloned()
        {
            f.set_num(Aes::Fill, c);
            titles.insert(Aes::Fill, "count".into());
            mapped.insert(Aes::Fill);
        }
        // stat-made axes' titles
        let stat_y = match stat_name.as_str() {
            "bin" | "count" => Some("count"),
            "density" => Some("density"),
            "ecdf" => Some("ecdf"),
            "qq" | "qq_line" => Some("sample"),
            _ => None,
        };
        if let Some(t) = stat_y
            && !mapping.iter().any(|(a, _)| *a == Aes::Y)
        {
            titles.insert(Aes::Y, t.to_string());
        }
        if matches!(stat_name.as_str(), "qq" | "qq_line") {
            titles.insert(Aes::X, "theoretical".into());
        }
        if stat_name == "boxplot" && !titles.contains_key(&Aes::Y) {
            titles.insert(Aes::Y, "y".into());
        }
        let is_whole = |a: Aes| f.num(a).is_some_and(|v| v.iter().all(|x| !x.is_finite() || x.fract() == 0.0));
        let whole = (is_whole(Aes::X), is_whole(Aes::Y));
        setup_geom(&geom, &mut f, &params);
        let said: BTreeMap<Aes, Vec<f64>> = if position == "identity" {
            BTreeMap::new()
        } else {
            [Aes::X, Aes::Y].into_iter().filter_map(|a| f.num(a).map(|v| (a, v.clone()))).collect()
        };
        position::apply(&position, &mut f, &params)?;
        let fields = tip_fields(&geom, &stat_name, &titles, &f);
        layers.push(LayerOut {
            geom,
            stat: stat_name,
            params,
            frame: f,
            vis: Vis::default(),
            titles,
            mapped,
            legend: layer.legend,
            fields,
            whole,
            said,
        });
    }

    // ── train position scales per panel ──
    let zoom = opts.zoom.map(|(a, b, _)| (a.min(b), a.max(b)));
    let zoom_panel = opts.zoom.and_then(|z| z.2);
    let (htrain_is_x, h_scale_free) = if flip { (false, facet.free_y) } else { (true, facet.free_x) };
    let (pw_est, ph_est) = estimate_panel(&theme, &facet, spec);
    let mut tx = Vec::with_capacity(facet.n);
    let mut ty = Vec::with_capacity(facet.n);
    let mut cache_x: HashMap<usize, Trained> = HashMap::new();
    let mut cache_y: HashMap<usize, Trained> = HashMap::new();
    for p in 0..facet.n {
        let zoom_here = zoom.filter(|_| zoom_panel.is_none() || !h_scale_free || zoom_panel == Some(p));
        let xk = facet.x_key(p);
        let yk = facet.y_key(p);
        let panels_x: Vec<usize> = (0..facet.n).filter(|q| facet.x_key(*q) == xk).collect();
        let panels_y: Vec<usize> = (0..facet.n).filter(|q| facet.y_key(*q) == yk).collect();
        // under zoom the horizontal scale is fixed and the other fits what shows
        let hz = |is_x: bool| if is_x == htrain_is_x { zoom_here } else { None };
        let (xz, yz) = (hz(true), hz(false));
        let t_x = match cache_x.get(&xk).filter(|_| xz.is_none() && yz.is_none()) {
            Some(t) => t.clone(),
            None => {
                let filt = if !htrain_is_x { yz.map(|z| ('y', sy.trans.apply(z.0).min(sy.trans.apply(z.1)), sy.trans.apply(z.0).max(sy.trans.apply(z.1)))) } else { None };
                let (lo, hi, floor, whole) = data_range(&layers, 'x', &panels_x, filt);
                let len = if flip { ph_est } else { pw_est };
                let t = train_axis(&sx, lo, hi, len, !flip, whole, floor, xz, spec.coord.xlim);
                if xz.is_none() && yz.is_none() {
                    cache_x.insert(xk, t.clone());
                }
                t
            }
        };
        let t_y = match cache_y.get(&yk).filter(|_| xz.is_none() && yz.is_none()) {
            Some(t) => t.clone(),
            None => {
                let filt = if htrain_is_x { xz.map(|z| ('x', sx.trans.apply(z.0).min(sx.trans.apply(z.1)), sx.trans.apply(z.0).max(sx.trans.apply(z.1)))) } else { None };
                let (lo, hi, floor, whole) = data_range(&layers, 'y', &panels_y, filt);
                let len = if flip { pw_est } else { ph_est };
                let t = train_axis(&sy, lo, hi, len, flip, whole, floor, yz, spec.coord.ylim);
                if xz.is_none() && yz.is_none() {
                    cache_y.insert(yk, t.clone());
                }
                t
            }
        };
        tx.push(t_x);
        ty.push(t_y);
    }

    // ── non-position scales: train, then map every row ──
    let used: BTreeSet<Aes> = layers
        .iter()
        .flat_map(|l| l.frame.cols.keys().copied().filter(|a| a.scaled()))
        .collect();
    for a in used {
        let sc = np.entry(a).or_insert_with(|| NpScale::new(a));
        if sc.kind == "identity" {
            sc.map = Some(NpMap::Identity);
            continue;
        }
        let mut levels: BTreeSet<String> = BTreeSet::new();
        let mut disc = false;
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for l in &layers {
            match l.frame.cols.get(&a) {
                Some(Col::Cat(_, lv)) => {
                    disc = true;
                    levels.extend(lv.iter().cloned());
                }
                Some(Col::Num(v)) => {
                    for x in v {
                        let t = sc.trans.apply(*x);
                        if t.is_finite() {
                            lo = lo.min(t);
                            hi = hi.max(t);
                        }
                    }
                }
                None => {}
            }
        }
        if disc {
            let order = super::p_strs(&sc.params, "limits");
            let lv = sorted_levels(levels.into_iter(), order.as_ref());
            sc.train_discrete(lv, app)?;
        } else {
            sc.train_continuous(lo, hi, app)?;
        }
    }
    for l in layers.iter_mut() {
        l.vis = map_visuals(l, &np, &theme)?;
    }

    let title_of = |a: Aes, axis: &PosScale| -> String {
        if let Some(n) = &axis.name {
            return n.clone();
        }
        if let Some(t) = spec.labels.get(a.name()) {
            return t.clone();
        }
        for l in &layers {
            if let Some(t) = l.titles.get(&a) {
                return t.clone();
            }
        }
        // an axis drawn only by xmin/xmax or the like
        let alt = if a == Aes::X { [Aes::Xmin, Aes::Xintercept] } else { [Aes::Ymin, Aes::Yintercept] };
        for l in &layers {
            for b in alt {
                if let Some(t) = l.titles.get(&b) {
                    return t.clone();
                }
            }
        }
        String::new()
    };
    let x_title = title_of(Aes::X, &sx);
    let y_title = title_of(Aes::Y, &sy);
    Ok(Built {
        theme,
        layers,
        sx,
        sy,
        np,
        facet,
        tx,
        ty,
        flip,
        ratio,
        labels: spec.labels.clone(),
        x_title,
        y_title,
        zoom,
    })
}

#[allow(clippy::too_many_arguments)]
fn train_axis(ps: &PosScale, lo: f64, hi: f64, len: f64, horizontal: bool, whole: bool, floor: bool, zoom: Option<(f64, f64)>, clim: Option<(f64, f64)>) -> Trained {
    // a coord's limits zoom without dropping data
    let z = zoom.or(clim);
    ps.train_on(lo, hi, len, horizontal, whole, floor, z)
}

/// A layer's rows, stat run per panel and group, the group's constant
/// aesthetics carried onto what the stat made.
fn run_stat(name: &str, f: &LFrame, params: &Params, ctx: &dyn Fn(u32) -> StatCtx) -> super::Result<LFrame> {
    let mut by: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for i in 0..f.n {
        by.entry((f.panel[i], f.group[i])).or_default().push(i);
    }
    let mut parts = Vec::with_capacity(by.len());
    for ((panel, group), idx) in by {
        let sub = f.take(&idx);
        let mut out = stat::compute(name, &sub, params, &ctx(panel))?;
        let m = out.n;
        out.panel = vec![panel; m];
        out.group = vec![group; m];
        if m > 0 {
            for (a, c) in &sub.cols {
                if out.cols.contains_key(a) || a.axis().is_some() || matches!(a, Aes::Weight | Aes::Sample) {
                    continue;
                }
                out.cols.insert(*a, c.repeat(0, m));
            }
        }
        parts.push(out);
    }
    if parts.is_empty() {
        let mut e = f.take(&[]);
        e.n = 0;
        return Ok(e);
    }
    Ok(LFrame::concat(parts))
}

/// Geom-specific columns before positions: bars' extents, tiles' boxes.
fn setup_geom(geom: &str, f: &mut LFrame, p: &Params) {
    let n = f.n;
    let x = f.num(Aes::X).cloned();
    let res = x.as_ref().map(|x| position::resolution(x)).unwrap_or(1.0);
    match geom {
        "bar" | "col" | "histogram" => {
            if f.num(Aes::Xmin).is_none()
                && let Some(x) = &x
            {
                let w = p_num(p, "width").unwrap_or(0.9) * if p_num(p, "width").is_some() { 1.0 } else { res };
                f.set_num(Aes::Xmin, x.iter().map(|v| v - w / 2.0).collect());
                f.set_num(Aes::Xmax, x.iter().map(|v| v + w / 2.0).collect());
            }
            if let Some(y) = f.num(Aes::Y).cloned() {
                f.set_num(Aes::Ymin, vec![0.0; n]);
                f.set_num(Aes::Ymax, y);
            }
        }
        "boxplot" | "violin" | "crossbar" | "errorbar" => {
            if let Some(x) = &x {
                let dflt = match geom {
                    "boxplot" => 0.75,
                    "violin" => 0.9,
                    "errorbar" => 0.5,
                    _ => 0.6,
                };
                let w = p_num(p, "width").unwrap_or(dflt) * res;
                f.set_num(Aes::Xmin, x.iter().map(|v| v - w / 2.0).collect());
                f.set_num(Aes::Xmax, x.iter().map(|v| v + w / 2.0).collect());
            }
        }
        "tile" | "raster" => {
            if let Some(x) = &x {
                let w = p_num(p, "width").unwrap_or(1.0) * res;
                f.set_num(Aes::Xmin, x.iter().map(|v| v - w / 2.0).collect());
                f.set_num(Aes::Xmax, x.iter().map(|v| v + w / 2.0).collect());
            }
            if let Some(y) = f.num(Aes::Y).cloned() {
                let h = p_num(p, "height").unwrap_or(1.0) * position::resolution(&y);
                f.set_num(Aes::Ymin, y.iter().map(|v| v - h / 2.0).collect());
                f.set_num(Aes::Ymax, y.iter().map(|v| v + h / 2.0).collect());
            }
        }
        _ => {}
    }
}

/// The finite range of an axis's values over some panels' rows (ymin,
/// outliers and the like included), whether a bar stands on zero there,
/// and, under a zoom, only rows inside it on the other axis.
fn data_range(layers: &[LayerOut], axis: char, panels: &[usize], within: Option<(char, f64, f64)>) -> (f64, f64, bool, bool) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut floor = false;
    let mut whole = true;
    let main = if axis == 'x' { Aes::X } else { Aes::Y };
    for l in layers {
        let f = &l.frame;
        let inside: Option<Vec<bool>> = within.map(|(oaxis, a, b)| {
            let main = if oaxis == 'x' { Aes::X } else { Aes::Y };
            let (mn, mx) = if oaxis == 'x' { (Aes::Xmin, Aes::Xmax) } else { (Aes::Ymin, Aes::Ymax) };
            (0..f.n)
                .map(|i| {
                    let lo_i = f.num(mn).map(|v| v[i]).filter(|v| v.is_finite()).or(f.num(main).map(|v| v[i]));
                    let hi_i = f.num(mx).map(|v| v[i]).filter(|v| v.is_finite()).or(f.num(main).map(|v| v[i]));
                    match (lo_i, hi_i) {
                        (Some(p), Some(q)) if p.is_finite() && q.is_finite() => q >= a && p <= b,
                        _ => true,
                    }
                })
                .collect()
        });
        let in_panel: Vec<bool> = (0..f.n).map(|i| panels.contains(&(f.panel[i] as usize))).collect();
        for (a, c) in &f.cols {
            if a.axis() != Some(axis) {
                continue;
            }
            // a reference line spans whatever the data does: it does not train
            if matches!(a, Aes::Xintercept | Aes::Yintercept) && l.geom != "hline" && l.geom != "vline" {
                continue;
            }
            if let Col::Num(v) = c {
                for i in 0..f.n {
                    if in_panel[i] && v[i].is_finite() && inside.as_ref().is_none_or(|m| m[i]) {
                        lo = lo.min(v[i]);
                        hi = hi.max(v[i]);
                        if *a == main && !(if axis == 'x' { l.whole.0 } else { l.whole.1 }) {
                            whole = false;
                        }
                    }
                }
            }
        }
        if axis == 'y'
            && let Some(o) = &f.outliers
        {
            for i in 0..f.n {
                if in_panel[i] {
                    for v in &o[i] {
                        lo = lo.min(*v);
                        hi = hi.max(*v);
                    }
                }
            }
        }
        // a violin's or hex's own width/height in the data
        if l.geom == "hex" {
            let w = f.stat.get(if axis == 'x' { "width" } else { "height" });
            if let (Some(w), Some(Col::Num(v))) = (w, f.cols.get(&if axis == 'x' { Aes::X } else { Aes::Y })) {
                for i in 0..f.n {
                    if in_panel[i] && v[i].is_finite() {
                        let half = if axis == 'x' { w[i] / 2.0 } else { w[i] * 2.0 / 3.0 };
                        lo = lo.min(v[i] - half);
                        hi = hi.max(v[i] + half);
                    }
                }
            }
        }
        if axis == 'y' && floored(&l.geom) {
            floor = true;
        }
    }
    if !lo.is_finite() {
        return (f64::NAN, f64::NAN, false, false);
    }
    (lo, hi, floor && lo == 0.0, whole)
}

/// Facet panels from the spec: each row's panel, and rows of a layer
/// without the facet's columns copied into every panel.
fn facet_panels(spec: &Spec, raws: &mut [RawLayer]) -> super::Result<FacetLayout> {
    let Some(fs) = spec.facet.as_ref().filter(|f| f.kind == "wrap" || f.kind == "grid") else {
        return Ok(FacetLayout {
            kind: "none".into(),
            n: 1,
            nrow: 1,
            ncol: 1,
            pos: vec![(0, 0)],
            labels: vec![String::new()],
            ..Default::default()
        });
    };
    let vars_r = fs.rows.clone();
    let vars_c = if fs.kind == "grid" { fs.cols.clone() } else { vec![] };
    // each layer's per-row key texts for a list of variables
    let key_of = |r: &RawLayer, vars: &[String]| -> Option<Vec<String>> {
        if vars.is_empty() {
            return Some(vec![String::new(); r.data_n]);
        }
        let data = r.layer.data.as_ref().or(spec.data.as_ref())?;
        let cols: Vec<&Column> = vars.iter().map(|v| data.get(v)).collect::<Option<Vec<_>>>()?;
        Some(
            (0..r.data_n)
                .map(|i| {
                    cols.iter()
                        .map(|c| match c {
                            Column::Num(v) => fmt_value(v[i]),
                            Column::Str(v) => v[i].clone().unwrap_or_else(|| "NA".into()),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .collect(),
        )
    };
    let mut rk: Vec<Option<Vec<String>>> = Vec::new();
    let mut ck: Vec<Option<Vec<String>>> = Vec::new();
    let mut rset: BTreeSet<String> = BTreeSet::new();
    let mut cset: BTreeSet<String> = BTreeSet::new();
    for r in raws.iter() {
        // a layer with its rows made from constants (a reference line) has no facet columns
        let has_data = r.data_n > 0 && !constant_params_aes(&r.geom).iter().any(|(k, _)| r.layer.params.contains_key(*k));
        let a = if has_data { key_of(r, &vars_r) } else { None };
        let b = if has_data { key_of(r, &vars_c) } else { None };
        if let (Some(a), Some(b)) = (&a, &b) {
            rset.extend(a.iter().cloned());
            cset.extend(b.iter().cloned());
        }
        if a.is_some() != b.is_some() {
            return bad("a layer has some of the facet's columns but not all");
        }
        rk.push(a);
        ck.push(b);
    }
    if rset.is_empty() {
        return bad(format!("facet: no layer's data has the column{} {}", if vars_r.len() + vars_c.len() > 1 { "s" } else { "" }, [vars_r.clone(), vars_c.clone()].concat().join(", ")));
    }
    let mut rl: Vec<String> = rset.into_iter().collect();
    rl.sort_by(|a, b| natural_cmp(a, b));
    let mut cl: Vec<String> = cset.into_iter().collect();
    cl.sort_by(|a, b| natural_cmp(a, b));
    let (free_x, free_y) = match fs.scales.as_str() {
        "free" => (true, true),
        "free_x" => (true, false),
        "free_y" => (false, true),
        _ => (false, false),
    };
    let mut lay = FacetLayout {
        kind: fs.kind.clone(),
        free_x,
        free_y,
        ..Default::default()
    };
    if fs.kind == "wrap" {
        let n = rl.len();
        let (nrow, ncol) = match (fs.nrow, fs.ncol) {
            (Some(r), Some(c)) => (r.max(1), c.max(1)),
            (None, Some(c)) => (n.div_ceil(c.max(1)), c.max(1)),
            (Some(r), None) => (r.max(1), n.div_ceil(r.max(1))),
            (None, None) => {
                let c = (n as f64).sqrt().ceil() as usize;
                (n.div_ceil(c.max(1)), c.max(1))
            }
        };
        if nrow * ncol < n {
            return bad(format!("facet_wrap: {} panels do not fit {} × {}", n, nrow, ncol));
        }
        lay.n = n;
        lay.nrow = nrow;
        lay.ncol = ncol;
        lay.pos = (0..n).map(|i| (i / ncol, i % ncol)).collect();
        lay.labels = rl.clone();
    } else {
        lay.nrow = rl.len();
        lay.ncol = cl.len();
        lay.n = lay.nrow * lay.ncol;
        lay.pos = (0..lay.n).map(|i| (i / lay.ncol, i % lay.ncol)).collect();
        lay.row_labels = if vars_r.is_empty() { vec![] } else { rl.clone() };
        lay.col_labels = if vars_c.is_empty() { vec![] } else { cl.clone() };
        lay.labels = vec![String::new(); lay.n];
    }
    let rindex: HashMap<&str, usize> = rl.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    let cindex: HashMap<&str, usize> = cl.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    for (li, r) in raws.iter_mut().enumerate() {
        match (&rk[li], &ck[li]) {
            (Some(a), Some(b)) => {
                r.frame.panel = (0..r.frame.n)
                    .map(|i| {
                        let ri = rindex[a[i].as_str()];
                        if lay.kind == "wrap" {
                            ri as u32
                        } else {
                            (ri * lay.ncol + cindex[b[i].as_str()]) as u32
                        }
                    })
                    .collect();
            }
            _ => {
                // every panel: the rows copied once per panel
                let n = r.frame.n;
                let idx: Vec<usize> = (0..lay.n).flat_map(|_| 0..n).collect();
                let mut f = r.frame.take(&idx);
                f.panel = (0..lay.n).flat_map(|p| std::iter::repeat_n(p as u32, n)).collect();
                r.frame = f;
                r.every_panel = true;
            }
        }
    }
    Ok(lay)
}

/// A rough panel size before the layout measures (for the breaks' count).
fn estimate_panel(theme: &Theme, facet: &FacetLayout, _spec: &Spec) -> (f64, f64) {
    let w = theme.width.unwrap_or(720.0) - 120.0;
    let h = theme.height.unwrap_or(layout::default_height(facet)) - 110.0;
    ((w / facet.ncol as f64 - 12.0).max(40.0), (h / facet.nrow as f64 - 24.0).max(30.0))
}

/// Default colours: the palette's first hue for data, the inks for words.
pub fn default_color(geom: &str, app: Appearance) -> Rgba {
    let ink = app.role(super::scene::Role::Ink);
    let ink2 = app.role(super::scene::Role::Ink2);
    match geom {
        "text" | "label" => ink,
        "hline" | "vline" | "abline" | "rug" | "qq_line" | "errorbar" | "linerange" | "crossbar" | "boxplot" => ink2,
        "smooth" => ink,
        _ => Rgba::hex(quiet(app)[0]).unwrap(),
    }
}

/// The scales' visual values for each row of a layer.
fn map_visuals(l: &LayerOut, np: &BTreeMap<Aes, NpScale>, theme: &Theme) -> super::Result<Vis> {
    let f = &l.frame;
    let n = f.n;
    let app = theme.app;
    let p = &l.params;
    let cparam = |k: &str| -> super::Result<Option<Rgba>> {
        match p_str(p, k).or_else(|| if k == "color" { p_str(p, "colour") } else { None }) {
            Some(s) => Ok(Some(super::scale::color_or_err(s)?)),
            None => Ok(None),
        }
    };
    let mapped = |a: Aes| f.cols.get(&a).zip(np.get(&a));
    let dcol = default_color(&l.geom, app);
    let color_const = cparam("color")?;
    let fill_const = cparam("fill")?;
    let mut vis = Vis {
        linewidth: p_num(p, "linewidth").map(|v| v * 2.8).unwrap_or(match l.geom.as_str() {
            "smooth" => 1.8,
            "hline" | "vline" | "abline" | "qq_line" => 1.1,
            "errorbar" | "linerange" | "pointrange" | "crossbar" | "boxplot" | "violin" | "rug" | "segment" => 1.2,
            "density" | "area" | "ribbon" | "tile" | "raster" | "hex" | "bin2d" | "bin_2d" => 1.0,
            _ => 1.6,
        }),
        ..Default::default()
    };
    let code_or_val = |c: &Col, i: usize| -> (Option<u32>, f64) {
        match c {
            Col::Cat(v, _) => (Some(v[i]), f64::NAN),
            Col::Num(v) => (None, v[i]),
        }
    };
    let ident_color = |c: &Col, i: usize| -> Rgba {
        match c {
            Col::Cat(v, l) if v[i] != NA => super::scale::color(&l[v[i] as usize]).unwrap_or(Rgba::new(150, 150, 150)),
            _ => Rgba::new(150, 150, 150),
        }
    };
    // colour
    vis.color = match (color_const, mapped(Aes::Color)) {
        (Some(c), _) => vec![c; n],
        (None, Some((c, s))) => {
            vis.color_mapped = true;
            (0..n)
                .map(|i| {
                    if matches!(s.map, Some(NpMap::Identity)) {
                        return ident_color(c, i);
                    }
                    let (code, v) = code_or_val(c, i);
                    s.color_of(code, s.trans.apply(v))
                })
                .collect()
        }
        (None, None) => vec![dcol; n],
    };
    // fill: mapped, given, else the colour's (when mapped) or the default
    vis.fill = match (fill_const, mapped(Aes::Fill)) {
        (Some(c), _) => vec![c; n],
        (None, Some((c, s))) => {
            vis.fill_mapped = true;
            (0..n)
                .map(|i| {
                    if matches!(s.map, Some(NpMap::Identity)) {
                        return ident_color(c, i);
                    }
                    let (code, v) = code_or_val(c, i);
                    s.color_of(code, s.trans.apply(v))
                })
                .collect()
        }
        (None, None) => {
            if vis.color_mapped && color_const.is_none() {
                vis.color.clone()
            } else {
                vec![Rgba::hex(quiet(app)[0]).unwrap(); n]
            }
        }
    };
    // a fill mapped and no colour: outlines follow the fill
    if vis.fill_mapped
        && !vis.color_mapped
        && color_const.is_none()
        && matches!(l.geom.as_str(), "violin" | "density" | "area" | "ribbon" | "smooth" | "boxplot" | "dotplot")
    {
        vis.color = vis.fill.clone();
        if l.geom == "boxplot" {
            vis.color = vec![dcol; n];
        }
    }
    vis.size = match (p_num(p, "size"), mapped(Aes::Size)) {
        (Some(v), _) => vec![v; n],
        (None, Some((c, s))) => (0..n)
            .map(|i| {
                let (code, v) = code_or_val(c, i);
                s.size_of(code, s.trans.apply(v))
            })
            .collect(),
        (None, None) => vec![
            match l.geom.as_str() {
                "text" | "label" => theme.tick() * 0.95 / 1.0,
                "pointrange" => 2.2,
                _ => 1.5,
            };
            n
        ],
    };
    vis.alpha = match (p_num(p, "alpha"), mapped(Aes::Alpha)) {
        (Some(v), _) => vec![v.clamp(0.0, 1.0); n],
        (None, Some((c, s))) => (0..n)
            .map(|i| {
                let (code, v) = code_or_val(c, i);
                s.alpha_of(code, s.trans.apply(v))
            })
            .collect(),
        (None, None) => vec![f64::NAN; n],
    };
    vis.shape = match (p.get("shape"), mapped(Aes::Shape)) {
        (Some(Param::Str(s)), _) => vec![super::scene::Shape::parse(s).unwrap_or(Shape::Circle); n],
        (Some(Param::Num(v)), _) => vec![super::scene::Shape::parse(&(*v as i64).to_string()).unwrap_or(Shape::Circle); n],
        (_, Some((c, s))) => (0..n).map(|i| s.shape_of(code_or_val(c, i).0)).collect(),
        _ => vec![Shape::Circle; n],
    };
    vis.dash = match (p_str(p, "linetype"), mapped(Aes::Linetype)) {
        (Some(s), _) => vec![super::scale::dash(s); n],
        (None, Some((c, s))) => (0..n).map(|i| s.dash_of(code_or_val(c, i).0)).collect(),
        (None, None) => vec![None; n],
    };
    if p_num(p, "linewidth").is_none()
        && let Some((c, s)) = mapped(Aes::Linewidth)
    {
        vis.lws = Some(
            (0..n)
                .map(|i| {
                    let (code, v) = code_or_val(c, i);
                    let w = s.linewidth_of(code, s.trans.apply(v));
                    if w.is_finite() { w * 2.8 } else { vis.linewidth }
                })
                .collect(),
        );
    }
    let _ = p_bool;
    Ok(vis)
}

/// The fields a layer's tooltip says, by geom.
fn tip_fields(geom: &str, stat: &str, titles: &BTreeMap<Aes, String>, f: &LFrame) -> Vec<(String, TipSrc)> {
    let t = |a: Aes, d: &str| titles.get(&a).cloned().unwrap_or_else(|| d.to_string());
    let mut out: Vec<(String, TipSrc)> = Vec::new();
    match (geom, stat) {
        (_, "bin") => {
            out.push((t(Aes::X, "x"), TipSrc::Range));
            out.push(("count".into(), TipSrc::Stat("count")));
        }
        (_, "count") => {
            out.push((t(Aes::X, "x"), TipSrc::Aes(Aes::X)));
            out.push(("count".into(), TipSrc::Stat("count")));
        }
        (_, "boxplot") => {
            out.push((t(Aes::X, "x"), TipSrc::Aes(Aes::X)));
            out.push(("median".into(), TipSrc::Aes(Aes::Middle)));
            out.push(("q1".into(), TipSrc::Aes(Aes::Lower)));
            out.push(("q3".into(), TipSrc::Aes(Aes::Upper)));
            out.push(("whiskers".into(), TipSrc::Aes(Aes::Ymin)));
            out.push(("n".into(), TipSrc::Stat("n")));
        }
        (_, "ydensity") => {
            out.push((t(Aes::X, "x"), TipSrc::Aes(Aes::X)));
            out.push(("median".into(), TipSrc::Stat("median")));
            out.push(("n".into(), TipSrc::Stat("n")));
        }
        (_, "density") => {
            out.push((t(Aes::X, "x"), TipSrc::Aes(Aes::X)));
            out.push(("density".into(), TipSrc::Stat("density")));
        }
        (_, "smooth") => {
            out.push((t(Aes::X, "x"), TipSrc::Aes(Aes::X)));
            out.push(("fit".into(), TipSrc::Aes(Aes::Y)));
            out.push(("lower".into(), TipSrc::Aes(Aes::Ymin)));
            out.push(("upper".into(), TipSrc::Aes(Aes::Ymax)));
        }
        (_, "summary") => {
            out.push((t(Aes::X, "x"), TipSrc::Aes(Aes::X)));
            out.push((t(Aes::Y, "y"), TipSrc::Aes(Aes::Y)));
            out.push(("lower".into(), TipSrc::Aes(Aes::Ymin)));
            out.push(("upper".into(), TipSrc::Aes(Aes::Ymax)));
            out.push(("n".into(), TipSrc::Stat("n")));
        }
        (_, "bin2d" | "bin_2d" | "binhex" | "hex") => {
            out.push((t(Aes::X, "x"), TipSrc::Aes(Aes::X)));
            out.push((t(Aes::Y, "y"), TipSrc::Aes(Aes::Y)));
            out.push(("count".into(), TipSrc::Stat("count")));
        }
        _ => {
            for a in [Aes::X, Aes::Y, Aes::Xmin, Aes::Xmax, Aes::Ymin, Aes::Ymax, Aes::Xend, Aes::Yend] {
                if f.cols.contains_key(&a) && titles.contains_key(&a) {
                    out.push((titles[&a].clone(), TipSrc::Aes(a)));
                }
            }
            if geom == "col" && !titles.contains_key(&Aes::Y) {
                out.push(("y".into(), TipSrc::Aes(Aes::Y)));
            }
        }
    }
    for a in [Aes::Color, Aes::Fill, Aes::Size, Aes::Alpha, Aes::Shape, Aes::Linetype, Aes::Linewidth, Aes::Label] {
        if f.cols.contains_key(&a)
            && let Some(name) = titles.get(&a)
            && !out.iter().any(|(n, _)| n == name)
        {
            out.push((name.clone(), TipSrc::Aes(a)));
        }
    }
    out
}

/// A row's value of a tooltip field, as its scale says it.
pub fn tip_value(b: &Built, l: &LayerOut, src: &TipSrc, i: usize) -> String {
    let f = &l.frame;
    match src {
        TipSrc::Stat(k) => f.stat.get(*k).map(|v| fmt_value(v[i])).unwrap_or_default(),
        TipSrc::Range => {
            let a = f.num(Aes::Xmin).map(|v| v[i]).unwrap_or(f64::NAN);
            let c = f.num(Aes::Xmax).map(|v| v[i]).unwrap_or(f64::NAN);
            format!("{} – {}", axis_value(&b.sx, a), axis_value(&b.sx, c))
        }
        TipSrc::Aes(a) if a.axis().is_some() && l.said.contains_key(a) => {
            axis_value(if a.axis() == Some('x') { &b.sx } else { &b.sy }, l.said[a][i])
        }
        TipSrc::Aes(a) => match (a.axis(), f.cols.get(a)) {
            (Some(ax), Some(Col::Num(v))) => {
                if *a == Aes::Ymin && l.stat == "boxplot" {
                    let hi = f.num(Aes::Ymax).map(|v| v[i]).unwrap_or(f64::NAN);
                    return format!("{} – {}", axis_value(&b.sy, v[i]), axis_value(&b.sy, hi));
                }
                axis_value(if ax == 'x' { &b.sx } else { &b.sy }, v[i])
            }
            (_, Some(c)) => {
                let s = b.np.get(a);
                match (c, s) {
                    (Col::Num(v), Some(sc)) => fmt_value(sc.trans.inverse(sc.trans.apply(v[i]))),
                    _ => c.text(i),
                }
            }
            _ => String::new(),
        },
    }
}

/// A position as its axis says it: a level, a date, the number untransformed.
pub fn axis_value(s: &PosScale, v: f64) -> String {
    if !v.is_finite() {
        return "NA".into();
    }
    match s.kind {
        PosKind::Discrete => {
            let k = v.round() as i64 - 1;
            if (v - v.round()).abs() < 1e-6 && k >= 0 && (k as usize) < s.levels.len() {
                s.levels[k as usize].clone()
            } else {
                fmt_value(v)
            }
        }
        PosKind::Time { datetime } => time_text(v, datetime),
        PosKind::Continuous => fmt_value(s.trans.inverse(v)),
    }
}

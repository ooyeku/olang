//! Stats: what a layer computes from its data before it is drawn — bins,
//! counts, kernel densities, boxplot summaries, means with intervals,
//! smoothers with standard-error bands, empirical cdfs, quantile pairs,
//! 2-D bins. Each runs on one panel's group of rows and returns the rows
//! the geom draws (aesthetics by name, computed variables in `stat`).

use super::data::{Col, LFrame};
use super::math::{inverse, logdet_spd, mean, qnorm, qt, quantile_sorted, sd, solve, sorted_finite};
use super::{Aes, Params, bad, p_bool, p_num, p_nums, p_str};
use std::collections::BTreeMap;

/// What a stat may know beyond its rows: the layer's whole x and y range
/// (transformed), so bins and density grids agree across groups.
pub struct StatCtx {
    pub x_range: (f64, f64),
    pub y_range: (f64, f64),
}

/// A stat's output frame, built row by row.
#[derive(Default)]
struct Out {
    cols: BTreeMap<Aes, Vec<f64>>,
    stat: BTreeMap<String, Vec<f64>>,
    outliers: Option<Vec<Vec<f64>>>,
    n: usize,
}

impl Out {
    fn push(&mut self, aes: &[(Aes, f64)], stat: &[(&str, f64)]) {
        for (a, v) in aes {
            self.cols.entry(*a).or_insert_with(|| vec![f64::NAN; self.n]).push(*v);
        }
        for (k, v) in stat {
            self.stat
                .entry(k.to_string())
                .or_insert_with(|| vec![f64::NAN; self.n])
                .push(*v);
        }
        self.n += 1;
        // keep every column the same length
        for c in self.cols.values_mut() {
            c.resize(self.n, f64::NAN);
        }
        for c in self.stat.values_mut() {
            c.resize(self.n, f64::NAN);
        }
    }

    fn frame(self) -> LFrame {
        LFrame {
            n: self.n,
            cols: self.cols.into_iter().map(|(a, v)| (a, Col::Num(v))).collect(),
            panel: vec![0; self.n],
            group: vec![0; self.n],
            stat: self.stat,
            outliers: self.outliers,
        }
    }
}

fn finite_pairs(x: &[f64], y: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut xs = Vec::with_capacity(x.len());
    let mut ys = Vec::with_capacity(x.len());
    for (a, b) in x.iter().zip(y) {
        if a.is_finite() && b.is_finite() {
            xs.push(*a);
            ys.push(*b);
        }
    }
    (xs, ys)
}

fn need<'a>(f: &'a LFrame, a: Aes, stat: &str) -> super::Result<&'a Vec<f64>> {
    match f.cols.get(&a) {
        Some(Col::Num(v)) => Ok(v),
        Some(Col::Cat(..)) => bad(format!(
            "stat_{} needs a numeric {} (it is categorical here){}",
            stat,
            a.name(),
            if stat == "bin" { ": a bar chart (gg.bar) counts categories" } else { "" }
        )),
        None => bad(format!("stat_{} needs the {} aesthetic", stat, a.name())),
    }
}

fn weights(f: &LFrame) -> Option<&Vec<f64>> {
    f.num(Aes::Weight)
}

/// Run stat `name` on one group's rows.
pub fn compute(name: &str, f: &LFrame, p: &Params, ctx: &StatCtx) -> super::Result<LFrame> {
    match name {
        "identity" => Ok(f.clone()),
        "bin" => bin(f, p, ctx),
        "count" => count(f),
        "density" => density(f, p, ctx),
        "boxplot" => boxplot(f, p),
        "ydensity" => ydensity(f, p),
        "summary" => summary(f, p),
        "smooth" => smooth(f, p, ctx),
        "ecdf" => ecdf(f, ctx),
        "qq" => qq(f, p),
        "qq_line" => qq_line(f, p),
        "bin2d" | "bin_2d" => bin2d(f, p, ctx),
        "binhex" | "hex" => binhex(f, p, ctx),
        "sum" => sum(f),
        other => bad(format!(
            "unknown stat \"{}\" (identity, bin, count, density, boxplot, ydensity, summary, smooth, ecdf, qq, qq_line, bin2d, binhex, sum)",
            other
        )),
    }
}

// ── bin ──────────────────────────────────────────────────────────────

/// The bin edges over a range: `bins` (30) or `binwidth`, aligned to
/// `boundary` or `center` (ggplot's rules), or `breaks`.
pub fn bin_edges(range: (f64, f64), p: &Params) -> Vec<f64> {
    if let Some(b) = p_nums(p, "breaks")
        && b.len() >= 2
    {
        return b;
    }
    let (lo, hi) = range;
    let (lo, hi) = if hi - lo < 1e-12 { (lo - 0.5, hi + 0.5) } else { (lo, hi) };
    let bins = p_num(p, "bins").unwrap_or(30.0).max(1.0);
    let width = p_num(p, "binwidth").unwrap_or_else(|| (hi - lo) / (bins - 1.0).max(1.0));
    let boundary = match (p_num(p, "boundary"), p_num(p, "center")) {
        (Some(b), _) => b,
        (None, Some(c)) => c - width / 2.0,
        (None, None) => {
            if p_num(p, "binwidth").is_some() {
                width / 2.0
            } else {
                // centre the first bin on the minimum, as ggplot does for bins
                lo - width / 2.0
            }
        }
    };
    let shift = ((lo - boundary) / width).floor();
    let origin = boundary + shift * width;
    let mut edges = vec![origin];
    let mut e = origin;
    while e < hi - 1e-9 * width && edges.len() < 100_000 {
        e += width;
        edges.push(e);
    }
    if edges.len() < 2 {
        edges.push(origin + width);
    }
    edges
}

fn bin(f: &LFrame, p: &Params, ctx: &StatCtx) -> super::Result<LFrame> {
    let x = need(f, Aes::X, "bin")?;
    let w = weights(f);
    let edges = bin_edges(ctx.x_range, p);
    let nb = edges.len() - 1;
    let mut counts = vec![0.0; nb];
    let lo = edges[0];
    let uniform = p_nums(p, "breaks").is_none();
    let width0 = edges[1] - edges[0];
    for (i, v) in x.iter().enumerate() {
        if !v.is_finite() {
            continue;
        }
        let k = if uniform {
            let mut k = ((v - lo) / width0).floor() as isize;
            // right-closed bins, the first closed on both sides (ggplot)
            if k > 0 && (v - (lo + k as f64 * width0)).abs() < 1e-9 * width0 {
                k -= 1;
            }
            k.clamp(0, nb as isize - 1) as usize
        } else {
            edges.partition_point(|e| e < v).saturating_sub(1).min(nb - 1)
        };
        counts[k] += w.map(|w| w[i]).unwrap_or(1.0);
    }
    let total: f64 = counts.iter().sum();
    let maxc = counts.iter().cloned().fold(0.0, f64::max);
    let mut out = Out::default();
    let dens: Vec<f64> = (0..nb)
        .map(|k| if total > 0.0 { counts[k] / total / (edges[k + 1] - edges[k]) } else { 0.0 })
        .collect();
    let maxd = dens.iter().cloned().fold(0.0, f64::max);
    for k in 0..nb {
        let (a, b) = (edges[k], edges[k + 1]);
        out.push(
            &[(Aes::X, (a + b) / 2.0), (Aes::Xmin, a), (Aes::Xmax, b), (Aes::Y, counts[k])],
            &[
                ("count", counts[k]),
                ("density", dens[k]),
                ("ncount", if maxc > 0.0 { counts[k] / maxc } else { 0.0 }),
                ("ndensity", if maxd > 0.0 { dens[k] / maxd } else { 0.0 }),
                ("width", b - a),
            ],
        );
    }
    Ok(out.frame())
}

// ── count ────────────────────────────────────────────────────────────

fn count(f: &LFrame) -> super::Result<LFrame> {
    let x = match f.cols.get(&Aes::X) {
        Some(Col::Num(v)) => v,
        _ => return bad("stat_count needs an x"),
    };
    let w = weights(f);
    let mut m: BTreeMap<i64, (f64, f64)> = BTreeMap::new();
    for (i, v) in x.iter().enumerate() {
        if v.is_finite() {
            let e = m.entry((v * 1e9).round() as i64).or_insert((*v, 0.0));
            e.1 += w.map(|w| w[i]).unwrap_or(1.0);
        }
    }
    let total: f64 = m.values().map(|v| v.1).sum();
    let mut out = Out::default();
    for (_, (xv, c)) in m {
        out.push(&[(Aes::X, xv), (Aes::Y, c)], &[("count", c), ("prop", if total > 0.0 { c / total } else { 0.0 })]);
    }
    Ok(out.frame())
}

fn sum(f: &LFrame) -> super::Result<LFrame> {
    let x = need(f, Aes::X, "sum")?;
    let y = need(f, Aes::Y, "sum")?;
    let mut m: BTreeMap<(i64, i64), (f64, f64, f64)> = BTreeMap::new();
    for (a, b) in x.iter().zip(y) {
        if a.is_finite() && b.is_finite() {
            let e = m.entry(((a * 1e9).round() as i64, (b * 1e9).round() as i64)).or_insert((*a, *b, 0.0));
            e.2 += 1.0;
        }
    }
    let total: f64 = m.values().map(|v| v.2).sum();
    let mut out = Out::default();
    for (_, (a, b, n)) in m {
        out.push(&[(Aes::X, a), (Aes::Y, b), (Aes::Size, n)], &[("n", n), ("prop", n / total)]);
    }
    Ok(out.frame())
}

// ── density ──────────────────────────────────────────────────────────

/// A bandwidth by rule: nrd0 (Silverman's rule of thumb, R's default),
/// nrd (Scott's variant), or a number.
pub fn bandwidth(xs_sorted: &[f64], rule: &str) -> super::Result<f64> {
    let n = xs_sorted.len() as f64;
    if xs_sorted.len() < 2 {
        return Ok(1.0);
    }
    let s = sd(xs_sorted);
    let iqr = quantile_sorted(xs_sorted, 0.75) - quantile_sorted(xs_sorted, 0.25);
    let lo = s.min(iqr / 1.34);
    let lo = if lo > 0.0 {
        lo
    } else if s > 0.0 {
        s
    } else if xs_sorted[0] != 0.0 {
        xs_sorted[0].abs()
    } else {
        1.0
    };
    Ok(match rule {
        "nrd0" | "silverman" => 0.9 * lo * n.powf(-0.2),
        "nrd" | "scott" => 1.06 * lo * n.powf(-0.2),
        other => match other.parse::<f64>() {
            Ok(v) if v > 0.0 => v,
            _ => return bad(format!("density: bw \"{}\" (nrd0, nrd, silverman, scott or a number)", other)),
        },
    })
}

#[derive(Clone, Copy)]
enum Kernel {
    Gaussian,
    Epanechnikov,
    Rectangular,
    Triangular,
    Biweight,
}

impl Kernel {
    fn parse(s: &str) -> super::Result<Kernel> {
        Ok(match s {
            "gaussian" | "normal" => Kernel::Gaussian,
            "epanechnikov" => Kernel::Epanechnikov,
            "rectangular" | "uniform" => Kernel::Rectangular,
            "triangular" => Kernel::Triangular,
            "biweight" | "quartic" => Kernel::Biweight,
            o => return bad(format!("density: kernel \"{}\" (gaussian, epanechnikov, rectangular, triangular, biweight)", o)),
        })
    }

    /// The kernel at distance `u` for bandwidth `bw` (its sd), and its
    /// support's half-width.
    fn support(self, bw: f64) -> f64 {
        match self {
            Kernel::Gaussian => 4.0 * bw,
            Kernel::Epanechnikov => bw * 5f64.sqrt(),
            Kernel::Rectangular => bw * 3f64.sqrt(),
            Kernel::Triangular => bw * 6f64.sqrt(),
            Kernel::Biweight => bw * 7f64.sqrt(),
        }
    }

    fn eval(self, u: f64, bw: f64) -> f64 {
        match self {
            Kernel::Gaussian => (-0.5 * (u / bw).powi(2)).exp() / (bw * (2.0 * std::f64::consts::PI).sqrt()),
            Kernel::Epanechnikov => {
                let a = bw * 5f64.sqrt();
                let t = u / a;
                if t.abs() < 1.0 { 0.75 * (1.0 - t * t) / a } else { 0.0 }
            }
            Kernel::Rectangular => {
                let a = bw * 3f64.sqrt();
                if u.abs() < a { 0.5 / a } else { 0.0 }
            }
            Kernel::Triangular => {
                let a = bw * 6f64.sqrt();
                let t = u.abs() / a;
                if t < 1.0 { (1.0 - t) / a } else { 0.0 }
            }
            Kernel::Biweight => {
                let a = bw * 7f64.sqrt();
                let t = u / a;
                if t.abs() < 1.0 { 15.0 / 16.0 * (1.0 - t * t).powi(2) / a } else { 0.0 }
            }
        }
    }
}

/// A weighted kernel density at `grid`: direct sums for small inputs,
/// linear binning onto a fine grid then a truncated convolution for
/// large ones (as R's density does with its FFT).
pub fn kde(xs: &[f64], ws: Option<&[f64]>, bw: f64, kernel_name: &str, grid: &[f64]) -> super::Result<Vec<f64>> {
    let kernel = Kernel::parse(kernel_name)?;
    let wsum: f64 = ws.map(|w| w.iter().sum()).unwrap_or(xs.len() as f64);
    if xs.is_empty() || wsum <= 0.0 {
        return Ok(vec![0.0; grid.len()]);
    }
    let sup = kernel.support(bw);
    if xs.len() * grid.len() <= 400_000 {
        return Ok(grid
            .iter()
            .map(|g| {
                let mut s = 0.0;
                for (i, x) in xs.iter().enumerate() {
                    let u = g - x;
                    if u.abs() <= sup {
                        s += kernel.eval(u, bw) * ws.map(|w| w[i]).unwrap_or(1.0);
                    }
                }
                s / wsum
            })
            .collect());
    }
    // binned: M cells over the grid's span widened by the support
    let lo = grid[0].min(xs.iter().cloned().fold(f64::INFINITY, f64::min)) - sup;
    let hi = grid[grid.len() - 1].max(xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max)) + sup;
    let m = 4096usize;
    let d = (hi - lo) / (m - 1) as f64;
    let mut mass = vec![0.0; m];
    for (i, x) in xs.iter().enumerate() {
        let w = ws.map(|w| w[i]).unwrap_or(1.0);
        let t = (x - lo) / d;
        let k = (t.floor() as usize).min(m - 2);
        let f = t - k as f64;
        mass[k] += w * (1.0 - f);
        mass[k + 1] += w * f;
    }
    let half = ((sup / d).ceil() as usize).min(m);
    let kern: Vec<f64> = (0..=half).map(|j| kernel.eval(j as f64 * d, bw)).collect();
    let mut dens = vec![0.0; m];
    for (k, &mk) in mass.iter().enumerate() {
        if mk == 0.0 {
            continue;
        }
        let a = k.saturating_sub(half);
        let b = (k + half).min(m - 1);
        for (j, dj) in dens[a..=b].iter_mut().enumerate() {
            let dist = (a + j).abs_diff(k);
            *dj += mk * kern[dist];
        }
    }
    Ok(grid
        .iter()
        .map(|g| {
            let t = (g - lo) / d;
            let k = (t.floor().max(0.0) as usize).min(m - 2);
            let f = t - k as f64;
            (dens[k] * (1.0 - f) + dens[k + 1] * f) / wsum
        })
        .collect())
}

fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    if n < 2 {
        return vec![a];
    }
    (0..n).map(|i| a + (b - a) * i as f64 / (n - 1) as f64).collect()
}

fn density(f: &LFrame, p: &Params, ctx: &StatCtx) -> super::Result<LFrame> {
    let x = need(f, Aes::X, "density")?;
    let w = weights(f);
    let mut xs = Vec::new();
    let mut ws = Vec::new();
    for (i, v) in x.iter().enumerate() {
        if v.is_finite() {
            xs.push(*v);
            ws.push(w.map(|w| w[i]).unwrap_or(1.0));
        }
    }
    let mut out = Out::default();
    if xs.len() < 2 {
        return Ok(out.frame());
    }
    let sorted = sorted_finite(&xs);
    let rule = match p.get("bw") {
        Some(super::Param::Num(v)) => v.to_string(),
        Some(super::Param::Str(s)) => s.clone(),
        _ => "nrd0".to_string(),
    };
    let bw = bandwidth(&sorted, &rule)? * p_num(p, "adjust").unwrap_or(1.0);
    let n = p_num(p, "n").unwrap_or(512.0).clamp(16.0, 4096.0) as usize;
    let (a, b) = if p_bool(p, "trim").unwrap_or(false) {
        (sorted[0], sorted[sorted.len() - 1])
    } else {
        ctx.x_range
    };
    let grid = linspace(a, b, n);
    let wsopt = if w.is_some() { Some(ws.as_slice()) } else { None };
    let dens = kde(&xs, wsopt, bw, p_str(p, "kernel").unwrap_or("gaussian"), &grid)?;
    let maxd = dens.iter().cloned().fold(0.0, f64::max);
    let nobs = xs.len() as f64;
    for (g, d) in grid.iter().zip(&dens) {
        out.push(
            &[(Aes::X, *g), (Aes::Y, *d)],
            &[
                ("density", *d),
                ("count", d * nobs),
                ("scaled", if maxd > 0.0 { d / maxd } else { 0.0 }),
                ("ndensity", if maxd > 0.0 { d / maxd } else { 0.0 }),
                ("n", nobs),
                ("bw", bw),
            ],
        );
    }
    Ok(out.frame())
}

// ── boxplot and violin ───────────────────────────────────────────────

fn group_x(f: &LFrame) -> f64 {
    f.num(Aes::X)
        .and_then(|x| x.iter().copied().find(|v| v.is_finite()))
        .unwrap_or(0.0)
}

fn boxplot(f: &LFrame, p: &Params) -> super::Result<LFrame> {
    let y = need(f, Aes::Y, "boxplot")?;
    let ys = sorted_finite(y);
    let mut out = Out::default();
    if ys.is_empty() {
        return Ok(out.frame());
    }
    let coef = p_num(p, "coef").unwrap_or(1.5);
    let (q1, q2, q3) = (
        quantile_sorted(&ys, 0.25),
        quantile_sorted(&ys, 0.5),
        quantile_sorted(&ys, 0.75),
    );
    let iqr = q3 - q1;
    let (flo, fhi) = (q1 - coef * iqr, q3 + coef * iqr);
    let inside: Vec<f64> = ys.iter().copied().filter(|v| *v >= flo && *v <= fhi).collect();
    let wlo = inside.first().copied().unwrap_or(q1);
    let whi = inside.last().copied().unwrap_or(q3);
    let outl: Vec<f64> = ys.iter().copied().filter(|v| *v < flo || *v > fhi).collect();
    let n = ys.len() as f64;
    let notch = 1.58 * iqr / n.sqrt();
    out.push(
        &[
            (Aes::X, group_x(f)),
            (Aes::Ymin, wlo),
            (Aes::Lower, q1),
            (Aes::Middle, q2),
            (Aes::Upper, q3),
            (Aes::Ymax, whi),
        ],
        &[
            ("n", n),
            ("notchlower", q2 - notch),
            ("notchupper", q2 + notch),
            ("mean", mean(&ys)),
        ],
    );
    out.outliers = Some(vec![outl]);
    Ok(out.frame())
}

fn ydensity(f: &LFrame, p: &Params) -> super::Result<LFrame> {
    let y = need(f, Aes::Y, "ydensity")?;
    let ys = sorted_finite(y);
    let mut out = Out::default();
    if ys.len() < 2 {
        return Ok(out.frame());
    }
    let rule = match p.get("bw") {
        Some(super::Param::Num(v)) => v.to_string(),
        Some(super::Param::Str(s)) => s.clone(),
        _ => "nrd0".to_string(),
    };
    let bw = bandwidth(&ys, &rule)? * p_num(p, "adjust").unwrap_or(1.0);
    let trim = p_bool(p, "trim").unwrap_or(true);
    let (a, b) = if trim {
        (ys[0], ys[ys.len() - 1])
    } else {
        (ys[0] - 3.0 * bw, ys[ys.len() - 1] + 3.0 * bw)
    };
    let grid = linspace(a, b, 512);
    let dens = kde(&ys, None, bw, p_str(p, "kernel").unwrap_or("gaussian"), &grid)?;
    let maxd = dens.iter().cloned().fold(0.0, f64::max);
    let n = ys.len() as f64;
    let x = group_x(f);
    let (q1, q2, q3) = (
        quantile_sorted(&ys, 0.25),
        quantile_sorted(&ys, 0.5),
        quantile_sorted(&ys, 0.75),
    );
    for (g, d) in grid.iter().zip(&dens) {
        out.push(
            &[(Aes::X, x), (Aes::Y, *g)],
            &[
                ("density", *d),
                ("scaled", if maxd > 0.0 { d / maxd } else { 0.0 }),
                ("count", d * n),
                ("n", n),
                ("q1", q1),
                ("median", q2),
                ("q3", q3),
            ],
        );
    }
    Ok(out.frame())
}

/// A violin layer's widths, once every group is computed: "area" (each
/// the same area), "count" (area by count) or "width" (each as wide).
pub fn violin_widths(f: &mut LFrame, scale: &str) {
    let dens = f.stat.get("density").cloned().unwrap_or_default();
    let counts = f.stat.get("count").cloned().unwrap_or_default();
    let scaled = f.stat.get("scaled").cloned().unwrap_or_default();
    let maxd = dens.iter().cloned().fold(0.0, f64::max).max(1e-300);
    let maxc = counts.iter().cloned().fold(0.0, f64::max).max(1e-300);
    let vw: Vec<f64> = (0..f.n)
        .map(|i| match scale {
            "count" => counts[i] / maxc,
            "width" => scaled[i],
            _ => dens[i] / maxd,
        })
        .collect();
    f.stat.insert("violinwidth".into(), vw);
}

// ── summary ──────────────────────────────────────────────────────────

fn summary(f: &LFrame, p: &Params) -> super::Result<LFrame> {
    let x = need(f, Aes::X, "summary")?;
    let y = need(f, Aes::Y, "summary")?;
    let fun = p_str(p, "fun").unwrap_or("mean_se");
    let level = p_num(p, "level").unwrap_or(0.95);
    let mult = p_num(p, "mult");
    let mut by: BTreeMap<i64, (f64, Vec<f64>)> = BTreeMap::new();
    for (a, b) in x.iter().zip(y) {
        if a.is_finite() && b.is_finite() {
            by.entry((a * 1e9).round() as i64).or_insert((*a, vec![])).1.push(*b);
        }
    }
    let mut out = Out::default();
    for (_, (xv, mut ys)) in by {
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = ys.len() as f64;
        let m = mean(&ys);
        let s = if ys.len() > 1 { sd(&ys) } else { 0.0 };
        let se = s / n.sqrt();
        let (mid, lo, hi) = match fun {
            "mean_se" => {
                let k = mult.unwrap_or(1.0);
                (m, m - k * se, m + k * se)
            }
            "mean_cl_normal" | "mean_ci" => {
                let t = if ys.len() > 1 { qt(0.5 + level / 2.0, n - 1.0) } else { 0.0 };
                (m, m - t * se, m + t * se)
            }
            "mean_sdl" => {
                let k = mult.unwrap_or(2.0);
                (m, m - k * s, m + k * s)
            }
            "median_hilow" => {
                let a = (1.0 - level) / 2.0;
                (
                    quantile_sorted(&ys, 0.5),
                    quantile_sorted(&ys, a),
                    quantile_sorted(&ys, 1.0 - a),
                )
            }
            "mean" => (m, m, m),
            "median" => {
                let md = quantile_sorted(&ys, 0.5);
                (md, md, md)
            }
            other => {
                return bad(format!(
                    "stat_summary: fun \"{}\" (mean_se, mean_cl_normal, mean_sdl, median_hilow, mean, median)",
                    other
                ));
            }
        };
        out.push(
            &[(Aes::X, xv), (Aes::Y, mid), (Aes::Ymin, lo), (Aes::Ymax, hi)],
            &[("n", n), ("se", se), ("sd", s)],
        );
    }
    Ok(out.frame())
}

// ── smooth ───────────────────────────────────────────────────────────

/// A smoother's fit over a grid: (x, fit, se, df) — df the residual
/// degrees of freedom the interval uses.
pub struct Fit {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub se: Vec<f64>,
    pub df: f64,
}

/// Least squares on a polynomial in x (degree `deg`), with each grid
/// point's standard error of the mean.
pub fn fit_lm(xs: &[f64], ys: &[f64], deg: usize, grid: &[f64]) -> super::Result<Fit> {
    let k = deg + 1;
    let n = xs.len();
    if n <= k {
        return bad(format!("smooth: method lm needs more than {} points in a group", k));
    }
    // centre and scale x for conditioning
    let mx = mean(xs);
    let sx = sd(xs).max(1e-300);
    let row = |x: f64| -> Vec<f64> {
        let t = (x - mx) / sx;
        (0..k).map(|j| t.powi(j as i32)).collect()
    };
    let mut xtx = vec![0.0; k * k];
    let mut xty = vec![0.0; k];
    for i in 0..n {
        let r = row(xs[i]);
        for a in 0..k {
            xty[a] += r[a] * ys[i];
            for b in 0..k {
                xtx[a * k + b] += r[a] * r[b];
            }
        }
    }
    let beta = solve(&xtx, &xty, k).ok_or_else(|| crate::OdsError::InvalidArgument("gg: smooth: the fit is singular (all x the same?)".into()))?;
    let inv = inverse(&xtx, k).unwrap_or(vec![0.0; k * k]);
    let mut rss = 0.0;
    for i in 0..n {
        let r = row(xs[i]);
        let f: f64 = r.iter().zip(&beta).map(|(a, b)| a * b).sum();
        rss += (ys[i] - f).powi(2);
    }
    let df = (n - k) as f64;
    let s2 = rss / df;
    let mut fy = Vec::with_capacity(grid.len());
    let mut fse = Vec::with_capacity(grid.len());
    for &g in grid {
        let r = row(g);
        fy.push(r.iter().zip(&beta).map(|(a, b)| a * b).sum());
        let mut q = 0.0;
        for a in 0..k {
            for b in 0..k {
                q += r[a] * inv[a * k + b] * r[b];
            }
        }
        fse.push((s2 * q.max(0.0)).sqrt());
    }
    Ok(Fit { x: grid.to_vec(), y: fy, se: fse, df })
}

/// One local fit (loess): the weighted polynomial at `x0` from the `q`
/// nearest of sorted `xs`; returns (fit, Σ l², l_self) — the operator's
/// row for the standard error and its weight on a point at x0.
fn loess_at(xs: &[f64], ys: &[f64], x0: f64, q: usize, span: f64, deg: usize) -> Option<(f64, f64, f64)> {
    let n = xs.len();
    let q = q.clamp(deg + 2, n);
    // the window of q nearest: start where x0 would sit, then slide
    let mut lo = xs.partition_point(|v| *v < x0).saturating_sub(q / 2);
    if lo + q > n {
        lo = n - q;
    }
    loop {
        if lo > 0 && (x0 - xs[lo - 1]) < (xs[lo + q - 1] - x0) {
            lo -= 1;
        } else if lo + q < n && (xs[lo + q] - x0) < (x0 - xs[lo]) {
            lo += 1;
        } else {
            break;
        }
    }
    let mut h = (x0 - xs[lo]).abs().max((xs[lo + q - 1] - x0).abs());
    if span > 1.0 {
        h *= span;
    }
    let h = h.max(1e-12) * 1.000001;
    let k = deg + 1;
    let mut xtwx = vec![0.0; k * k];
    let mut ws = Vec::with_capacity(q);
    for i in lo..lo + q {
        let u = ((xs[i] - x0) / h).abs();
        let w = if u < 1.0 { (1.0 - u * u * u).powi(3) } else { 0.0 };
        ws.push(w);
        let d = (xs[i] - x0) / h;
        let mut pa = 1.0;
        for a in 0..k {
            let mut pb = 1.0;
            for b in 0..k {
                xtwx[a * k + b] += w * pa * pb;
                pb *= d;
            }
            pa *= d;
        }
    }
    let inv = inverse(&xtwx, k)?;
    // l_i = e1ᵀ (XᵀWX)⁻¹ x_i w_i
    let mut fit = 0.0;
    let mut l2 = 0.0;
    for (j, i) in (lo..lo + q).enumerate() {
        let d = (xs[i] - x0) / h;
        let mut li = 0.0;
        let mut pb = 1.0;
        for b in 0..k {
            li += inv[b] * pb;
            pb *= d;
        }
        li *= ws[j];
        fit += li * ys[i];
        l2 += li * li;
    }
    // a point at x0 itself has weight 1 and design row e1
    let self_w = inv[0];
    Some((fit, l2, self_w))
}

/// Local regression (tricube weights, `span` of the points, degree 2),
/// with standard errors from the smoother's operator and the residual
/// variance (Cleveland's δ approximated by n − trace).
pub fn fit_loess(xs0: &[f64], ys0: &[f64], span: f64, deg: usize, grid: &[f64]) -> super::Result<Fit> {
    let n = xs0.len();
    if n < deg + 3 {
        return bad(format!("smooth: loess needs at least {} points in a group", deg + 3));
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| xs0[a].partial_cmp(&xs0[b]).unwrap());
    let xs: Vec<f64> = idx.iter().map(|&i| xs0[i]).collect();
    let ys: Vec<f64> = idx.iter().map(|&i| ys0[i]).collect();
    let q = ((span.min(1.0)) * n as f64).floor() as usize;
    let mut gy = Vec::with_capacity(grid.len());
    let mut gl2 = Vec::with_capacity(grid.len());
    let mut gself = Vec::with_capacity(grid.len());
    for &g in grid {
        let (f, l2, s) = loess_at(&xs, &ys, g, q, span, deg).unwrap_or((f64::NAN, f64::NAN, 0.0));
        gy.push(f);
        gl2.push(l2);
        gself.push(s);
    }
    // residuals and the operator's trace: exact on small data, by the
    // grid's interpolation on large
    let (rss, trace) = if n <= 1500 {
        let mut rss = 0.0;
        let mut tr = 0.0;
        for i in 0..n {
            if let Some((f, _, s)) = loess_at(&xs, &ys, xs[i], q, span, deg) {
                rss += (ys[i] - f).powi(2);
                tr += s;
            }
        }
        (rss, tr)
    } else {
        let interp = |v: &[f64], x: f64| -> f64 {
            let k = grid.partition_point(|g| *g < x).clamp(1, grid.len() - 1);
            let (a, b) = (grid[k - 1], grid[k]);
            let t = if b > a { ((x - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
            v[k - 1] * (1.0 - t) + v[k] * t
        };
        let mut rss = 0.0;
        let mut tr = 0.0;
        for i in 0..n {
            rss += (ys[i] - interp(&gy, xs[i])).powi(2);
            tr += interp(&gself, xs[i]);
        }
        (rss, tr)
    };
    let df = (n as f64 - trace).max(1.0);
    let s = (rss / df).sqrt();
    Ok(Fit {
        x: grid.to_vec(),
        y: gy,
        se: gl2.iter().map(|l| s * l.max(0.0).sqrt()).collect(),
        df,
    })
}

/// A cubic B-spline basis at `x` over equally spaced knots.
fn bspline_row(x: f64, lo: f64, hi: f64, nseg: usize) -> Vec<f64> {
    let deg = 3;
    let dx = (hi - lo) / nseg as f64;
    let nb = nseg + deg;
    let knots: Vec<f64> = (0..nb + deg + 1)
        .map(|i| lo + (i as f64 - deg as f64) * dx)
        .collect();
    // Cox–de Boor
    let x = x.clamp(lo, hi - 1e-9 * (hi - lo).max(1e-300));
    let mut b: Vec<f64> = (0..knots.len() - 1)
        .map(|i| if x >= knots[i] && x < knots[i + 1] { 1.0 } else { 0.0 })
        .collect();
    for d in 1..=deg {
        for i in 0..knots.len() - 1 - d {
            let l = if knots[i + d] > knots[i] {
                (x - knots[i]) / (knots[i + d] - knots[i]) * b[i]
            } else {
                0.0
            };
            let r = if knots[i + d + 1] > knots[i + 1] {
                (knots[i + d + 1] - x) / (knots[i + d + 1] - knots[i + 1]) * b[i + 1]
            } else {
                0.0
            };
            b[i] = l + r;
        }
    }
    b.truncate(nb);
    b
}

/// The basis size a gam takes when `k` is not given: mgcv's 10 for 30
/// or more distinct x, fewer for sparse data (a third of the distinct
/// x, at least 4), so a few points cannot pull a wiggly basis through
/// them.
pub fn gam_k(xs: &[f64]) -> usize {
    let mut u: Vec<f64> = xs.iter().copied().filter(|v| v.is_finite()).collect();
    u.sort_by(|a, b| a.partial_cmp(b).unwrap());
    u.dedup();
    (u.len() / 3).clamp(4, 10)
}

/// A penalized regression spline ("gam" lite: P-splines, a cubic basis of
/// `k` functions, a second-difference penalty whose weight REML chooses,
/// as ggplot's geom_smooth asks of mgcv), with Bayesian standard errors as
/// mgcv gives them.
pub fn fit_gam(xs: &[f64], ys: &[f64], k: usize, grid: &[f64]) -> super::Result<Fit> {
    let n = xs.len();
    let k = k.clamp(5, 40).min(n.saturating_sub(1)).max(4);
    if n < 6 {
        return bad("smooth: gam needs at least 6 points in a group");
    }
    let lo = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if hi - lo < 1e-12 {
        return bad("smooth: every x is the same");
    }
    let nseg = k - 3;
    let nb = nseg + 3;
    let mut btb = vec![0.0; nb * nb];
    let mut bty = vec![0.0; nb];
    let mut yty = 0.0;
    for i in 0..n {
        let r = bspline_row(xs[i], lo, hi, nseg);
        for a in 0..nb {
            if r[a] == 0.0 {
                continue;
            }
            bty[a] += r[a] * ys[i];
            for b in 0..nb {
                btb[a * nb + b] += r[a] * r[b];
            }
        }
        yty += ys[i] * ys[i];
    }
    // second-difference penalty DᵀD
    let mut pen = vec![0.0; nb * nb];
    for j in 0..nb - 2 {
        let d = [(j, 1.0), (j + 1, -2.0), (j + 2, 1.0)];
        for &(a, va) in &d {
            for &(b, vb) in &d {
                pen[a * nb + b] += va * vb;
            }
        }
    }
    let scale = (0..nb).map(|i| btb[i * nb + i]).sum::<f64>() / (0..nb).map(|i| pen[i * nb + i]).sum::<f64>().max(1e-300);
    // REML: the penalty's null space (a line) is fixed, the rest random;
    // σ² profiled out, −2 log Lr = (n − 2) log(rss + λ βᵀPβ) + log|BᵀB + λP|
    // − (k − 2) log λ (GCV undersmooths a few points into a wiggle)
    let rank = (nb - 2) as f64;
    let mut best: Option<(f64, Vec<f64>, Vec<f64>, f64, f64)> = None;
    for e in -16..=16 {
        let lambda = scale * 10f64.powf(e as f64 * 0.5);
        let a: Vec<f64> = btb.iter().zip(&pen).map(|(b, p)| b + lambda * p).collect();
        let Some(inv) = inverse(&a, nb) else { continue };
        let beta: Vec<f64> = (0..nb).map(|i| (0..nb).map(|j| inv[i * nb + j] * bty[j]).sum()).collect();
        // edf = tr(A⁻¹ BᵀB)
        let mut edf = 0.0;
        for i in 0..nb {
            for j in 0..nb {
                edf += inv[i * nb + j] * btb[j * nb + i];
            }
        }
        let mut btb_beta = vec![0.0; nb];
        for i in 0..nb {
            for j in 0..nb {
                btb_beta[i] += btb[i * nb + j] * beta[j];
            }
        }
        let rss = yty - 2.0 * beta.iter().zip(&bty).map(|(a, b)| a * b).sum::<f64>()
            + beta.iter().zip(&btb_beta).map(|(a, b)| a * b).sum::<f64>();
        let rss = rss.max(0.0);
        let mut pen_beta = 0.0;
        for i in 0..nb {
            for j in 0..nb {
                pen_beta += beta[i] * pen[i * nb + j] * beta[j];
            }
        }
        let Some(ld) = logdet_spd(&a, nb) else { continue };
        let reml = (n as f64 - 2.0) * (rss + lambda * pen_beta).max(1e-300).ln() + ld - rank * lambda.ln();
        if best.as_ref().is_none_or(|b| reml < b.0) {
            best = Some((reml, beta, inv, edf, rss));
        }
    }
    let (_, beta, inv, edf, rss) = best.ok_or_else(|| crate::OdsError::InvalidArgument("gg: smooth: gam did not fit".into()))?;
    let df = (n as f64 - edf).max(1.0);
    let s2 = rss / df;
    let mut fy = Vec::with_capacity(grid.len());
    let mut fse = Vec::with_capacity(grid.len());
    for &g in grid {
        let r = bspline_row(g, lo, hi, nseg);
        fy.push(r.iter().zip(&beta).map(|(a, b)| a * b).sum());
        let mut q = 0.0;
        for a in 0..nb {
            if r[a] == 0.0 {
                continue;
            }
            for b in 0..nb {
                q += r[a] * inv[a * nb + b] * r[b];
            }
        }
        fse.push((s2 * q.max(0.0)).sqrt());
    }
    Ok(Fit { x: grid.to_vec(), y: fy, se: fse, df })
}

fn smooth(f: &LFrame, p: &Params, ctx: &StatCtx) -> super::Result<LFrame> {
    let (xs, ys) = finite_pairs(need(f, Aes::X, "smooth")?, need(f, Aes::Y, "smooth")?);
    let mut out = Out::default();
    if xs.len() < 3 {
        return Ok(out.frame());
    }
    let method = p_str(p, "method").unwrap_or("auto");
    let method = if method == "auto" {
        if xs.len() < 1000 { "loess" } else { "gam" }
    } else {
        method
    };
    let n = p_num(p, "n").unwrap_or(80.0).clamp(5.0, 2000.0) as usize;
    let (a, b) = if p_bool(p, "fullrange").unwrap_or(false) {
        ctx.x_range
    } else {
        let lo = xs.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    };
    let grid = linspace(a, b, n);
    let fit = match method {
        "lm" => fit_lm(&xs, &ys, p_num(p, "degree").unwrap_or(1.0).clamp(1.0, 6.0) as usize, &grid)?,
        "loess" | "lowess" => fit_loess(
            &xs,
            &ys,
            p_num(p, "span").unwrap_or(0.75).clamp(0.05, 4.0),
            p_num(p, "degree").unwrap_or(2.0).clamp(0.0, 2.0) as usize,
            &grid,
        )?,
        "gam" => {
            let k = match p_num(p, "k") {
                Some(k) => k.max(1.0) as usize,
                None => gam_k(&xs),
            };
            fit_gam(&xs, &ys, k, &grid)?
        }
        other => return bad(format!("smooth: method \"{}\" (lm, loess, gam, auto)", other)),
    };
    let level = p_num(p, "level").unwrap_or(0.95);
    let t = qt(0.5 + level / 2.0, fit.df);
    for i in 0..fit.x.len() {
        let (y, se) = (fit.y[i], fit.se[i]);
        out.push(
            &[(Aes::X, fit.x[i]), (Aes::Y, y), (Aes::Ymin, y - t * se), (Aes::Ymax, y + t * se)],
            &[("se", se)],
        );
    }
    Ok(out.frame())
}

// ── ecdf and qq ──────────────────────────────────────────────────────

fn ecdf(f: &LFrame, ctx: &StatCtx) -> super::Result<LFrame> {
    let xs = sorted_finite(need(f, Aes::X, "ecdf")?);
    let mut out = Out::default();
    let n = xs.len() as f64;
    if xs.is_empty() {
        return Ok(out.frame());
    }
    out.push(&[(Aes::X, ctx.x_range.0.min(xs[0])), (Aes::Y, 0.0)], &[("ecdf", 0.0)]);
    let mut i = 0;
    while i < xs.len() {
        let mut j = i;
        while j + 1 < xs.len() && xs[j + 1] == xs[i] {
            j += 1;
        }
        let e = (j + 1) as f64 / n;
        out.push(&[(Aes::X, xs[i]), (Aes::Y, e)], &[("ecdf", e)]);
        i = j + 1;
    }
    out.push(&[(Aes::X, ctx.x_range.1.max(xs[xs.len() - 1])), (Aes::Y, 1.0)], &[("ecdf", 1.0)]);
    Ok(out.frame())
}

/// Plotting positions (R's ppoints): (i − a) / (n + 1 − 2a), a = 3/8 for
/// n ≤ 10, else ½.
fn ppoints(n: usize) -> Vec<f64> {
    let a = if n <= 10 { 0.375 } else { 0.5 };
    (1..=n).map(|i| (i as f64 - a) / (n as f64 + 1.0 - 2.0 * a)).collect()
}

fn qq_sample(f: &LFrame) -> super::Result<Vec<f64>> {
    let s = f.num(Aes::Sample).or_else(|| f.num(Aes::Y)).or_else(|| f.num(Aes::X));
    match s {
        Some(v) => Ok(sorted_finite(v)),
        None => bad("stat_qq needs the sample aesthetic"),
    }
}

fn qq(f: &LFrame, p: &Params) -> super::Result<LFrame> {
    let ys = qq_sample(f)?;
    let (mu, sigma) = (p_num(p, "mean").unwrap_or(0.0), p_num(p, "sd").unwrap_or(1.0));
    let mut out = Out::default();
    for (y, pp) in ys.iter().zip(ppoints(ys.len())) {
        let t = mu + sigma * qnorm(pp);
        out.push(&[(Aes::X, t), (Aes::Y, *y)], &[("theoretical", t), ("sample", *y)]);
    }
    Ok(out.frame())
}

fn qq_line(f: &LFrame, p: &Params) -> super::Result<LFrame> {
    let ys = qq_sample(f)?;
    let mut out = Out::default();
    if ys.len() < 2 {
        return Ok(out.frame());
    }
    let (mu, sigma) = (p_num(p, "mean").unwrap_or(0.0), p_num(p, "sd").unwrap_or(1.0));
    let (y1, y2) = (quantile_sorted(&ys, 0.25), quantile_sorted(&ys, 0.75));
    let (x1, x2) = (mu + sigma * qnorm(0.25), mu + sigma * qnorm(0.75));
    let slope = (y2 - y1) / (x2 - x1);
    let icpt = y1 - slope * x1;
    let pp = ppoints(ys.len());
    let (ta, tb) = (mu + sigma * qnorm(pp[0]), mu + sigma * qnorm(pp[pp.len() - 1]));
    for t in [ta, tb] {
        out.push(&[(Aes::X, t), (Aes::Y, icpt + slope * t)], &[("slope", slope), ("intercept", icpt)]);
    }
    Ok(out.frame())
}

// ── 2-D bins ─────────────────────────────────────────────────────────

fn two_bins(p: &Params) -> (f64, f64) {
    match p_nums(p, "bins") {
        Some(b) if b.len() >= 2 => (b[0], b[1]),
        Some(b) if b.len() == 1 => (b[0], b[0]),
        _ => (30.0, 30.0),
    }
}

fn bin2d(f: &LFrame, p: &Params, ctx: &StatCtx) -> super::Result<LFrame> {
    let x = need(f, Aes::X, "bin2d")?;
    let y = need(f, Aes::Y, "bin2d")?;
    let (bx, by) = two_bins(p);
    let (xl, xh) = ctx.x_range;
    let (yl, yh) = ctx.y_range;
    let (wx, wy) = match p_nums(p, "binwidth") {
        Some(w) if w.len() >= 2 => (w[0], w[1]),
        _ => ((xh - xl).max(1e-12) / bx, (yh - yl).max(1e-12) / by),
    };
    let mut m: BTreeMap<(i64, i64), f64> = BTreeMap::new();
    let wt = weights(f);
    for i in 0..x.len() {
        if x[i].is_finite() && y[i].is_finite() {
            let kx = (((x[i] - xl) / wx).floor() as i64).min((bx as i64 - 1).max(0));
            let ky = (((y[i] - yl) / wy).floor() as i64).min((by as i64 - 1).max(0));
            *m.entry((kx, ky)).or_insert(0.0) += wt.map(|w| w[i]).unwrap_or(1.0);
        }
    }
    let total: f64 = m.values().sum();
    let mut out = Out::default();
    for ((kx, ky), c) in m {
        let (x0, y0) = (xl + kx as f64 * wx, yl + ky as f64 * wy);
        out.push(
            &[
                (Aes::X, x0 + wx / 2.0),
                (Aes::Y, y0 + wy / 2.0),
                (Aes::Xmin, x0),
                (Aes::Xmax, x0 + wx),
                (Aes::Ymin, y0),
                (Aes::Ymax, y0 + wy),
            ],
            &[("count", c), ("density", c / total), ("ncount", c)],
        );
    }
    let maxc = out.stat.get("count").map(|v| v.iter().cloned().fold(0.0, f64::max)).unwrap_or(1.0);
    if let Some(nc) = out.stat.get_mut("ncount") {
        for v in nc.iter_mut() {
            *v /= maxc.max(1e-300);
        }
    }
    Ok(out.frame())
}

/// Hexagonal bins: two offset rectangular lattices, each point to the
/// nearer centre (hexbin's algorithm). Columns are `width` apart, rows
/// `height` apart (odd rows offset half a column); a hexagon is `width`
/// across and 4/3 × `height` tall (the rows' step is ¾ of it).
fn binhex(f: &LFrame, p: &Params, ctx: &StatCtx) -> super::Result<LFrame> {
    let x = need(f, Aes::X, "binhex")?;
    let y = need(f, Aes::Y, "binhex")?;
    let (bx, by) = two_bins(p);
    let (xl, xh) = ctx.x_range;
    let (yl, yh) = ctx.y_range;
    let (dx, dy) = match p_nums(p, "binwidth") {
        Some(w) if w.len() >= 2 => (w[0], w[1]),
        _ => ((xh - xl).max(1e-12) / bx, (yh - yl).max(1e-12) / by),
    };
    let mut m: BTreeMap<(i64, i64), f64> = BTreeMap::new();
    for i in 0..x.len() {
        if !(x[i].is_finite() && y[i].is_finite()) {
            continue;
        }
        let (u, v) = ((x[i] - xl) / dx, (y[i] - yl) / dy);
        // lattice A: even rows at whole columns; B: odd rows at half columns
        let (ia, ja) = (u.round(), (v / 2.0).round() * 2.0);
        let (ib, jb) = ((u - 0.5).round() + 0.5, ((v - 1.0) / 2.0).round() * 2.0 + 1.0);
        let da = (u - ia).powi(2) + 0.75 * (v - ja).powi(2);
        let db = (u - ib).powi(2) + 0.75 * (v - jb).powi(2);
        // keys in half columns, so both lattices share one map
        let key = if da <= db {
            ((ia * 2.0) as i64, ja as i64)
        } else {
            ((ib * 2.0) as i64, jb as i64)
        };
        *m.entry(key).or_insert(0.0) += weights(f).map(|w| w[i]).unwrap_or(1.0);
    }
    let total: f64 = m.values().sum();
    let mut out = Out::default();
    for ((i2, j), c) in m {
        let cx = xl + i2 as f64 / 2.0 * dx;
        let cy = yl + j as f64 * dy;
        out.push(
            &[(Aes::X, cx), (Aes::Y, cy)],
            &[("count", c), ("density", c / total), ("width", dx), ("height", dy)],
        );
    }
    Ok(out.frame())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: Vec<f64>, y: Option<Vec<f64>>) -> LFrame {
        let mut f = LFrame {
            n: x.len(),
            panel: vec![0; x.len()],
            group: vec![0; x.len()],
            ..Default::default()
        };
        f.set_num(Aes::X, x);
        if let Some(y) = y {
            f.set_num(Aes::Y, y);
        }
        f
    }

    fn ctx(lo: f64, hi: f64) -> StatCtx {
        StatCtx {
            x_range: (lo, hi),
            y_range: (0.0, 1.0),
        }
    }

    #[test]
    fn bins_count_every_point() {
        let x: Vec<f64> = (0..100).map(|i| i as f64 / 10.0).collect();
        let f = frame(x, None);
        let mut p = Params::new();
        p.insert("bins".into(), super::super::Param::Num(10.0));
        let out = bin(&f, &p, &ctx(0.0, 9.9)).unwrap();
        let counts = &out.stat["count"];
        assert_eq!(counts.iter().sum::<f64>(), 100.0);
        assert_eq!(counts.len(), 10);
        // density integrates to one
        let w = out.stat["width"][0];
        assert!((out.stat["density"].iter().sum::<f64>() * w - 1.0).abs() < 1e-9);
    }

    #[test]
    fn bandwidth_matches_r() {
        // R: bw.nrd0(c(1,2,3,4,10)) = 0.9 * min(sd, IQR/1.34) * 5^-0.2
        let v = [1.0, 2.0, 3.0, 4.0, 10.0];
        let expect = 0.9 * (2.0f64 / 1.34) * 5f64.powf(-0.2);
        assert!((bandwidth(&v, "nrd0").unwrap() - expect).abs() < 1e-12);
    }

    #[test]
    fn kde_integrates_to_one_both_paths() {
        let xs: Vec<f64> = (0..2000).map(|i| ((i * 7919) % 1000) as f64 / 100.0).collect();
        let grid = linspace(-5.0, 15.0, 512);
        for xs in [&xs[..300], &xs[..]] {
            let d = kde(xs, None, 0.5, "gaussian", &grid).unwrap();
            let area: f64 = d.iter().sum::<f64>() * (grid[1] - grid[0]);
            assert!((area - 1.0).abs() < 1e-3, "{area}");
        }
        // binned agrees with direct
        let direct = kde(&xs[..150], None, 0.5, "epanechnikov", &grid).unwrap();
        let mut many = Vec::new();
        for _ in 0..3000 {
            many.extend_from_slice(&xs[..150]);
        }
        let binned = kde(&many, None, 0.5, "epanechnikov", &grid).unwrap();
        for (a, b) in direct.iter().zip(&binned) {
            assert!((a - b).abs() < 2e-3, "{a} {b}");
        }
    }

    #[test]
    fn boxplot_five_numbers() {
        let y: Vec<f64> = (1..=9).map(|v| v as f64).chain([30.0]).collect();
        let f = frame(vec![1.0; 10], Some(y));
        let out = boxplot(&f, &Params::new()).unwrap();
        assert_eq!(out.num(Aes::Middle).unwrap()[0], 5.5);
        assert_eq!(out.num(Aes::Lower).unwrap()[0], 3.25);
        assert_eq!(out.num(Aes::Ymax).unwrap()[0], 9.0);
        assert_eq!(out.outliers.unwrap()[0], vec![30.0]);
    }

    #[test]
    fn lm_band_matches_textbook() {
        // y = 2 + 3x + noise; at the mean of x, se = s / sqrt(n)
        let x: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let noise = [0.3, -0.2, 0.1, -0.4, 0.5, -0.1, 0.2, -0.3, 0.0, 0.4, -0.5, 0.1, 0.3, -0.2, 0.2, -0.1, 0.0, 0.3, -0.3, 0.1];
        let y: Vec<f64> = x.iter().zip(noise).map(|(x, e)| 2.0 + 3.0 * x + e).collect();
        let fit = fit_lm(&x, &y, 1, &[9.5]).unwrap();
        assert!((fit.y[0] - (2.0 + 3.0 * 9.5)).abs() < 0.2);
        // s from the residuals
        let b = fit_lm(&x, &y, 1, &x).unwrap();
        let rss: f64 = y.iter().zip(&b.y).map(|(a, f)| (a - f).powi(2)).sum();
        let s = (rss / 18.0).sqrt();
        assert!((fit.se[0] - s / 20f64.sqrt()).abs() < 1e-9);
        assert_eq!(fit.df, 18.0);
    }

    #[test]
    fn loess_follows_a_curve_and_gam_too() {
        let x: Vec<f64> = (0..200).map(|i| i as f64 / 20.0).collect();
        let y: Vec<f64> = x.iter().enumerate().map(|(i, v)| v.sin() + 0.05 * ((i * 37 % 11) as f64 - 5.0) / 5.0).collect();
        let grid = linspace(0.5, 9.5, 10);
        let l = fit_loess(&x, &y, 0.3, 2, &grid).unwrap();
        let g = fit_gam(&x, &y, 10, &grid).unwrap();
        for i in 0..grid.len() {
            assert!((l.y[i] - grid[i].sin()).abs() < 0.08, "loess at {}", grid[i]);
            assert!((g.y[i] - grid[i].sin()).abs() < 0.08, "gam at {}: {}", grid[i], g.y[i]);
            assert!(l.se[i] > 0.0 && l.se[i] < 0.1);
            assert!(g.se[i] > 0.0 && g.se[i] < 0.1);
        }
        // the large path agrees with the exact one
        let mut xs2 = Vec::new();
        let mut ys2 = Vec::new();
        for k in 0..10 {
            for (a, b) in x.iter().zip(&y) {
                xs2.push(a + k as f64 * 1e-6);
                ys2.push(*b);
            }
        }
        let big = fit_loess(&xs2, &ys2, 0.3, 2, &grid).unwrap();
        for i in 0..grid.len() {
            assert!((big.y[i] - l.y[i]).abs() < 0.02);
        }
    }

    #[test]
    fn a_gam_through_a_few_noisy_points_does_not_swing() {
        // eleven points on a line with noise (the gallery's gam-sparse):
        // GCV took them for a wiggle; REML and a basis from the distinct x
        // keep the curve near the line
        let noise = |i: usize, k: usize| {
            let v = ((i as f64) * 12.9898 + (k as f64) * 78.233).sin() * 43758.5453;
            v - v.floor()
        };
        for n in [9usize, 11, 19, 40] {
            let x: Vec<f64> = (0..n).map(|i| noise(i, 20 + n) * 10.0).collect();
            let y: Vec<f64> = (0..n).map(|i| x[i] * 0.3 + noise(i, 30 + n) * 3.0).collect();
            let lo = x.iter().cloned().fold(f64::INFINITY, f64::min);
            let hi = x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let grid = linspace(lo, hi, 60);
            let g = fit_gam(&x, &y, gam_k(&x), &grid).unwrap();
            let l = fit_lm(&x, &y, 1, &grid).unwrap();
            let dev = g.y.iter().zip(&l.y).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(dev < 1.0, "n = {n}: the gam strays {dev} from the line");
        }
        assert_eq!(gam_k(&[1.0, 2.0, 3.0, 1.0, 2.0]), 4);
        assert_eq!(gam_k(&(0..21).map(|i| i as f64).collect::<Vec<_>>()), 7);
        assert_eq!(gam_k(&(0..200).map(|i| i as f64).collect::<Vec<_>>()), 10);
    }

    #[test]
    fn qq_and_ecdf() {
        let f = frame((1..=100).map(|v| v as f64).collect(), None);
        let e = ecdf(&f, &ctx(0.0, 101.0)).unwrap();
        assert_eq!(*e.num(Aes::Y).unwrap().last().unwrap(), 1.0);
        assert_eq!(e.num(Aes::Y).unwrap()[50], 0.5);
        let mut g = frame(vec![0.0; 5], None);
        g.set_num(Aes::Sample, vec![3.0, 1.0, 2.0, 5.0, 4.0]);
        let q = qq(&g, &Params::new()).unwrap();
        assert_eq!(q.num(Aes::Y).unwrap(), &vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!(q.num(Aes::X).unwrap()[2].abs() < 1e-12);
    }

    #[test]
    fn summary_cl_normal() {
        let f = frame(vec![1.0; 4], Some(vec![1.0, 2.0, 3.0, 4.0]));
        let mut p = Params::new();
        p.insert("fun".into(), super::super::Param::Str("mean_cl_normal".into()));
        let out = summary(&f, &p).unwrap();
        // R: t.test(1:4)$conf.int = 0.4457397 4.5542603
        assert!((out.num(Aes::Ymin).unwrap()[0] - 0.4457397).abs() < 1e-6);
        assert!((out.num(Aes::Ymax).unwrap()[0] - 4.5542603).abs() < 1e-6);
    }
}

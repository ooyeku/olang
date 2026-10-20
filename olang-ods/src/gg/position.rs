//! Positions: overlapping marks moved apart — stacked, filled to one,
//! dodged side by side, jittered, nudged.

use super::data::LFrame;
use super::math::Rng;
use super::{Aes, Params, bad, p_bool, p_num};
use std::collections::BTreeMap;

/// The smallest gap between distinct values (1 for whole numbers).
pub fn resolution(v: &[f64]) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return 1.0;
    }
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s.dedup();
    let mut r = f64::INFINITY;
    for w in s.windows(2) {
        r = r.min(w[1] - w[0]);
    }
    if r.is_finite() && r > 0.0 { r } else { 1.0 }
}

fn key(v: f64) -> i64 {
    (v * 1e7).round() as i64
}

pub fn apply(name: &str, f: &mut LFrame, p: &Params) -> super::Result<()> {
    match name {
        "identity" => Ok(()),
        "stack" => stack(f, false, p_bool(p, "reverse").unwrap_or(false)),
        "fill" => stack(f, true, p_bool(p, "reverse").unwrap_or(false)),
        "dodge" | "dodge2" => dodge(f, p),
        "jitter" => jitter(f, p),
        "nudge" => {
            nudge(f, p);
            Ok(())
        }
        other => bad(format!("unknown position \"{}\" (identity, stack, fill, dodge, dodge2, jitter, nudge)", other)),
    }
}

/// Stack (or fill to 1) at each x within a panel: positive values up,
/// negative down, the first group on top.
fn stack(f: &mut LFrame, fill: bool, reverse: bool) -> super::Result<()> {
    let n = f.n;
    let Some(x) = f.num(Aes::X).cloned() else { return Ok(()) };
    let Some(y) = f.num(Aes::Y).cloned() else { return Ok(()) };
    let mut order: Vec<usize> = (0..n).collect();
    // within an x, the last group stacks first (so the first ends on top)
    order.sort_by(|&a, &b| {
        let ga = f.group[a];
        let gb = f.group[b];
        if reverse { ga.cmp(&gb) } else { gb.cmp(&ga) }
    });
    let mut pos: BTreeMap<(u32, i64), f64> = BTreeMap::new();
    let mut neg: BTreeMap<(u32, i64), f64> = BTreeMap::new();
    let mut ymin = vec![f64::NAN; n];
    let mut ymax = vec![f64::NAN; n];
    for &i in &order {
        if !x[i].is_finite() || !y[i].is_finite() {
            continue;
        }
        let k = (f.panel[i], key(x[i]));
        let acc = if y[i] >= 0.0 { pos.entry(k).or_insert(0.0) } else { neg.entry(k).or_insert(0.0) };
        ymin[i] = *acc;
        *acc += y[i];
        ymax[i] = *acc;
    }
    if fill {
        for i in 0..n {
            if !x[i].is_finite() {
                continue;
            }
            let k = (f.panel[i], key(x[i]));
            let tot = pos.get(&k).copied().unwrap_or(0.0) - neg.get(&k).copied().unwrap_or(0.0);
            if tot > 0.0 {
                ymin[i] /= tot;
                ymax[i] /= tot;
            }
        }
    }
    f.set_num(Aes::Y, ymax.clone());
    f.set_num(Aes::Ymin, ymin);
    f.set_num(Aes::Ymax, ymax);
    Ok(())
}

/// Side by side at each x: the groups there share the dodge's width
/// (`dodge_width`, `gg.position_dodge(width:)`; else the mark's own),
/// each mark its own width shared among them (ggplot's dodge).
fn dodge(f: &mut LFrame, p: &Params) -> super::Result<()> {
    let n = f.n;
    let Some(x) = f.num(Aes::X).cloned() else { return Ok(()) };
    let res = resolution(&x);
    let width = p_num(p, "width");
    let dodge_width = p_num(p, "dodge_width");
    let xmin = f.num(Aes::Xmin).cloned();
    let xmax = f.num(Aes::Xmax).cloned();
    // the groups at each (panel, x)
    let mut at: BTreeMap<(u32, i64), Vec<u32>> = BTreeMap::new();
    for i in 0..n {
        if x[i].is_finite() {
            let e = at.entry((f.panel[i], key(x[i]))).or_default();
            if !e.contains(&f.group[i]) {
                e.push(f.group[i]);
            }
        }
    }
    for v in at.values_mut() {
        v.sort();
    }
    let gap = p_num(p, "padding").unwrap_or(0.0);
    let mut nx = x.clone();
    let mut nmin = vec![f64::NAN; n];
    let mut nmax = vec![f64::NAN; n];
    for i in 0..n {
        if !x[i].is_finite() {
            continue;
        }
        // the mark's own width, and the width the groups spread over
        let own = match (&xmin, &xmax, width) {
            (Some(a), Some(b), _) if a[i].is_finite() && b[i].is_finite() => b[i] - a[i],
            (_, _, Some(w)) => w,
            _ => 0.9 * res,
        };
        let w = dodge_width.unwrap_or(own);
        let gs = &at[&(f.panel[i], key(x[i]))];
        let k = gs.iter().position(|g| *g == f.group[i]).unwrap_or(0);
        let m = gs.len().max(1) as f64;
        let slot = w / m;
        let left = x[i] - w / 2.0 + k as f64 * slot;
        nx[i] = left + slot / 2.0;
        let half = own / m * (1.0 - gap) / 2.0;
        nmin[i] = nx[i] - half;
        nmax[i] = nx[i] + half;
    }
    f.set_num(Aes::X, nx);
    f.set_num(Aes::Xmin, nmin);
    f.set_num(Aes::Xmax, nmax);
    Ok(())
}

fn jitter(f: &mut LFrame, p: &Params) -> super::Result<()> {
    let mut rng = Rng::new(p_num(p, "seed").unwrap_or(1.0) as u64);
    let n = f.n;
    let wx = match (p_num(p, "width"), f.num(Aes::X)) {
        (Some(w), _) => w,
        (None, Some(x)) => 0.4 * resolution(x),
        _ => 0.0,
    };
    let wy = match (p_num(p, "height"), f.num(Aes::Y)) {
        (Some(h), _) => h,
        (None, Some(y)) => 0.4 * resolution(y),
        _ => 0.0,
    };
    let jx: Vec<f64> = (0..n).map(|_| rng.sym() * wx).collect();
    let jy: Vec<f64> = (0..n).map(|_| rng.sym() * wy).collect();
    for (a, j) in [(Aes::X, &jx), (Aes::Xmin, &jx), (Aes::Xmax, &jx), (Aes::Y, &jy), (Aes::Ymin, &jy), (Aes::Ymax, &jy)] {
        if let Some(v) = f.num(a).cloned() {
            f.set_num(a, v.iter().zip(j.iter()).map(|(v, d)| v + d).collect());
        }
    }
    Ok(())
}

fn nudge(f: &mut LFrame, p: &Params) {
    let dx = p_num(p, "nudge_x").or(p_num(p, "x")).unwrap_or(0.0);
    let dy = p_num(p, "nudge_y").or(p_num(p, "y")).unwrap_or(0.0);
    if let Some(v) = f.num(Aes::X).cloned() {
        f.set_num(Aes::X, v.iter().map(|v| v + dx).collect());
    }
    if let Some(v) = f.num(Aes::Y).cloned() {
        f.set_num(Aes::Y, v.iter().map(|v| v + dy).collect());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(x: Vec<f64>, y: Vec<f64>, g: Vec<u32>) -> LFrame {
        let n = x.len();
        let mut f = LFrame {
            n,
            panel: vec![0; n],
            group: g,
            ..Default::default()
        };
        f.set_num(Aes::X, x);
        f.set_num(Aes::Y, y);
        f
    }

    #[test]
    fn stack_and_fill() {
        let mut f = bars(vec![1.0, 1.0, 2.0, 2.0], vec![2.0, 3.0, 1.0, 1.0], vec![0, 1, 0, 1]);
        apply("stack", &mut f, &Params::new()).unwrap();
        // group 1 first (bottom), group 0 on top
        assert_eq!(f.num(Aes::Ymin).unwrap(), &vec![3.0, 0.0, 1.0, 0.0]);
        assert_eq!(f.num(Aes::Ymax).unwrap(), &vec![5.0, 3.0, 2.0, 1.0]);
        let mut g = bars(vec![1.0, 1.0], vec![1.0, 3.0], vec![0, 1]);
        apply("fill", &mut g, &Params::new()).unwrap();
        assert_eq!(g.num(Aes::Ymax).unwrap(), &vec![1.0, 0.75]);
    }

    #[test]
    fn dodge_splits_the_width() {
        let mut f = bars(vec![1.0, 1.0, 2.0], vec![1.0, 1.0, 1.0], vec![0, 1, 0]);
        apply("dodge", &mut f, &Params::new()).unwrap();
        let x = f.num(Aes::X).unwrap();
        assert!((x[0] - 0.775).abs() < 1e-9 && (x[1] - 1.225).abs() < 1e-9);
        // alone at x = 2: the whole width
        assert!((x[2] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_dodge_width_spreads_the_groups_and_each_mark_keeps_its_share_of_its_own() {
        // error bars 0.2 wide, dodged over 0.6 (ggplot's errorbar(width = 0.2,
        // position = position_dodge(0.6)))
        let mut f = bars(vec![1.0, 1.0], vec![1.0, 1.0], vec![0, 1]);
        f.set_num(Aes::Xmin, vec![0.9, 0.9]);
        f.set_num(Aes::Xmax, vec![1.1, 1.1]);
        let mut p = Params::new();
        p.insert("dodge_width".into(), super::super::Param::Num(0.6));
        apply("dodge", &mut f, &p).unwrap();
        let x = f.num(Aes::X).unwrap();
        assert!((x[0] - 0.85).abs() < 1e-9 && (x[1] - 1.15).abs() < 1e-9, "{x:?}");
        let (a, b) = (f.num(Aes::Xmin).unwrap(), f.num(Aes::Xmax).unwrap());
        assert!((b[0] - a[0] - 0.1).abs() < 1e-9, "each bar its half of 0.2");
        // a point range (no box): its width the dodge's
        let mut g = bars(vec![2.0, 2.0, 2.0], vec![1.0; 3], vec![0, 1, 2]);
        apply("dodge", &mut g, &p).unwrap();
        let x = g.num(Aes::X).unwrap();
        assert!((x[0] - 1.8).abs() < 1e-9 && (x[1] - 2.0).abs() < 1e-9 && (x[2] - 2.2).abs() < 1e-9, "{x:?}");
    }

    #[test]
    fn jitter_is_seeded() {
        let mut a = bars(vec![1.0; 50], vec![1.0; 50], vec![0; 50]);
        let mut b = a.clone();
        apply("jitter", &mut a, &Params::new()).unwrap();
        apply("jitter", &mut b, &Params::new()).unwrap();
        assert_eq!(a.num(Aes::X), b.num(Aes::X));
        assert!(a.num(Aes::X).unwrap().iter().all(|v| (v - 1.0).abs() <= 0.4));
    }
}

//! Gradient-boosted trees over binned features (the histogram method of
//! LightGBM and sklearn's `HistGradientBoosting*`).
//!
//! Each feature is cut once into at most 255 quantile bins (u8 codes).
//! A node's split search is then a pass over a per-feature histogram of
//! (Σgradient, Σhessian, count) — O(rows) to build, O(bins) to scan —
//! and a node's larger child gets its histogram by subtracting the
//! smaller child's from the parent's. Gains and leaf values are the
//! second-order ones: gain = G_L²/(H_L+λ) + G_R²/(H_R+λ) − G²/(H+λ),
//! leaf = −G/(H+λ), shrunk by the learning rate. Squared loss has a
//! unit Hessian, so for regression this is exactly variance reduction.
//!
//! Trees grow best-first (the leaf with the largest gain splits next),
//! bounded by `max_depth` and `max_leaf_nodes`. Each tree's thresholds
//! are kept as real values too, so prediction on new data needs no
//! binning. Early stopping holds out a validation fraction (stratified
//! for classification), stops after `n_iter_no_change` rounds without
//! improvement, and keeps the trees up to the best round.

use super::data::Matrix;
use super::rng::Pcg;
use super::tree::{LEAF, Tree};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Features cut into bins: `codes[f][i]` is row i's bin of feature f; a
/// row goes left at a split (f, b) when its code ≤ b, which is the same
/// as its value ≤ `edges[f][b]`.
#[derive(Clone, Debug)]
pub struct Binned {
    pub n: usize,
    pub codes: Vec<Vec<u8>>,
    pub edges: Vec<Vec<f64>>,
}

impl Binned {
    pub fn n_bins(&self, f: usize) -> usize {
        self.edges[f].len() + 1
    }
}

/// Bin edges of one feature from (a sample of) its values.
fn edges_of(values: &[f64], max_bins: usize) -> Vec<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    v.sort_unstable_by(f64::total_cmp);
    v.dedup();
    if v.len() <= max_bins {
        return v.windows(2).map(|w| w[0] / 2.0 + w[1] / 2.0).collect();
    }
    // Quantile edges over the distinct-value sample weighted by
    // occurrence: take percentiles of the full sorted sample.
    let mut all: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    all.sort_unstable_by(f64::total_cmp);
    let m = all.len();
    let mut edges = Vec::with_capacity(max_bins - 1);
    for b in 1..max_bins {
        let q = b as f64 / max_bins as f64 * (m - 1) as f64;
        let lo = q.floor() as usize;
        let hi = (lo + 1).min(m - 1);
        let e = all[lo] + (all[hi] - all[lo]) * (q - lo as f64);
        if edges.last().is_none_or(|&l: &f64| e > l) {
            edges.push(e);
        }
    }
    edges
}

#[inline]
fn code_of(edges: &[f64], v: f64) -> u8 {
    edges.partition_point(|e| *e < v) as u8
}

/// Bin every feature. Edges come from at most 200,000 rows drawn from
/// `rng` (sklearn's subsample size), codes for all rows.
pub fn bin(x: &Matrix, max_bins: usize, rng: &mut Pcg, par: bool) -> Binned {
    let max_bins = max_bins.clamp(2, 256);
    let sample: Option<Vec<usize>> = (x.n > 200_000).then(|| rng.choose(x.n, 200_000));
    let one = |c: &Vec<f64>| -> (Vec<f64>, Vec<u8>) {
        let edges = match &sample {
            Some(idx) => edges_of(&idx.iter().map(|&i| c[i]).collect::<Vec<_>>(), max_bins),
            None => edges_of(c, max_bins),
        };
        let codes = c.iter().map(|&v| code_of(&edges, v)).collect();
        (edges, codes)
    };
    let parts: Vec<(Vec<f64>, Vec<u8>)> = {
        #[cfg(feature = "parallel")]
        {
            if par {
                x.cols.par_iter().map(one).collect()
            } else {
                x.cols.iter().map(one).collect()
            }
        }
        #[cfg(not(feature = "parallel"))]
        {
            let _ = par;
            x.cols.iter().map(one).collect()
        }
    };
    let (edges, codes) = parts.into_iter().unzip();
    Binned { n: x.n, codes, edges }
}

/// Re-bin new rows with trained edges.
pub fn bin_with(x: &Matrix, edges: &[Vec<f64>]) -> Binned {
    Binned {
        n: x.n,
        codes: x
            .cols
            .iter()
            .zip(edges)
            .map(|(c, e)| c.iter().map(|&v| code_of(e, v)).collect())
            .collect(),
        edges: edges.to_vec(),
    }
}

#[derive(Clone, Debug)]
pub struct HistParams {
    pub max_depth: Option<usize>,
    pub max_leaf_nodes: Option<usize>,
    pub min_samples_leaf: usize,
    pub l2: f64,
    pub min_hessian: f64,
}

#[derive(Clone, Copy, Default)]
struct Cell {
    g: f64,
    h: f64,
    c: f64,
}

struct Split {
    gain: f64,
    feature: usize,
    bin: usize,
}

struct Leaf {
    id: usize,
    start: usize,
    end: usize,
    depth: usize,
    g: f64,
    h: f64,
    hist: Vec<Cell>,
    split: Option<Split>,
}

const MAXB: usize = 256;

/// Rows × features above which histogram building fans out over the
/// features (measured: below it the pool's hand-off costs more than the
/// pass).
const PAR_HIST_CELLS: usize = 64_000;

fn build_hist(b: &Binned, rows: &[u32], g: &[f64], h: &[f64], par: bool) -> Vec<Cell> {
    let k = b.codes.len();
    let mut hist = vec![Cell::default(); k * MAXB];
    // Gradients in node order: the feature loops then stream them.
    let gn: Vec<f64> = rows.iter().map(|&r| g[r as usize]).collect();
    let hn: Vec<f64> = rows.iter().map(|&r| h[r as usize]).collect();
    let fill = |(f, out): (usize, &mut [Cell])| {
        let codes = &b.codes[f];
        for (pos, &r) in rows.iter().enumerate() {
            let cell = &mut out[codes[r as usize] as usize];
            cell.g += gn[pos];
            cell.h += hn[pos];
            cell.c += 1.0;
        }
    };
    #[cfg(feature = "parallel")]
    {
        if par && rows.len() * k >= PAR_HIST_CELLS && k > 1 {
            hist.par_chunks_mut(MAXB).enumerate().for_each(fill);
            return hist;
        }
    }
    let _ = par;
    hist.chunks_mut(MAXB).enumerate().for_each(fill);
    hist
}

fn best_split(b: &Binned, hist: &[Cell], g: f64, h: f64, n: f64, p: &HistParams) -> Option<Split> {
    let parent = g * g / (h + p.l2);
    let msl = p.min_samples_leaf as f64;
    let mut best: Option<Split> = None;
    for f in 0..b.codes.len() {
        let nb = b.n_bins(f);
        let cells = &hist[f * MAXB..f * MAXB + nb];
        let (mut gl, mut hl, mut cl) = (0.0, 0.0, 0.0);
        for (bin, cell) in cells.iter().enumerate().take(nb - 1) {
            gl += cell.g;
            hl += cell.h;
            cl += cell.c;
            let cr = n - cl;
            if cl < msl {
                continue;
            }
            if cr < msl {
                break;
            }
            let hr = h - hl;
            if hl < p.min_hessian || hr < p.min_hessian {
                continue;
            }
            let gr = g - gl;
            let gain = gl * gl / (hl + p.l2) + gr * gr / (hr + p.l2) - parent;
            if gain > 1e-12 && best.as_ref().is_none_or(|s| gain > s.gain) {
                best = Some(Split {
                    gain,
                    feature: f,
                    bin,
                });
            }
        }
    }
    best
}

/// One histogram tree on `rows` with gradients `g`, Hessians `h`.
/// Returns the tree (leaf values already multiplied by `shrink`) and
/// its split bins (for predicting binned rows).
pub fn grow_hist(
    b: &Binned,
    g: &[f64],
    h: &[f64],
    mut rows: Vec<u32>,
    p: &HistParams,
    shrink: f64,
    par: bool,
) -> (Tree, Vec<u16>) {
    let k = b.codes.len();
    let mut tree = Tree {
        n_out: 1,
        importance: vec![0.0; k],
        ..Tree::default()
    };
    let mut bin_thr: Vec<u16> = Vec::new();
    let new_node = |tree: &mut Tree, bin_thr: &mut Vec<u16>, gs: f64, hs: f64, n: usize| -> usize {
        let id = tree.feature.len();
        tree.feature.push(LEAF);
        tree.threshold.push(f64::NAN);
        tree.left.push(LEAF);
        tree.right.push(LEAF);
        tree.value.push(-gs / (hs + p.l2) * shrink);
        tree.impurity.push(0.0);
        tree.weighted_n.push(hs);
        tree.n_samples.push(n as u32);
        bin_thr.push(u16::MAX);
        id
    };
    let (g0, h0) = rows
        .iter()
        .fold((0.0, 0.0), |(a, c), &r| (a + g[r as usize], c + h[r as usize]));
    let root = new_node(&mut tree, &mut bin_thr, g0, h0, rows.len());
    let hist = build_hist(b, &rows, g, h, par);
    let can_split = |depth: usize| p.max_depth.is_none_or(|d| depth < d);
    let mut leaves: Vec<Leaf> = vec![Leaf {
        id: root,
        start: 0,
        end: rows.len(),
        depth: 0,
        g: g0,
        h: h0,
        split: if can_split(0) {
            best_split(b, &hist, g0, h0, rows.len() as f64, p)
        } else {
            None
        },
        hist,
    }];
    let mut n_leaves = 1usize;
    loop {
        if p.max_leaf_nodes.is_some_and(|m| n_leaves >= m) {
            break;
        }
        // The open leaf with the largest gain.
        let Some(pick) = leaves
            .iter()
            .enumerate()
            .filter(|(_, l)| l.split.is_some())
            .max_by(|a, b| {
                a.1.split.as_ref().unwrap().gain.total_cmp(&b.1.split.as_ref().unwrap().gain)
            })
            .map(|(i, _)| i)
        else {
            break;
        };
        let leaf = leaves.swap_remove(pick);
        let split = leaf.split.as_ref().unwrap();
        let (f, bin) = (split.feature, split.bin);
        let codes = &b.codes[f];
        // Partition the leaf's rows: code ≤ bin to the left.
        let seg = &mut rows[leaf.start..leaf.end];
        let mut lo = 0usize;
        for i in 0..seg.len() {
            if codes[seg[i] as usize] as usize <= bin {
                seg.swap(lo, i);
                lo += 1;
            }
        }
        let mid = leaf.start + lo;
        let (gl, hl) = rows[leaf.start..mid]
            .iter()
            .fold((0.0, 0.0), |(a, c), &r| (a + g[r as usize], c + h[r as usize]));
        let (gr, hr) = (leaf.g - gl, leaf.h - hl);
        let nl = mid - leaf.start;
        let nr = leaf.end - mid;
        tree.feature[leaf.id] = f as u32;
        tree.threshold[leaf.id] = b.edges[f][bin];
        bin_thr[leaf.id] = bin as u16;
        tree.importance[f] += split.gain / 2.0;
        let lid = new_node(&mut tree, &mut bin_thr, gl, hl, nl);
        let rid = new_node(&mut tree, &mut bin_thr, gr, hr, nr);
        tree.left[leaf.id] = lid as u32;
        tree.right[leaf.id] = rid as u32;
        n_leaves += 1;
        let depth = leaf.depth + 1;
        if !can_split(depth) {
            continue;
        }
        // Histogram of the smaller child built, the larger's by
        // subtraction from the parent's.
        let left_small = nl <= nr;
        let (s_start, s_end) = if left_small { (leaf.start, mid) } else { (mid, leaf.end) };
        let small = build_hist(b, &rows[s_start..s_end], g, h, par);
        let mut large = leaf.hist;
        for (l, s) in large.iter_mut().zip(&small) {
            l.g -= s.g;
            l.h -= s.h;
            l.c -= s.c;
        }
        let (lh, rh) = if left_small { (small, large) } else { (large, small) };
        let ls = best_split(b, &lh, gl, hl, nl as f64, p);
        let rs = best_split(b, &rh, gr, hr, nr as f64, p);
        leaves.push(Leaf {
            id: lid,
            start: leaf.start,
            end: mid,
            depth,
            g: gl,
            h: hl,
            hist: lh,
            split: ls,
        });
        leaves.push(Leaf {
            id: rid,
            start: mid,
            end: leaf.end,
            depth,
            g: gr,
            h: hr,
            hist: rh,
            split: rs,
        });
    }
    (tree, bin_thr)
}

#[inline]
fn binned_leaf(t: &Tree, bin_thr: &[u16], b: &Binned, i: usize) -> usize {
    let mut node = 0usize;
    loop {
        let f = t.feature[node];
        if f == LEAF {
            return node;
        }
        node = if b.codes[f as usize][i] as u16 <= bin_thr[node] {
            t.left[node] as usize
        } else {
            t.right[node] as usize
        };
    }
}

fn add_binned(t: &Tree, bin_thr: &[u16], b: &Binned, raw: &mut [f64], par: bool) {
    let one = |(i, r): (usize, &mut f64)| {
        *r += t.value[binned_leaf(t, bin_thr, b, i)];
    };
    #[cfg(feature = "parallel")]
    {
        if par && raw.len() >= 50_000 {
            raw.par_iter_mut().enumerate().for_each(one);
            return;
        }
    }
    let _ = par;
    raw.iter_mut().enumerate().for_each(one);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loss {
    Squared,
    LogLoss,
}

#[derive(Clone, Debug)]
pub struct BoostParams {
    pub loss: Loss,
    pub n_trees: usize,
    pub learning_rate: f64,
    pub tree: HistParams,
    pub subsample: f64,
    pub max_bins: usize,
    pub early_stopping: bool,
    pub validation_fraction: f64,
    pub n_iter_no_change: usize,
    pub tol: f64,
    pub seed: u64,
}

#[derive(Clone, Debug)]
pub struct Booster {
    pub loss: Loss,
    pub init: f64,
    pub trees: Vec<Tree>,
    pub edges: Vec<Vec<f64>>,
    /// Training loss after each round (on the rows trained on).
    pub train_loss: Vec<f64>,
    /// Validation loss after each round, when early stopping held rows out.
    pub valid_loss: Vec<f64>,
    pub best_iter: usize,
}

fn loss_of(loss: Loss, y: &[f64], raw: &[f64], w: Option<&[f64]>) -> f64 {
    let mut s = 0.0;
    let mut ws = 0.0;
    for i in 0..y.len() {
        let wi = w.map(|w| w[i]).unwrap_or(1.0);
        let l = match loss {
            Loss::Squared => {
                let e = y[i] - raw[i];
                e * e
            }
            Loss::LogLoss => {
                let z = raw[i];
                let sp = if z > 0.0 { z + (-z).exp().ln_1p() } else { z.exp().ln_1p() };
                sp - y[i] * z
            }
        };
        s += wi * l;
        ws += wi;
    }
    match loss {
        // Half the mean squared error, as sklearn reports it.
        Loss::Squared => s / ws / 2.0,
        Loss::LogLoss => s / ws,
    }
}

pub fn fit(x: &Matrix, y: &[f64], w: Option<&[f64]>, p: &BoostParams, par: bool) -> Booster {
    let mut rng = Pcg::new(p.seed, 0);
    let n_all = x.n;
    // Hold out the validation rows (stratified by class for log loss).
    let (train_idx, valid_idx): (Vec<usize>, Vec<usize>) = if p.early_stopping && n_all >= 10 {
        let mut tr = Vec::new();
        let mut va = Vec::new();
        let groups: Vec<Vec<usize>> = if p.loss == Loss::LogLoss {
            vec![
                (0..n_all).filter(|&i| y[i] < 0.5).collect(),
                (0..n_all).filter(|&i| y[i] >= 0.5).collect(),
            ]
        } else {
            vec![(0..n_all).collect()]
        };
        for mut grp in groups {
            rng.shuffle(&mut grp);
            let nv = ((grp.len() as f64) * p.validation_fraction).round() as usize;
            va.extend_from_slice(&grp[..nv]);
            tr.extend_from_slice(&grp[nv..]);
        }
        tr.sort_unstable();
        va.sort_unstable();
        (tr, va)
    } else {
        ((0..n_all).collect(), Vec::new())
    };
    let xt = if valid_idx.is_empty() { x.clone() } else { x.take(&train_idx) };
    let yt: Vec<f64> = train_idx.iter().map(|&i| y[i]).collect();
    let wt: Option<Vec<f64>> = w.map(|w| train_idx.iter().map(|&i| w[i]).collect());
    let binned = bin(&xt, p.max_bins, &mut rng, par);
    let (bv, yv, wv) = if valid_idx.is_empty() {
        (None, Vec::new(), None)
    } else {
        let xv = x.take(&valid_idx);
        (
            Some(bin_with(&xv, &binned.edges)),
            valid_idx.iter().map(|&i| y[i]).collect::<Vec<f64>>(),
            w.map(|w| valid_idx.iter().map(|&i| w[i]).collect::<Vec<f64>>()),
        )
    };
    let n = xt.n;
    let wsum: f64 = wt.as_ref().map(|w| w.iter().sum()).unwrap_or(n as f64);
    let ymean = match &wt {
        Some(w) => yt.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() / wsum,
        None => yt.iter().sum::<f64>() / n as f64,
    };
    let init = match p.loss {
        Loss::Squared => ymean,
        Loss::LogLoss => {
            let m = ymean.clamp(1e-12, 1.0 - 1e-12);
            (m / (1.0 - m)).ln()
        }
    };
    let mut raw = vec![init; n];
    let mut raw_v = vec![init; yv.len()];
    let mut g = vec![0.0; n];
    let mut h = vec![1.0; n];
    let mut trees = Vec::with_capacity(p.n_trees);
    let mut train_loss = Vec::new();
    let mut valid_loss = Vec::new();
    let mut best = (f64::INFINITY, 0usize);
    for it in 0..p.n_trees {
        for i in 0..n {
            let wi = wt.as_ref().map(|w| w[i]).unwrap_or(1.0);
            match p.loss {
                Loss::Squared => {
                    g[i] = wi * (raw[i] - yt[i]);
                    h[i] = wi;
                }
                Loss::LogLoss => {
                    let pi = super::linear::sigmoid(raw[i]);
                    g[i] = wi * (pi - yt[i]);
                    h[i] = wi * (pi * (1.0 - pi)).max(1e-16);
                }
            }
        }
        let rows: Vec<u32> = if p.subsample < 1.0 {
            let m = ((n as f64) * p.subsample).round().max(1.0) as usize;
            let mut r: Vec<u32> = Pcg::child(p.seed, it as u64 + 1)
                .choose(n, m)
                .into_iter()
                .map(|i| i as u32)
                .collect();
            r.sort_unstable();
            r
        } else {
            (0..n as u32).collect()
        };
        let (tree, bin_thr) = grow_hist(&binned, &g, &h, rows, &p.tree, p.learning_rate, par);
        add_binned(&tree, &bin_thr, &binned, &mut raw, par);
        train_loss.push(loss_of(p.loss, &yt, &raw, wt.as_deref()));
        trees.push(tree);
        if let Some(bv) = &bv {
            add_binned(trees.last().unwrap(), &bin_thr, bv, &mut raw_v, par);
            let vl = loss_of(p.loss, &yv, &raw_v, wv.as_deref());
            valid_loss.push(vl);
            if vl < best.0 - p.tol * best.0.abs().min(1e300) {
                best = (vl, it);
            } else if it - best.1 >= p.n_iter_no_change {
                break;
            }
        }
    }
    let best_iter = if bv.is_some() {
        trees.truncate(best.1 + 1);
        best.1
    } else {
        trees.len().saturating_sub(1)
    };
    Booster {
        loss: p.loss,
        init,
        trees,
        edges: binned.edges,
        train_loss,
        valid_loss,
        best_iter,
    }
}

impl Booster {
    /// The raw score (mean, or log-odds) per row.
    pub fn raw(&self, x: &Matrix, par: bool) -> Vec<f64> {
        let one = |i: usize| -> f64 {
            let mut s = self.init;
            for t in &self.trees {
                s += t.value[t.leaf_of(x, i)];
            }
            s
        };
        #[cfg(feature = "parallel")]
        {
            if par && x.n * self.trees.len() >= 200_000 {
                return (0..x.n).into_par_iter().map(one).collect();
            }
        }
        let _ = par;
        (0..x.n).map(one).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_split_distinct_values() {
        let e = edges_of(&[3.0, 1.0, 2.0, 2.0, 1.0], 255);
        assert_eq!(e, vec![1.5, 2.5]);
        assert_eq!(code_of(&e, 1.0), 0);
        assert_eq!(code_of(&e, 2.0), 1);
        assert_eq!(code_of(&e, 3.0), 2);
        assert_eq!(code_of(&e, 1.5), 0);
    }

    #[test]
    fn boosting_fits_a_smooth_function() {
        let n = 2000;
        let a: Vec<f64> = (0..n).map(|i| i as f64 / n as f64 * 6.0).collect();
        let b: Vec<f64> = (0..n).map(|i| ((i * 7919) % 1000) as f64 / 1000.0).collect();
        let y: Vec<f64> = a.iter().zip(&b).map(|(a, b)| a.sin() + 2.0 * b).collect();
        let x = Matrix::new(n, vec![a, b]);
        let p = BoostParams {
            loss: Loss::Squared,
            n_trees: 200,
            learning_rate: 0.1,
            tree: HistParams {
                max_depth: Some(3),
                max_leaf_nodes: None,
                min_samples_leaf: 5,
                l2: 0.0,
                min_hessian: 1e-3,
            },
            subsample: 1.0,
            max_bins: 255,
            early_stopping: false,
            validation_fraction: 0.1,
            n_iter_no_change: 10,
            tol: 1e-7,
            seed: 1,
        };
        let m = fit(&x, &y, None, &p, false);
        let pred = m.raw(&x, false);
        let mse: f64 = pred.iter().zip(&y).map(|(p, y)| (p - y) * (p - y)).sum::<f64>() / n as f64;
        assert!(mse < 0.01, "{mse}");
        assert!(m.train_loss.windows(2).all(|w| w[1] <= w[0] + 1e-12));
    }
}

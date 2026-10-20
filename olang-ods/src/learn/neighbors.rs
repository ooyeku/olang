//! k-nearest neighbours (brute force, Euclidean) and Gaussian naive
//! Bayes.
//!
//! Neighbour search is exact: every query row's distances to every
//! training row, kept in a bounded best-k list, ties broken by training
//! order. Distances are computed a block of queries against a cache-
//! sized chunk of training rows at a time, one contiguous column per
//! feature, so the inner loop vectorizes and the training data is read
//! once per block rather than once per query. Blocks of queries fan out
//! across the pool.

use super::data::Matrix;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Query rows × training rows above which a neighbour search fans out.
pub const PAR_KNN_PAIRS: usize = 2_000_000;

#[derive(Clone, Debug)]
pub struct Knn {
    /// Training columns (column-major): a query's distances to every
    /// training row are then one vectorized pass per feature.
    pub cols: Vec<Vec<f64>>,
    pub n: usize,
    pub dims: usize,
    pub k: usize,
    pub distance_weighted: bool,
}

/// Queries per parallel task (each task reuses one distance buffer).
const QUERY_BLOCK: usize = 64;
/// Training rows per chunk: 1024 rows × a few dozen features stays in L2
/// while a block of queries passes over it.
const TRAIN_CHUNK: usize = 1024;

impl Knn {
    pub fn new(x: &Matrix, k: usize, distance_weighted: bool) -> Knn {
        Knn {
            cols: x.cols.clone(),
            n: x.n,
            dims: x.k(),
            k: k.min(x.n).max(1),
            distance_weighted,
        }
    }

    /// Fold one chunk of squared distances (training rows `base..`)
    /// into a query's bounded best list, ascending by distance; ties keep
    /// the earlier training row.
    fn fold_in(&self, best: &mut Vec<(f64, usize)>, dist: &[f64], base: usize) {
        let k = self.k;
        let mut worst = if best.len() == k { best[k - 1].0 } else { f64::INFINITY };
        for (j, &d) in dist.iter().enumerate() {
            if best.len() < k || d < worst {
                let pos = best.partition_point(|b| b.0 <= d);
                best.insert(pos, (d, base + j));
                if best.len() > k {
                    best.pop();
                }
                if best.len() == k {
                    worst = best[k - 1].0;
                }
            }
        }
    }

    /// For each query row, its k nearest training rows and their
    /// distances, nearest first. Queries go in blocks of 64 against
    /// training chunks small enough to stay in cache across the block.
    pub fn neighbours(&self, q: &Matrix, par: bool) -> Vec<Vec<(f64, usize)>> {
        let qr = q.row_major();
        let d = self.dims;
        let block = |b: usize| -> Vec<Vec<(f64, usize)>> {
            let lo = b * QUERY_BLOCK;
            let hi = ((b + 1) * QUERY_BLOCK).min(q.n);
            let mut best: Vec<Vec<(f64, usize)>> = (lo..hi).map(|_| Vec::with_capacity(self.k + 1)).collect();
            let mut dist = vec![0.0f64; TRAIN_CHUNK];
            let mut start = 0;
            while start < self.n {
                let end = (start + TRAIN_CHUNK).min(self.n);
                let len = end - start;
                for (qi, i) in (lo..hi).enumerate() {
                    let row = &qr[i * d..(i + 1) * d];
                    let dd = &mut dist[..len];
                    dd.iter_mut().for_each(|v| *v = 0.0);
                    for (m, col) in self.cols.iter().enumerate() {
                        let qm = row[m];
                        for (v, t) in dd.iter_mut().zip(&col[start..end]) {
                            let e = t - qm;
                            *v += e * e;
                        }
                    }
                    self.fold_in(&mut best[qi], dd, start);
                }
                start = end;
            }
            best.into_iter()
                .map(|v| v.into_iter().map(|(dd, j)| (dd.sqrt(), j)).collect())
                .collect()
        };
        let nb = q.n.div_ceil(QUERY_BLOCK);
        let parts: Vec<Vec<Vec<(f64, usize)>>> = {
            #[cfg(feature = "parallel")]
            {
                if par && q.n * self.n >= PAR_KNN_PAIRS {
                    (0..nb).into_par_iter().map(block).collect()
                } else {
                    (0..nb).map(block).collect()
                }
            }
            #[cfg(not(feature = "parallel"))]
            {
                let _ = par;
                (0..nb).map(block).collect()
            }
        };
        parts.into_iter().flatten().collect()
    }

    /// The weights of a neighbour list: uniform, or 1/d (an exact match
    /// takes all the weight, shared among exact matches).
    pub fn weights(&self, nb: &[(f64, usize)]) -> Vec<f64> {
        if !self.distance_weighted {
            return vec![1.0; nb.len()];
        }
        if nb.iter().any(|(d, _)| *d == 0.0) {
            return nb.iter().map(|(d, _)| if *d == 0.0 { 1.0 } else { 0.0 }).collect();
        }
        nb.iter().map(|(d, _)| 1.0 / d).collect()
    }

    /// Class probabilities: `out[c][i]`.
    pub fn predict_proba(&self, q: &Matrix, y: &[usize], n_classes: usize, par: bool) -> Vec<Vec<f64>> {
        let nbs = self.neighbours(q, par);
        let mut out = vec![vec![0.0; q.n]; n_classes];
        for (i, nb) in nbs.iter().enumerate() {
            let w = self.weights(nb);
            let total: f64 = w.iter().sum();
            for ((_, j), wj) in nb.iter().zip(&w) {
                out[y[*j]][i] += wj / total;
            }
        }
        out
    }

    pub fn predict_values(&self, q: &Matrix, y: &[f64], par: bool) -> Vec<f64> {
        let nbs = self.neighbours(q, par);
        nbs.iter()
            .map(|nb| {
                let w = self.weights(nb);
                let total: f64 = w.iter().sum();
                nb.iter().zip(&w).map(|((_, j), wj)| y[*j] * wj).sum::<f64>() / total
            })
            .collect()
    }
}

// ---------------------------------------------------------------------
// Gaussian naive Bayes
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct GaussianNb {
    /// `theta[c][j]`, `var[c][j]`.
    pub theta: Vec<Vec<f64>>,
    pub var: Vec<Vec<f64>>,
    pub prior: Vec<f64>,
    pub epsilon: f64,
}

/// sklearn's `GaussianNB`: per-class means and (biased) variances, each
/// variance plus `var_smoothing ×` the largest feature variance.
pub fn gaussian_nb(x: &Matrix, y: &[usize], n_classes: usize, w: Option<&[f64]>, var_smoothing: f64) -> GaussianNb {
    let k = x.k();
    let n = x.n as f64;
    let max_var = x
        .cols
        .iter()
        .map(|c| {
            let m = c.iter().sum::<f64>() / n;
            c.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / n
        })
        .fold(0.0f64, f64::max);
    let epsilon = var_smoothing * max_var;
    let wt = |i: usize| w.map(|w| w[i]).unwrap_or(1.0);
    let mut cw = vec![0.0; n_classes];
    let mut theta = vec![vec![0.0; k]; n_classes];
    let mut var = vec![vec![0.0; k]; n_classes];
    for (i, &c) in y.iter().enumerate() {
        cw[c] += wt(i);
    }
    for j in 0..k {
        let col = &x.cols[j];
        for (i, &c) in y.iter().enumerate() {
            theta[c][j] += wt(i) * col[i];
        }
        for c in 0..n_classes {
            if cw[c] > 0.0 {
                theta[c][j] /= cw[c];
            }
        }
        for (i, &c) in y.iter().enumerate() {
            let d = col[i] - theta[c][j];
            var[c][j] += wt(i) * d * d;
        }
        for c in 0..n_classes {
            if cw[c] > 0.0 {
                var[c][j] /= cw[c];
            }
            var[c][j] += epsilon;
        }
    }
    let total: f64 = cw.iter().sum();
    GaussianNb {
        theta,
        var,
        prior: cw.iter().map(|c| c / total).collect(),
        epsilon,
    }
}

impl GaussianNb {
    /// Posterior class probabilities: `out[c][i]`.
    pub fn predict_proba(&self, x: &Matrix) -> Vec<Vec<f64>> {
        let kc = self.prior.len();
        let mut jll = vec![vec![0.0; x.n]; kc];
        for c in 0..kc {
            let base = self.prior[c].max(1e-300).ln()
                - 0.5
                    * self.var[c]
                        .iter()
                        .map(|v| (2.0 * std::f64::consts::PI * v).ln())
                        .sum::<f64>();
            let out = &mut jll[c];
            out.iter_mut().for_each(|o| *o = base);
            for (j, col) in x.cols.iter().enumerate() {
                let m = self.theta[c][j];
                let v = self.var[c][j];
                for (o, xv) in out.iter_mut().zip(col) {
                    let d = xv - m;
                    *o -= 0.5 * d * d / v;
                }
            }
        }
        for i in 0..x.n {
            let m = (0..kc).map(|c| jll[c][i]).fold(f64::NEG_INFINITY, f64::max);
            let s: f64 = (0..kc).map(|c| (jll[c][i] - m).exp()).sum();
            for c in 0..kc {
                jll[c][i] = (jll[c][i] - m).exp() / s;
            }
        }
        jll
    }
}

//! Unsupervised models: k-means (k-means++ seeding, Lloyd iterations,
//! the best of several starts), Gaussian mixtures by EM, PCA, and the
//! silhouette score.
//!
//! k-means' assignment step is the O(rows·k·dims) part; it runs over
//! row blocks, in parallel when the caller's `par` allows, each block
//! returning its partial centre sums, so the result does not depend on
//! how many threads ran it (blocks are summed in order).

use super::data::Matrix;
use super::linalg::{chol_inverse, chol_logdet, cholesky_jittered, eigh};
use super::rng::Pcg;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

const BLOCK: usize = 4096;

/// Rows × centres × dims above which assignment fans out.
pub const PAR_ASSIGN_WORK: usize = 1_000_000;

#[derive(Clone, Debug)]
pub struct KMeans {
    /// Row-major `k × dims`.
    pub centers: Vec<f64>,
    pub k: usize,
    pub dims: usize,
    pub labels: Vec<usize>,
    pub inertia: f64,
    pub n_iter: usize,
}

#[inline]
fn sqdist(a: &[f64], b: &[f64]) -> f64 {
    let mut s = 0.0;
    for (x, y) in a.iter().zip(b) {
        let d = x - y;
        s += d * d;
    }
    s
}

/// Nearest centre of each row of `rows` (row-major), with the squared
/// distance.
fn nearest(rows: &[f64], dims: usize, centers: &[f64], k: usize, i: usize) -> (usize, f64) {
    let r = &rows[i * dims..(i + 1) * dims];
    let mut best = (0usize, f64::INFINITY);
    for c in 0..k {
        let d = sqdist(r, &centers[c * dims..(c + 1) * dims]);
        if d < best.1 {
            best = (c, d);
        }
    }
    best
}

struct Assigned {
    labels: Vec<usize>,
    dist: Vec<f64>,
    sums: Vec<f64>,
    counts: Vec<f64>,
}

fn assign(rows: &[f64], n: usize, dims: usize, centers: &[f64], k: usize, par: bool) -> Assigned {
    let block = |b: usize| -> Assigned {
        let lo = b * BLOCK;
        let hi = ((b + 1) * BLOCK).min(n);
        let mut a = Assigned {
            labels: Vec::with_capacity(hi - lo),
            dist: Vec::with_capacity(hi - lo),
            sums: vec![0.0; k * dims],
            counts: vec![0.0; k],
        };
        for i in lo..hi {
            let (c, d) = nearest(rows, dims, centers, k, i);
            a.labels.push(c);
            a.dist.push(d);
            a.counts[c] += 1.0;
            for m in 0..dims {
                a.sums[c * dims + m] += rows[i * dims + m];
            }
        }
        a
    };
    let nb = n.div_ceil(BLOCK);
    let parts: Vec<Assigned> = {
        #[cfg(feature = "parallel")]
        {
            if par && n * k * dims >= PAR_ASSIGN_WORK {
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
    let mut out = Assigned {
        labels: Vec::with_capacity(n),
        dist: Vec::with_capacity(n),
        sums: vec![0.0; k * dims],
        counts: vec![0.0; k],
    };
    for p in parts {
        out.labels.extend(p.labels);
        out.dist.extend(p.dist);
        for (o, s) in out.sums.iter_mut().zip(&p.sums) {
            *o += s;
        }
        for (o, s) in out.counts.iter_mut().zip(&p.counts) {
            *o += s;
        }
    }
    out
}

/// k-means++ seeding with sklearn's greedy local trials (2 + ln k
/// candidates per centre, the one that lowers the potential most kept).
fn plus_plus(rows: &[f64], n: usize, dims: usize, k: usize, rng: &mut Pcg) -> Vec<f64> {
    let mut centers = Vec::with_capacity(k * dims);
    let first = rng.below(n);
    centers.extend_from_slice(&rows[first * dims..(first + 1) * dims]);
    let mut closest: Vec<f64> = (0..n)
        .map(|i| sqdist(&rows[i * dims..(i + 1) * dims], &centers[0..dims]))
        .collect();
    let trials = 2 + (k as f64).ln().floor() as usize;
    for _ in 1..k {
        let pot: f64 = closest.iter().sum();
        let mut best: Option<(f64, usize, Vec<f64>)> = None;
        for _ in 0..trials {
            let target = rng.uniform() * pot;
            let mut acc = 0.0;
            let mut cand = n - 1;
            for (i, d) in closest.iter().enumerate() {
                acc += d;
                if acc > target {
                    cand = i;
                    break;
                }
            }
            let row = &rows[cand * dims..(cand + 1) * dims];
            let new_closest: Vec<f64> = (0..n)
                .map(|i| closest[i].min(sqdist(&rows[i * dims..(i + 1) * dims], row)))
                .collect();
            let p: f64 = new_closest.iter().sum();
            if best.as_ref().is_none_or(|b| p < b.0) {
                best = Some((p, cand, new_closest));
            }
        }
        let (_, cand, nc) = best.unwrap();
        centers.extend_from_slice(&rows[cand * dims..(cand + 1) * dims]);
        closest = nc;
    }
    centers
}

pub struct KMeansParams {
    pub k: usize,
    pub n_init: usize,
    pub max_iter: usize,
    pub tol: f64,
    pub seed: u64,
}

pub fn kmeans(x: &Matrix, p: &KMeansParams, par: bool) -> KMeans {
    let n = x.n;
    let dims = x.k();
    let k = p.k.min(n).max(1);
    let rows = x.row_major();
    let mean_var = x
        .cols
        .iter()
        .map(|c| {
            let m = c.iter().sum::<f64>() / n as f64;
            c.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / n as f64
        })
        .sum::<f64>()
        / dims.max(1) as f64;
    let tol = p.tol * mean_var;
    let run = |start: usize| -> KMeans {
        let mut rng = Pcg::child(p.seed, start as u64);
        let mut centers = plus_plus(&rows, n, dims, k, &mut rng);
        let mut iters = 0;
        let mut a = assign(&rows, n, dims, &centers, k, par);
        for it in 0..p.max_iter {
            iters = it + 1;
            let mut new = a.sums.clone();
            for c in 0..k {
                if a.counts[c] > 0.0 {
                    for m in 0..dims {
                        new[c * dims + m] /= a.counts[c];
                    }
                } else {
                    // An empty cluster takes the row farthest from its centre.
                    let far = a
                        .dist
                        .iter()
                        .enumerate()
                        .max_by(|x, y| x.1.total_cmp(y.1))
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    new[c * dims..(c + 1) * dims].copy_from_slice(&rows[far * dims..(far + 1) * dims]);
                    a.dist[far] = 0.0;
                }
            }
            let shift: f64 = sqdist(&new, &centers);
            centers = new;
            a = assign(&rows, n, dims, &centers, k, par);
            if shift <= tol {
                break;
            }
        }
        KMeans {
            inertia: a.dist.iter().sum(),
            centers,
            k,
            dims,
            labels: a.labels,
            n_iter: iters,
        }
    };
    let mut best: Option<KMeans> = None;
    for s in 0..p.n_init.max(1) {
        let r = run(s);
        if best.as_ref().is_none_or(|b| r.inertia < b.inertia) {
            best = Some(r);
        }
    }
    best.unwrap()
}

impl KMeans {
    pub fn predict(&self, x: &Matrix, par: bool) -> (Vec<usize>, Vec<f64>) {
        let rows = x.row_major();
        let a = assign(&rows, x.n, self.dims, &self.centers, self.k, par);
        (a.labels, a.dist)
    }
}

// ---------------------------------------------------------------------
// Silhouette
// ---------------------------------------------------------------------

/// Mean silhouette coefficient over `rows` (indices into `x`), Euclidean.
/// O(m²·dims) for m rows, parallel over rows.
pub fn silhouette(x: &Matrix, labels: &[usize], rows: &[usize], par: bool) -> f64 {
    let dims = x.k();
    let all = x.row_major();
    let m = rows.len();
    let n_labels = rows.iter().map(|&i| labels[i]).max().map(|v| v + 1).unwrap_or(0);
    let mut sizes = vec![0usize; n_labels];
    for &i in rows {
        sizes[labels[i]] += 1;
    }
    let one = |a: usize| -> f64 {
        let i = rows[a];
        let li = labels[i];
        if sizes[li] <= 1 {
            return 0.0;
        }
        let mut sums = vec![0.0; n_labels];
        for &j in rows {
            sums[labels[j]] += sqdist(&all[i * dims..(i + 1) * dims], &all[j * dims..(j + 1) * dims]).sqrt();
        }
        let ai = sums[li] / (sizes[li] - 1) as f64;
        let bi = (0..n_labels)
            .filter(|&c| c != li && sizes[c] > 0)
            .map(|c| sums[c] / sizes[c] as f64)
            .fold(f64::INFINITY, f64::min);
        if !bi.is_finite() {
            return 0.0;
        }
        (bi - ai) / ai.max(bi)
    };
    let s: f64 = {
        #[cfg(feature = "parallel")]
        {
            if par && m * m >= 1_000_000 {
                (0..m).into_par_iter().map(one).sum()
            } else {
                (0..m).map(one).sum()
            }
        }
        #[cfg(not(feature = "parallel"))]
        {
            let _ = par;
            (0..m).map(one).sum()
        }
    };
    s / m as f64
}

// ---------------------------------------------------------------------
// PCA
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Pca {
    pub mean: Vec<f64>,
    /// Column scales (1.0 unless scaled).
    pub scale: Vec<f64>,
    /// Row-major `n_components × dims`.
    pub components: Vec<f64>,
    pub n_components: usize,
    pub dims: usize,
    pub explained_variance: Vec<f64>,
    pub explained_variance_ratio: Vec<f64>,
}

/// PCA by eigen-decomposition of the covariance matrix (n − 1 divisor,
/// sklearn's `explained_variance_`). Each component's sign makes its
/// largest-magnitude loading positive (sklearn's `svd_flip` on V).
pub fn pca(x: &Matrix, n_components: usize, scale: bool, par: bool) -> Pca {
    let n = x.n;
    let dims = x.k();
    let mean: Vec<f64> = x.cols.iter().map(|c| c.iter().sum::<f64>() / n as f64).collect();
    let sd: Vec<f64> = if scale {
        x.cols
            .iter()
            .zip(&mean)
            .map(|(c, m)| {
                let v = c.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / (n as f64 - 1.0).max(1.0);
                if v > 0.0 { v.sqrt() } else { 1.0 }
            })
            .collect()
    } else {
        vec![1.0; dims]
    };
    let centred = Matrix::new(
        n,
        x.cols
            .iter()
            .zip(mean.iter().zip(&sd))
            .map(|(c, (m, s))| c.iter().map(|v| (v - m) / s).collect())
            .collect(),
    );
    let mut cov = super::linear::weighted_gram(&centred, None, false, par);
    let denom = (n as f64 - 1.0).max(1.0);
    cov.iter_mut().for_each(|v| *v /= denom);
    let (vals, vecs) = eigh(&cov, dims);
    let total: f64 = vals.iter().map(|v| v.max(0.0)).sum();
    let nc = n_components.min(dims).max(1);
    let mut components = vec![0.0; nc * dims];
    for c in 0..nc {
        let col: Vec<f64> = (0..dims).map(|j| vecs[j * dims + c]).collect();
        let big = col
            .iter()
            .copied()
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
            .unwrap_or(1.0);
        let sign = if big < 0.0 { -1.0 } else { 1.0 };
        for j in 0..dims {
            components[c * dims + j] = sign * col[j];
        }
    }
    let ev: Vec<f64> = vals[..nc].iter().map(|v| v.max(0.0)).collect();
    Pca {
        mean,
        scale: sd,
        explained_variance_ratio: ev.iter().map(|v| if total > 0.0 { v / total } else { 0.0 }).collect(),
        explained_variance: ev,
        components,
        n_components: nc,
        dims,
    }
}

impl Pca {
    /// Scores: `out[c][i]`.
    pub fn transform(&self, x: &Matrix) -> Vec<Vec<f64>> {
        let mut out = vec![vec![0.0; x.n]; self.n_components];
        for (j, col) in x.cols.iter().enumerate() {
            let (m, s) = (self.mean[j], self.scale[j]);
            for (c, o) in out.iter_mut().enumerate() {
                let w = self.components[c * self.dims + j] / s;
                for (ov, xv) in o.iter_mut().zip(col) {
                    *ov += (xv - m) * w;
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------
// Gaussian mixture
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Gmm {
    pub weights: Vec<f64>,
    /// Row-major `k × dims`.
    pub means: Vec<f64>,
    /// One row-major `dims × dims` covariance per component.
    pub covariances: Vec<Vec<f64>>,
    pub k: usize,
    pub dims: usize,
    pub log_likelihood: f64,
    pub n_iter: usize,
    pub converged: bool,
}

pub struct GmmParams {
    pub k: usize,
    pub max_iter: usize,
    pub tol: f64,
    pub reg_covar: f64,
    pub seed: u64,
}

/// Per-row log densities of each component plus log weight: `out[c][i]`.
fn log_resp(rows: &[f64], n: usize, d: usize, g: &Gmm) -> Vec<Vec<f64>> {
    let mut out = vec![vec![0.0; n]; g.k];
    for c in 0..g.k {
        let (l, _) = cholesky_jittered(&g.covariances[c], d).expect("covariance is regularized");
        let logdet = chol_logdet(&l, d);
        let base = g.weights[c].max(1e-300).ln() - 0.5 * (d as f64 * (2.0 * std::f64::consts::PI).ln() + logdet);
        let mu = &g.means[c * d..(c + 1) * d];
        let mut z = vec![0.0; d];
        for i in 0..n {
            let r = &rows[i * d..(i + 1) * d];
            // Solve L z = (x − μ); the Mahalanobis term is ‖z‖².
            for a in 0..d {
                let mut s = r[a] - mu[a];
                for b in 0..a {
                    s -= l[a * d + b] * z[b];
                }
                z[a] = s / l[a * d + a];
            }
            out[c][i] = base - 0.5 * z.iter().map(|v| v * v).sum::<f64>();
        }
    }
    out
}

/// Normalize log responsibilities in place to probabilities; returns the
/// mean log-likelihood.
fn normalize(lr: &mut [Vec<f64>], n: usize) -> f64 {
    let k = lr.len();
    let mut total = 0.0;
    for i in 0..n {
        let m = (0..k).map(|c| lr[c][i]).fold(f64::NEG_INFINITY, f64::max);
        let s: f64 = (0..k).map(|c| (lr[c][i] - m).exp()).sum();
        let lse = m + s.ln();
        total += lse;
        for c in lr.iter_mut() {
            c[i] = (c[i] - lse).exp();
        }
    }
    total / n as f64
}

fn m_step(rows: &[f64], n: usize, d: usize, resp: &[Vec<f64>], reg: f64) -> (Vec<f64>, Vec<f64>, Vec<Vec<f64>>) {
    let k = resp.len();
    let mut weights = vec![0.0; k];
    let mut means = vec![0.0; k * d];
    let mut covs = vec![vec![0.0; d * d]; k];
    for c in 0..k {
        let nk: f64 = resp[c].iter().sum::<f64>() + 10.0 * f64::EPSILON;
        weights[c] = nk / n as f64;
        for i in 0..n {
            for a in 0..d {
                means[c * d + a] += resp[c][i] * rows[i * d + a];
            }
        }
        for a in 0..d {
            means[c * d + a] /= nk;
        }
        for i in 0..n {
            let r = resp[c][i];
            for a in 0..d {
                let da = rows[i * d + a] - means[c * d + a];
                for b in 0..=a {
                    covs[c][a * d + b] += r * da * (rows[i * d + b] - means[c * d + b]);
                }
            }
        }
        for a in 0..d {
            for b in 0..=a {
                let v = covs[c][a * d + b] / nk;
                covs[c][a * d + b] = v;
                covs[c][b * d + a] = v;
            }
            covs[c][a * d + a] += reg;
        }
    }
    (weights, means, covs)
}

/// EM for a full-covariance Gaussian mixture, initialized from k-means
/// labels (sklearn's default).
pub fn gmm(x: &Matrix, p: &GmmParams, par: bool) -> Gmm {
    let n = x.n;
    let d = x.k();
    let k = p.k.min(n).max(1);
    let rows = x.row_major();
    let km = kmeans(
        x,
        &KMeansParams {
            k,
            n_init: 1,
            max_iter: 300,
            tol: 1e-4,
            seed: p.seed,
        },
        par,
    );
    let mut resp = vec![vec![0.0; n]; k];
    for (i, &l) in km.labels.iter().enumerate() {
        resp[l][i] = 1.0;
    }
    let (w, m, c) = m_step(&rows, n, d, &resp, p.reg_covar);
    let mut g = Gmm {
        weights: w,
        means: m,
        covariances: c,
        k,
        dims: d,
        log_likelihood: f64::NEG_INFINITY,
        n_iter: 0,
        converged: false,
    };
    for it in 0..p.max_iter {
        let mut lr = log_resp(&rows, n, d, &g);
        let ll = normalize(&mut lr, n);
        let change = ll - g.log_likelihood;
        g.log_likelihood = ll;
        g.n_iter = it + 1;
        if change.abs() < p.tol {
            g.converged = true;
            break;
        }
        let (w, m, c) = m_step(&rows, n, d, &lr, p.reg_covar);
        g.weights = w;
        g.means = m;
        g.covariances = c;
    }
    g
}

impl Gmm {
    pub fn predict_proba(&self, x: &Matrix) -> Vec<Vec<f64>> {
        let rows = x.row_major();
        let mut lr = log_resp(&rows, x.n, self.dims, self);
        normalize(&mut lr, x.n);
        lr
    }

    /// Mean log-likelihood per row of `x`.
    pub fn score(&self, x: &Matrix) -> f64 {
        let rows = x.row_major();
        let mut lr = log_resp(&rows, x.n, self.dims, self);
        normalize(&mut lr, x.n)
    }

    pub fn n_parameters(&self) -> usize {
        let d = self.dims;
        self.k * d * (d + 1) / 2 + self.k * d + self.k - 1
    }

    /// Precision matrices (for callers who want them).
    pub fn precisions(&self) -> Vec<Vec<f64>> {
        self.covariances
            .iter()
            .map(|c| {
                let (l, _) = cholesky_jittered(c, self.dims).expect("regularized");
                chol_inverse(&l, self.dims)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blobs() -> Matrix {
        let mut rng = Pcg::new(5, 0);
        let mut a = Vec::new();
        let mut b = Vec::new();
        for c in 0..3 {
            for _ in 0..100 {
                a.push(c as f64 * 10.0 + rng.normal());
                b.push((c % 2) as f64 * 10.0 + rng.normal());
            }
        }
        Matrix::new(300, vec![a, b])
    }

    #[test]
    fn kmeans_finds_three_blobs() {
        let x = blobs();
        let km = kmeans(&x, &KMeansParams { k: 3, n_init: 4, max_iter: 100, tol: 1e-4, seed: 1 }, false);
        for c in 0..3 {
            let l = km.labels[c * 100];
            assert!(km.labels[c * 100..(c + 1) * 100].iter().all(|&v| v == l));
        }
        assert!(km.inertia < 300.0 * 2.5);
        let s = silhouette(&x, &km.labels, &(0..300).collect::<Vec<_>>(), false);
        assert!(s > 0.7, "{s}");
    }

    #[test]
    fn gmm_separates_blobs() {
        let x = blobs();
        let g = gmm(&x, &GmmParams { k: 3, max_iter: 100, tol: 1e-6, reg_covar: 1e-6, seed: 2 }, false);
        assert!(g.converged);
        let w: f64 = g.weights.iter().sum();
        assert!((w - 1.0).abs() < 1e-9);
        assert!(g.weights.iter().all(|w| (w - 1.0 / 3.0).abs() < 0.02));
    }

    #[test]
    fn pca_first_axis_carries_the_spread() {
        let n = 200;
        let t: Vec<f64> = (0..n).map(|i| i as f64 / 10.0).collect();
        let a: Vec<f64> = t.iter().map(|v| 2.0 * v).collect();
        let b: Vec<f64> = t.iter().enumerate().map(|(i, v)| v + if i % 2 == 0 { 0.1 } else { -0.1 }).collect();
        let x = Matrix::new(n, vec![a, b]);
        let p = pca(&x, 2, false, false);
        assert!(p.explained_variance_ratio[0] > 0.999);
        assert!(p.components[0] > 0.0 && p.components[1] > 0.0);
        let s = p.transform(&x);
        let m: f64 = s[0].iter().sum::<f64>() / n as f64;
        assert!(m.abs() < 1e-9);
    }
}

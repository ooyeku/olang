//! Linear models: OLS with inference, ridge, the lasso and elastic net
//! (coordinate descent on the Gram matrix), and logistic regression —
//! binary (IRLS: Newton on the weighted Gram matrix) and multinomial
//! (Newton on the softmax log-likelihood).
//!
//! Every model is tall and skinny (many rows, tens of columns), so the
//! one O(rows) step per iteration is a weighted Gram matrix: k(k+1)/2
//! independent dot products, fanned out across the pool when the
//! caller's `par` says the size pays. The k×k solves are in-crate
//! Cholesky. Results match statsmodels' OLS/Logit/MNLogit and sklearn's
//! Ridge/Lasso/ElasticNet/LogisticRegression to their own tolerances.

use super::data::{Matrix, Result, bad};
use super::linalg::{chol_inverse, chol_solve, cholesky, cholesky_jittered, dot};
use crate::dist;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Rows × columns above which a Gram matrix's dot products fan out.
pub const PAR_GRAM_WORK: usize = 50_000;

/// `XᵀWX` for the columns `cols` (an intercept column of ones prepended
/// when `intercept`), weights `w` (None = all ones). Row-major p×p.
pub fn weighted_gram(x: &Matrix, w: Option<&[f64]>, intercept: bool, par: bool) -> Vec<f64> {
    let k = x.k();
    let p = k + usize::from(intercept);
    let n = x.n;
    // Below about 200k multiply-adds per Gram row the pool's hand-off
    // costs more than the dot products.
    let par = par && n * p >= PAR_GRAM_WORK;
    let ones;
    let wcol: &[f64] = match w {
        Some(w) => w,
        None => {
            ones = vec![1.0; n];
            &ones
        }
    };
    // Column a of the design, times the weights: the left factor of
    // every dot product in its row of the Gram matrix.
    let col = |a: usize| -> &[f64] {
        if intercept {
            if a == 0 { wcol } else { &x.cols[a - 1] }
        } else {
            &x.cols[a]
        }
    };
    let weighted: Vec<Vec<f64>> = (0..p)
        .map(|a| {
            if intercept && a == 0 {
                wcol.to_vec()
            } else {
                col(a).iter().zip(wcol).map(|(v, w)| v * w).collect()
            }
        })
        .collect();
    let pairs: Vec<(usize, usize)> = (0..p).flat_map(|i| (0..=i).map(move |j| (i, j))).collect();
    let at = |&(i, j): &(usize, usize)| -> f64 {
        if intercept && j == 0 {
            // Σ w·x_i: the weighted column's plain sum.
            weighted[i].iter().sum()
        } else {
            dot(&weighted[i], col(j))
        }
    };
    let vals: Vec<f64> = {
        #[cfg(feature = "parallel")]
        {
            if par {
                pairs.par_iter().map(at).collect()
            } else {
                pairs.iter().map(at).collect()
            }
        }
        #[cfg(not(feature = "parallel"))]
        {
            let _ = par;
            pairs.iter().map(at).collect()
        }
    };
    let mut g = vec![0.0; p * p];
    for (&(i, j), v) in pairs.iter().zip(vals) {
        g[i * p + j] = v;
        g[j * p + i] = v;
    }
    g
}

/// `XᵀWv` (intercept entry first when `intercept`).
pub fn weighted_xtv(x: &Matrix, w: Option<&[f64]>, v: &[f64], intercept: bool) -> Vec<f64> {
    let wv: Vec<f64> = match w {
        Some(w) => v.iter().zip(w).map(|(a, b)| a * b).collect(),
        None => v.to_vec(),
    };
    let mut out = Vec::with_capacity(x.k() + 1);
    if intercept {
        out.push(wv.iter().sum());
    }
    for c in &x.cols {
        out.push(dot(c, &wv));
    }
    out
}

/// `Xβ` (+ β₀ when `intercept`).
pub fn linear_predict(x: &Matrix, beta: &[f64], intercept: bool) -> Vec<f64> {
    let off = usize::from(intercept);
    let mut out = vec![if intercept { beta[0] } else { 0.0 }; x.n];
    for (j, c) in x.cols.iter().enumerate() {
        let b = beta[j + off];
        if b == 0.0 {
            continue;
        }
        for (o, v) in out.iter_mut().zip(c) {
            *o += b * v;
        }
    }
    out
}

// ---------------------------------------------------------------------
// OLS
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct OlsFit {
    /// Intercept first (when fitted), then one per feature.
    pub coef: Vec<f64>,
    pub se: Vec<f64>,
    pub t: Vec<f64>,
    pub p: Vec<f64>,
    /// (XᵀWX)⁻¹, row-major — prediction intervals need all of it.
    pub xtx_inv: Vec<f64>,
    pub sigma2: f64,
    pub df_resid: f64,
    pub n: usize,
    pub r2: f64,
    pub adj_r2: f64,
    pub sse: f64,
    pub log_lik: f64,
    pub intercept: bool,
    pub fitted: Vec<f64>,
    /// `y − fitted`, row by row.
    pub resid: Vec<f64>,
}

pub fn ols(x: &Matrix, y: &[f64], w: Option<&[f64]>, intercept: bool, par: bool) -> Result<OlsFit> {
    let n = x.n;
    let p = x.k() + usize::from(intercept);
    if n <= p {
        return Err(bad(format!(
            "linear: needs more complete rows ({}) than coefficients ({})",
            n, p
        )));
    }
    let g = weighted_gram(x, w, intercept, par);
    let xty = weighted_xtv(x, w, y, intercept);
    let l = cholesky(&g, p).ok_or_else(|| {
        bad("linear: the design is singular (a constant, duplicated or collinear column?)")
    })?;
    let coef = chol_solve(&l, p, &xty);
    let xtx_inv = chol_inverse(&l, p);
    let fitted = linear_predict(x, &coef, intercept);
    let wsum: f64 = w.map(|w| w.iter().sum()).unwrap_or(n as f64);
    let ymean = match w {
        Some(w) => y.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() / wsum,
        None => y.iter().sum::<f64>() / n as f64,
    };
    let mut sse = 0.0;
    let mut sst = 0.0;
    for i in 0..n {
        let wi = w.map(|w| w[i]).unwrap_or(1.0);
        let e = y[i] - fitted[i];
        sse += wi * e * e;
        let d = if intercept { y[i] - ymean } else { y[i] };
        sst += wi * d * d;
    }
    let df_resid = (n - p) as f64;
    let sigma2 = sse / df_resid;
    let se: Vec<f64> = (0..p)
        .map(|i| (sigma2 * xtx_inv[i * p + i]).sqrt())
        .collect();
    let t: Vec<f64> = coef.iter().zip(&se).map(|(c, s)| c / s).collect();
    let pv = t
        .iter()
        .map(|&tv| t_two_sided(tv, df_resid))
        .collect::<Result<Vec<f64>>>()?;
    let r2 = if sst == 0.0 {
        f64::NAN
    } else {
        1.0 - sse / sst
    };
    let dfm = if intercept { n as f64 - 1.0 } else { n as f64 };
    let adj_r2 = 1.0 - (1.0 - r2) * dfm / df_resid;
    let nf = n as f64;
    let log_lik = -nf / 2.0 * ((2.0 * std::f64::consts::PI).ln() + (sse / nf).ln() + 1.0);
    Ok(OlsFit {
        coef,
        se,
        t,
        p: pv,
        xtx_inv,
        sigma2,
        df_resid,
        n,
        r2,
        adj_r2,
        sse,
        log_lik,
        intercept,
        resid: y.iter().zip(&fitted).map(|(a, b)| a - b).collect(),
        fitted,
    })
}

pub fn t_two_sided(t: f64, df: f64) -> Result<f64> {
    if !t.is_finite() {
        return Ok(if t.is_nan() { f64::NAN } else { 0.0 });
    }
    // The lower tail at −|t|: no cancellation, so tiny p-values survive.
    Ok(2.0 * dist::t_cdf(-t.abs(), df)?)
}

pub fn z_two_sided(z: f64) -> f64 {
    if !z.is_finite() {
        return if z.is_nan() { f64::NAN } else { 0.0 };
    }
    2.0 * dist::norm_cdf(-z.abs(), 0.0, 1.0).unwrap_or(0.0)
}

impl OlsFit {
    /// The standard error of the fitted mean at each row of `x`
    /// (`sqrt(σ² x₀ᵀ(XᵀX)⁻¹x₀)`).
    pub fn mean_se(&self, x: &Matrix) -> Vec<f64> {
        let p = self.coef.len();
        let off = usize::from(self.intercept);
        let mut row = vec![0.0; p];
        (0..x.n)
            .map(|i| {
                if self.intercept {
                    row[0] = 1.0;
                }
                for (j, c) in x.cols.iter().enumerate() {
                    row[j + off] = c[i];
                }
                let mut q = 0.0;
                for a in 0..p {
                    let mut s = 0.0;
                    for b in 0..p {
                        s += self.xtx_inv[a * p + b] * row[b];
                    }
                    q += row[a] * s;
                }
                (self.sigma2 * q).sqrt()
            })
            .collect()
    }
}

// ---------------------------------------------------------------------
// Ridge, lasso, elastic net
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Penalized {
    /// Intercept first, then one per feature.
    pub coef: Vec<f64>,
    pub n_iter: usize,
}

/// Weighted means of each column and of y (the centring a fit with an
/// intercept does before penalizing).
fn centres(x: &Matrix, y: &[f64]) -> (Vec<f64>, f64) {
    let n = x.n as f64;
    let xm = x.cols.iter().map(|c| c.iter().sum::<f64>() / n).collect();
    let ym = y.iter().sum::<f64>() / n;
    (xm, ym)
}

/// Ridge: minimizes ‖y − Xβ − β₀‖² + α‖β‖² (sklearn's `Ridge`), the
/// intercept unpenalized.
pub fn ridge(x: &Matrix, y: &[f64], alpha: f64, intercept: bool, par: bool) -> Result<Penalized> {
    let k = x.k();
    let (g, xty, xm, ym) = centred_gram(x, y, intercept, par);
    let mut a = g;
    for j in 0..k {
        a[j * k + j] += alpha;
    }
    let l = cholesky(&a, k)
        .ok_or_else(|| bad("ridge: the system is singular (alpha = 0 with collinear columns?)"))?;
    let b = chol_solve(&l, k, &xty);
    let b0 = if intercept { ym - dot(&xm, &b) } else { 0.0 };
    let mut coef = vec![b0];
    coef.extend(b);
    Ok(Penalized { coef, n_iter: 1 })
}

/// The centred Gram matrix XcᵀXc and Xcᵀyc, from one uncentred pass:
/// XcᵀXc = XᵀX − n·x̄x̄ᵀ.
fn centred_gram(
    x: &Matrix,
    y: &[f64],
    intercept: bool,
    par: bool,
) -> (Vec<f64>, Vec<f64>, Vec<f64>, f64) {
    let k = x.k();
    let n = x.n as f64;
    let g = weighted_gram(x, None, false, par);
    let xty = weighted_xtv(x, None, y, false);
    if !intercept {
        return (g, xty, vec![0.0; k], 0.0);
    }
    let (xm, ym) = centres(x, y);
    let mut gc = g;
    for i in 0..k {
        for j in 0..k {
            gc[i * k + j] -= n * xm[i] * xm[j];
        }
    }
    let xtyc: Vec<f64> = (0..k).map(|j| xty[j] - n * xm[j] * ym).collect();
    (gc, xtyc, xm, ym)
}

/// Elastic net by cyclic coordinate descent on the Gram matrix:
/// minimizes 1/(2n)‖y − Xβ − β₀‖² + α·ρ‖β‖₁ + α(1−ρ)/2‖β‖² (sklearn's
/// `ElasticNet`; ρ = 1 is the lasso). Each sweep is O(k²), independent
/// of the row count.
// The solver's knobs are its arguments, as sklearn's are.
#[allow(clippy::too_many_arguments)]
pub fn elastic_net(
    x: &Matrix,
    y: &[f64],
    alpha: f64,
    l1_ratio: f64,
    intercept: bool,
    max_iter: usize,
    tol: f64,
    par: bool,
) -> Result<Penalized> {
    if !(0.0..=1.0).contains(&l1_ratio) {
        return Err(bad("elastic_net: l1_ratio must lie in [0, 1]"));
    }
    if alpha < 0.0 {
        return Err(bad("elastic_net: alpha must be >= 0"));
    }
    let k = x.k();
    let n = x.n as f64;
    let (g, xty, xm, ym) = centred_gram(x, y, intercept, par);
    let l1 = alpha * l1_ratio * n;
    let l2 = alpha * (1.0 - l1_ratio) * n;
    let mut b = vec![0.0f64; k];
    // q_j = (XᵀX β)_j, kept up to date as coordinates move.
    let mut q = vec![0.0f64; k];
    let mut iters = 0;
    for it in 0..max_iter {
        iters = it + 1;
        let mut max_delta = 0.0f64;
        let mut max_b = 0.0f64;
        for j in 0..k {
            let gjj = g[j * k + j];
            if gjj == 0.0 {
                continue;
            }
            let old = b[j];
            let rho = xty[j] - q[j] + gjj * old;
            let new = soft(rho, l1) / (gjj + l2);
            if new != old {
                let d = new - old;
                for m in 0..k {
                    q[m] += g[m * k + j] * d;
                }
                b[j] = new;
                max_delta = max_delta.max(d.abs());
            }
            max_b = max_b.max(new.abs());
        }
        if max_b == 0.0 || max_delta <= tol * max_b {
            break;
        }
    }
    let b0 = if intercept { ym - dot(&xm, &b) } else { 0.0 };
    let mut coef = vec![b0];
    coef.extend(b);
    Ok(Penalized {
        coef,
        n_iter: iters,
    })
}

#[inline]
fn soft(z: f64, t: f64) -> f64 {
    if z > t {
        z - t
    } else if z < -t {
        z + t
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------
// Logistic regression
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct LogitFit {
    /// Intercept first.
    pub coef: Vec<f64>,
    pub se: Vec<f64>,
    pub z: Vec<f64>,
    pub p: Vec<f64>,
    pub n_iter: usize,
    pub converged: bool,
    pub log_lik: f64,
    pub null_log_lik: f64,
    pub n: usize,
}

#[inline]
pub fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        let e = z.exp();
        e / (1.0 + e)
    }
}

/// Binary logistic regression by IRLS (Newton–Raphson): minimizes
/// Σ wᵢ·logloss(yᵢ, σ(xᵢβ)) + λ/2‖β₁..‖², the intercept unpenalized.
/// λ = 0 is statsmodels' `Logit`; λ = 1/C is sklearn's
/// `LogisticRegression(C=C)` with its default L2 penalty.
pub fn logistic(
    x: &Matrix,
    y: &[f64],
    w: Option<&[f64]>,
    lambda: f64,
    max_iter: usize,
    tol: f64,
    par: bool,
) -> Result<LogitFit> {
    let n = x.n;
    let p = x.k() + 1;
    let wt = |i: usize| w.map(|w| w[i]).unwrap_or(1.0);
    let penalty = |beta: &[f64]| lambda / 2.0 * beta[1..].iter().map(|b| b * b).sum::<f64>();
    // The negative log-likelihood at a linear predictor, summed in fixed
    // pieces so it is the same number on any thread count.
    let nll = |eta: &[f64]| -> f64 { piece_sum(n, par, |i| wt(i) * log1pexp_loss(eta[i], y[i])) };
    let mut beta = vec![0.0f64; p];
    // Start the intercept at the weighted log-odds: one Newton step fewer.
    let wsum: f64 = (0..n).map(wt).sum();
    let ybar = (0..n).map(|i| wt(i) * y[i]).sum::<f64>() / wsum;
    if ybar > 0.0 && ybar < 1.0 {
        beta[0] = (ybar / (1.0 - ybar)).ln();
    }
    let mut eta = linear_predict(x, &beta, true);
    let null_log_lik = -nll(&eta);
    let mut cur = -null_log_lik + penalty(&beta);
    let mut converged = false;
    let mut iters = 0;
    let mut resid = vec![0.0; n];
    let mut hw = vec![0.0; n];
    for it in 0..max_iter {
        iters = it + 1;
        elementwise(&eta, &mut resid, &mut hw, par, |i, e| {
            let pi = sigmoid(e);
            (wt(i) * (pi - y[i]), wt(i) * pi * (1.0 - pi))
        });
        let mut grad = weighted_xtv(x, None, &resid, true);
        let mut h = weighted_gram(x, Some(&hw), true, par);
        for j in 1..p {
            grad[j] += lambda * beta[j];
            h[j * p + j] += lambda;
        }
        let (l, _) = cholesky_jittered(&h, p).ok_or_else(|| {
            bad("logistic: the Hessian is singular (separable or constant columns?)")
        })?;
        let step = chol_solve(&l, p, &grad);
        // Step-halving keeps Newton monotone on separable data.
        let mut t = 1.0;
        let (next, next_eta, next_loss) = loop {
            let next: Vec<f64> = beta.iter().zip(&step).map(|(b, s)| b - t * s).collect();
            let ne = linear_predict(x, &next, true);
            let nl = nll(&ne) + penalty(&next);
            if nl <= cur + 1e-12 * cur.abs().max(1.0) || t < 1e-8 {
                break (next, ne, nl);
            }
            t *= 0.5;
        };
        let max_step = step.iter().map(|s| (t * s).abs()).fold(0.0, f64::max);
        let improvement = cur - next_loss;
        beta = next;
        eta = next_eta;
        cur = next_loss;
        if max_step < tol || improvement.abs() < tol * 1e-3 * cur.abs().max(1.0) {
            converged = true;
            break;
        }
    }
    // Inference from the (penalized) Hessian at the optimum.
    elementwise(&eta, &mut resid, &mut hw, par, |i, e| {
        let pi = sigmoid(e);
        (0.0, wt(i) * pi * (1.0 - pi))
    });
    let mut h = weighted_gram(x, Some(&hw), true, par);
    for j in 1..p {
        h[j * p + j] += lambda;
    }
    let se: Vec<f64> = match cholesky(&h, p) {
        Some(l) => {
            let inv = chol_inverse(&l, p);
            (0..p).map(|i| inv[i * p + i].sqrt()).collect()
        }
        None => vec![f64::NAN; p],
    };
    let z: Vec<f64> = beta.iter().zip(&se).map(|(b, s)| b / s).collect();
    let pv: Vec<f64> = z.iter().map(|&z| z_two_sided(z)).collect();
    let log_lik = -nll(&eta);
    Ok(LogitFit {
        coef: beta,
        se,
        z,
        p: pv,
        n_iter: iters,
        converged,
        log_lik,
        null_log_lik,
        n,
    })
}

/// Rows per piece of an elementwise pass; partial sums are added in
/// piece order whether or not the pieces ran in parallel.
const PIECE: usize = 8192;
/// Rows above which elementwise passes fan out.
#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
const PAR_ROWS: usize = 32_768;

/// Σ f(i) over 0..n in fixed pieces (deterministic on any thread count).
fn piece_sum(n: usize, par: bool, f: impl Fn(usize) -> f64 + Sync) -> f64 {
    let piece = |c: usize| -> f64 { (c * PIECE..((c + 1) * PIECE).min(n)).map(&f).sum() };
    let pieces = n.div_ceil(PIECE);
    let parts: Vec<f64> = {
        #[cfg(feature = "parallel")]
        {
            if par && n >= PAR_ROWS {
                (0..pieces).into_par_iter().map(piece).collect()
            } else {
                (0..pieces).map(piece).collect()
            }
        }
        #[cfg(not(feature = "parallel"))]
        {
            let _ = par;
            (0..pieces).map(piece).collect()
        }
    };
    parts.iter().sum()
}

/// (a[i], b[i]) = f(i, src[i]).
fn elementwise(
    src: &[f64],
    a: &mut [f64],
    b: &mut [f64],
    par: bool,
    f: impl Fn(usize, f64) -> (f64, f64) + Sync,
) {
    let one = |(c, (ac, bc)): (usize, (&mut [f64], &mut [f64]))| {
        for k in 0..ac.len() {
            let i = c * PIECE + k;
            let (u, v) = f(i, src[i]);
            ac[k] = u;
            bc[k] = v;
        }
    };
    #[cfg(feature = "parallel")]
    {
        if par && src.len() >= PAR_ROWS {
            a.par_chunks_mut(PIECE)
                .zip(b.par_chunks_mut(PIECE))
                .enumerate()
                .for_each(one);
            return;
        }
    }
    let _ = par;
    a.chunks_mut(PIECE)
        .zip(b.chunks_mut(PIECE))
        .enumerate()
        .for_each(one);
}

/// −[y log σ(η) + (1−y) log(1−σ(η))], computed without overflow.
#[inline]
fn log1pexp_loss(eta: f64, y: f64) -> f64 {
    // log(1 + e^η) − yη
    let softplus = if eta > 0.0 {
        eta + (-eta).exp().ln_1p()
    } else {
        eta.exp().ln_1p()
    };
    softplus - y * eta
}

#[derive(Clone, Debug)]
pub struct SoftmaxFit {
    /// `coef[c]` is class c's intercept then features; centred so each
    /// coefficient sums to zero over the classes.
    pub coef: Vec<Vec<f64>>,
    pub n_iter: usize,
    pub converged: bool,
    pub log_lik: f64,
}

/// Class probabilities for each row: `out[c][i]`.
pub fn softmax_predict(x: &Matrix, coef: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let etas: Vec<Vec<f64>> = coef.iter().map(|b| linear_predict(x, b, true)).collect();
    let kc = coef.len();
    let mut out = vec![vec![0.0; x.n]; kc];
    for i in 0..x.n {
        let m = (0..kc)
            .map(|c| etas[c][i])
            .fold(f64::NEG_INFINITY, f64::max);
        let mut s = 0.0;
        for c in 0..kc {
            let e = (etas[c][i] - m).exp();
            out[c][i] = e;
            s += e;
        }
        for c in out.iter_mut() {
            c[i] /= s;
        }
    }
    out
}

/// Multinomial logistic regression by Newton's method on the full
/// softmax Hessian: minimizes Σ wᵢ(−log p_{i,yᵢ}) + λ/2 Σ_c‖β_c,1..‖².
/// With λ = 0 the symmetric parameterization has a flat direction;
/// a vanishing ridge removes it and the result is centred, which leaves
/// every probability unchanged (statsmodels' MNLogit up to its choice of
/// reference class).
#[allow(clippy::too_many_arguments)]
pub fn softmax(
    x: &Matrix,
    y: &[usize],
    n_classes: usize,
    w: Option<&[f64]>,
    lambda: f64,
    max_iter: usize,
    tol: f64,
    par: bool,
) -> Result<SoftmaxFit> {
    let n = x.n;
    let p = x.k() + 1;
    let kc = n_classes;
    let dim = kc * p;
    let wt = |i: usize| w.map(|w| w[i]).unwrap_or(1.0);
    let objective = |coef: &[Vec<f64>]| -> f64 {
        let probs = softmax_predict(x, coef);
        let mut l = 0.0;
        for i in 0..n {
            l -= wt(i) * probs[y[i]][i].max(1e-300).ln();
        }
        l + lambda / 2.0
            * coef
                .iter()
                .map(|b| b[1..].iter().map(|v| v * v).sum::<f64>())
                .sum::<f64>()
    };
    let mut coef = vec![vec![0.0f64; p]; kc];
    let mut cur = objective(&coef);
    let mut converged = false;
    let mut iters = 0;
    for it in 0..max_iter {
        iters = it + 1;
        let probs = softmax_predict(x, &coef);
        let mut grad = vec![0.0; dim];
        for c in 0..kc {
            let r: Vec<f64> = (0..n)
                .map(|i| wt(i) * (probs[c][i] - if y[i] == c { 1.0 } else { 0.0 }))
                .collect();
            let g = weighted_xtv(x, None, &r, true);
            for j in 0..p {
                grad[c * p + j] = g[j] + if j > 0 { lambda * coef[c][j] } else { 0.0 };
            }
        }
        let mut h = vec![0.0; dim * dim];
        for c in 0..kc {
            for d in 0..=c {
                let hw: Vec<f64> = (0..n)
                    .map(|i| {
                        let pc = probs[c][i];
                        let pd = probs[d][i];
                        wt(i) * (if c == d { pc * (1.0 - pc) } else { -pc * pd })
                    })
                    .collect();
                let block = weighted_gram(x, Some(&hw), true, par);
                for a in 0..p {
                    for b in 0..p {
                        h[(c * p + a) * dim + d * p + b] = block[a * p + b];
                        h[(d * p + b) * dim + c * p + a] = block[a * p + b];
                    }
                }
            }
        }
        for c in 0..kc {
            for j in 0..p {
                let reg = if j > 0 { lambda } else { 0.0 };
                // The flat direction (adding a constant to every class)
                // gets a vanishing ridge so the system is solvable.
                h[(c * p + j) * dim + c * p + j] += reg + 1e-10;
            }
        }
        let (l, _) = cholesky_jittered(&h, dim)
            .ok_or_else(|| bad("multinomial: the Hessian is singular"))?;
        let step = chol_solve(&l, dim, &grad);
        let mut t = 1.0;
        let mut next;
        let mut next_obj;
        loop {
            next = coef.clone();
            for c in 0..kc {
                for j in 0..p {
                    next[c][j] -= t * step[c * p + j];
                }
            }
            next_obj = objective(&next);
            if next_obj <= cur + 1e-12 * cur.abs().max(1.0) || t < 1e-8 {
                break;
            }
            t *= 0.5;
        }
        let max_step = step.iter().map(|s| (t * s).abs()).fold(0.0, f64::max);
        let improvement = cur - next_obj;
        coef = next;
        cur = next_obj;
        if max_step < tol || improvement.abs() < tol * 1e-3 * cur.abs().max(1.0) {
            converged = true;
            break;
        }
    }
    // Centre: Σ_c β_cj = 0 for each j. Probabilities are unchanged.
    for j in 0..p {
        let m = coef.iter().map(|b| b[j]).sum::<f64>() / kc as f64;
        for b in coef.iter_mut() {
            b[j] -= m;
        }
    }
    let probs = softmax_predict(x, &coef);
    let log_lik = (0..n)
        .map(|i| wt(i) * probs[y[i]][i].max(1e-300).ln())
        .sum::<f64>();
    Ok(SoftmaxFit {
        coef,
        n_iter: iters,
        converged,
        log_lik,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mat(cols: Vec<Vec<f64>>) -> Matrix {
        let n = cols[0].len();
        Matrix::new(n, cols)
    }

    #[test]
    fn ols_recovers_exact_line() {
        let x1: Vec<f64> = (0..20).map(|i| i as f64).collect();
        let x2: Vec<f64> = (0..20).map(|i| ((i * 7) % 5) as f64).collect();
        let y: Vec<f64> = x1
            .iter()
            .zip(&x2)
            .enumerate()
            .map(|(i, (a, b))| 1.5 + 2.0 * a - 0.5 * b + if i % 2 == 0 { 0.01 } else { -0.01 })
            .collect();
        let fit = ols(&mat(vec![x1, x2]), &y, None, true, false).unwrap();
        assert!((fit.coef[1] - 2.0).abs() < 1e-3);
        assert!((fit.coef[2] + 0.5).abs() < 1e-2);
        assert!(fit.r2 > 0.9999);
    }

    #[test]
    fn lasso_zero_alpha_is_ols_and_large_alpha_is_zero() {
        let x1: Vec<f64> = (0..50).map(|i| (i as f64 * 0.37).sin()).collect();
        let x2: Vec<f64> = (0..50).map(|i| (i as f64 * 0.11).cos()).collect();
        let y: Vec<f64> = x1.iter().zip(&x2).map(|(a, b)| 3.0 * a - b + 0.2).collect();
        let m = mat(vec![x1, x2]);
        let en = elastic_net(&m, &y, 0.0, 1.0, true, 10_000, 1e-12, false).unwrap();
        assert!((en.coef[1] - 3.0).abs() < 1e-6 && (en.coef[2] + 1.0).abs() < 1e-6);
        let big = elastic_net(&m, &y, 100.0, 1.0, true, 1000, 1e-10, false).unwrap();
        assert_eq!(&big.coef[1..], &[0.0, 0.0]);
        let r = ridge(&m, &y, 0.0, true, false).unwrap();
        assert!((r.coef[0] - 0.2).abs() < 1e-8);
    }

    #[test]
    fn logistic_converges_and_softmax_agrees_on_two_classes() {
        let x1: Vec<f64> = (0..200)
            .map(|i| ((i * 37) % 101) as f64 / 50.0 - 1.0)
            .collect();
        let y: Vec<f64> = x1
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if v * 2.0 + ((i * 13) % 7) as f64 / 7.0 - 0.5 > 0.0 {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let m = mat(vec![x1]);
        let fit = logistic(&m, &y, None, 0.0, 100, 1e-10, false).unwrap();
        assert!(fit.converged);
        let yc: Vec<usize> = y.iter().map(|&v| v as usize).collect();
        let sm = softmax(&m, &yc, 2, None, 0.0, 100, 1e-10, false).unwrap();
        // Centred two-class softmax is half the logit on each side.
        assert!((sm.coef[1][1] - sm.coef[0][1] - fit.coef[1]).abs() < 1e-5);
    }
}

//! Time-series kernels: `stats.ts`.
//!
//! - `acf`: the sample autocorrelation by FFT (O(n log n)), with
//!   Bartlett's intervals — statsmodels' `acf(x, nlags, fft=True,
//!   alpha=…)`.
//! - `pacf`: partial autocorrelations by Yule–Walker (Levinson–Durbin
//!   on the autocovariances), statsmodels' `pacf(method="ywadjusted")`
//!   and `"ywmle"`.
//! - `decompose`: a centred moving average's trend, the phase means of
//!   what is left as the season, and the residual — statsmodels'
//!   `seasonal_decompose` (two-sided, no trend extrapolation).
//! - `HoltWinters`: additive or multiplicative season, an additive trend
//!   or none, Winters' recursions (the season updated from the new
//!   level, as Winters 1960); the rates fit by bounded Nelder–Mead on the
//!   one-step squared error. The first season sets the initial state:
//!   its mean the level at its middle, the slope the step to the second
//!   season's mean, each value less that line its seasonal term.
//! - `seasonal_naive`: the last season, repeated.
//!
//! Series here are dense: a null in a time series has no lag to stand
//! at, so the language layer refuses one before calling in.

use crate::OdsError;

type Result<T> = std::result::Result<T, OdsError>;

fn bad(msg: impl Into<String>) -> OdsError {
    OdsError::InvalidArgument(msg.into())
}

// ---------------------------------------------------------------------
// FFT
// ---------------------------------------------------------------------

type C = [f64; 2];

#[inline]
fn cmul(a: C, b: C) -> C {
    [a[0] * b[0] - a[1] * b[1], a[0] * b[1] + a[1] * b[0]]
}

/// Below this many points a transform stays on one thread.
const FFT_PAR: usize = 1 << 15;

/// One radix-2 stage's butterflies over a block of `2h` points with the
/// stage's twiddles `w` (length `h`).
#[inline]
fn butterflies(block: &mut [C], w: &[C]) {
    let (lo, hi) = block.split_at_mut(w.len());
    for ((a, b), w) in lo.iter_mut().zip(hi.iter_mut()).zip(w) {
        let t = cmul(*b, *w);
        *b = [a[0] - t[0], a[1] - t[1]];
        *a = [a[0] + t[0], a[1] + t[1]];
    }
}

/// In-place iterative radix-2 FFT of interleaved complex `x`, length a
/// power of two, with `tw` the table e^{-2πik/(2n)} for k in 0..n (every
/// other entry is this length's twiddle). `inverse` conjugates the
/// twiddles (no 1/n scaling). Each stage reads its twiddles contiguously;
/// past `FFT_PAR` points the blocks of a stage (or, in the last stages,
/// the halves of one block) run on the pool. The arithmetic is the same
/// either way.
fn fft(x: &mut [C], tw: &[C], inverse: bool, par: bool) {
    let n = x.len();
    debug_assert!(n.is_power_of_two() && tw.len() == n);
    if n < 2 {
        return;
    }
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if i < j {
            x.swap(i, j);
        }
    }
    let conj = |c: C| if inverse { [c[0], -c[1]] } else { c };
    let par = par && n >= FFT_PAR;
    #[cfg(not(feature = "parallel"))]
    let _ = par;
    let stage_w = |len: usize| -> Vec<C> {
        let step = 2 * n / len;
        (0..len / 2).map(|k| conj(tw[k * step])).collect()
    };
    // The stages up to CACHE points run block by block while the block is
    // in cache: one pass over memory for all of them, not one each.
    const CACHE: usize = 1 << 12;
    let inner = CACHE.min(n);
    let ws: Vec<Vec<C>> = (1..=inner.trailing_zeros())
        .map(|b| stage_w(1 << b))
        .collect();
    let small = |block: &mut [C]| {
        for (b, w) in ws.iter().enumerate() {
            for sub in block.chunks_mut(2 << b) {
                butterflies(sub, w);
            }
        }
    };
    #[cfg(feature = "parallel")]
    if par {
        use rayon::prelude::*;
        x.par_chunks_mut(inner).for_each(small);
    } else {
        x.chunks_mut(inner).for_each(small);
    }
    #[cfg(not(feature = "parallel"))]
    x.chunks_mut(inner).for_each(small);
    let mut len = inner * 2;
    while len <= n {
        // stage `len`: e^{∓2πik/len} = tw[k · 2n/len]
        let w = stage_w(len);
        #[cfg(feature = "parallel")]
        if par {
            use rayon::prelude::*;
            if n / len >= 64 {
                x.par_chunks_mut(len)
                    .for_each(|block| butterflies(block, &w));
            } else {
                // Few, long blocks: each block's halves in pieces.
                const PIECE: usize = 1 << 12;
                for block in x.chunks_mut(len) {
                    let (lo, hi) = block.split_at_mut(len / 2);
                    lo.par_chunks_mut(PIECE)
                        .zip(hi.par_chunks_mut(PIECE))
                        .zip(w.par_chunks(PIECE))
                        .for_each(|((a, b), w)| {
                            for ((a, b), w) in a.iter_mut().zip(b.iter_mut()).zip(w) {
                                let t = cmul(*b, *w);
                                *b = [a[0] - t[0], a[1] - t[1]];
                                *a = [a[0] + t[0], a[1] + t[1]];
                            }
                        });
                }
            }
            len <<= 1;
            continue;
        }
        for block in x.chunks_mut(len) {
            butterflies(block, &w);
        }
        len <<= 1;
    }
}

/// `f(k)` for k in 0..n, on the pool when `par`.
fn tabulate<T: Send>(n: usize, par: bool, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    #[cfg(feature = "parallel")]
    if par {
        use rayon::prelude::*;
        let min = if n >= 1 << 14 { 1 << 12 } else { 1 };
        return (0..n).into_par_iter().with_min_len(min).map(f).collect();
    }
    let _ = par;
    (0..n).map(f).collect()
}

/// The biased autocovariances (divided by n) of `x` about its mean at
/// lags 0..=nlags, by FFT. A real sequence of length m is transformed as
/// a complex one of length m/2 (its even and odd samples), so the work is
/// two half-length transforms.
pub fn autocov(x: &[f64], nlags: usize) -> Vec<f64> {
    autocov_par(x, nlags, false)
}

/// `autocov`, the transforms on the pool when `par` and the series is long.
pub fn autocov_par(x: &[f64], nlags: usize, par: bool) -> Vec<f64> {
    let n = x.len();
    let mean = x.iter().sum::<f64>() / n as f64;
    // A few lags of a long series are cheaper as direct sums (a
    // vectorized dot product a lag, the lags on the pool) than as the
    // transforms of the whole of it. The choice depends on the shape
    // alone, never on the thread count, so every pool gives the same bits.
    let lags = nlags.min(n.saturating_sub(1));
    let log2n = (usize::BITS - n.leading_zeros()) as usize;
    if lags < 8 * log2n {
        return autocov_direct(x, mean, lags, par);
    }
    let m = (2 * n).next_power_of_two().max(4);
    let h = m / 2;
    let par = par && h >= FFT_PAR;
    // One table of e^{-2πik/m}, k in 0..h: the half-length transforms'
    // twiddles (every other entry) and the packing's.
    // Built as products of two short tables (k = a·1024 + b): a complex
    // multiply an entry rather than a sine and a cosine.
    let unit = |k: usize| {
        let (s, c) = (-std::f64::consts::TAU * k as f64 / m as f64).sin_cos();
        [c, s]
    };
    const SPLIT: usize = 1024;
    let fine: Vec<C> = (0..SPLIT.min(h)).map(unit).collect();
    let coarse: Vec<C> = (0..h.div_ceil(SPLIT)).map(|a| unit(a * SPLIT)).collect();
    let tw: Vec<C> = tabulate(h, par, |k| cmul(coarse[k / SPLIT], fine[k % SPLIT]));
    // w^k for k in h..=m (the unpacking reaches k = h): e^{-iπ} w^{k-h}.
    let w_at = |k: usize| -> C {
        if k < h {
            tw[k]
        } else {
            let t = tw[k - h];
            [-t[0], -t[1]]
        }
    };
    // z[k] = x[2k] + i x[2k+1] (zero padded)
    let mut z: Vec<C> = vec![[0.0, 0.0]; h];
    for (k, pair) in x.chunks(2).enumerate() {
        z[k] = [pair[0] - mean, pair.get(1).map_or(0.0, |v| v - mean)];
    }
    fft(&mut z, &tw, false, par);
    // Unpack X[k] (k in 0..=h) of the real sequence and take |X|², real
    // and even: its inverse transform is the circular autocovariance.
    let p: Vec<f64> = tabulate(h + 1, par, |k| {
        let (a, b) = (z[k % h], z[(h - k) % h]);
        // E = (Z[k] + conj Z[h-k]) / 2, O = (Z[k] - conj Z[h-k]) / (2i)
        let e = [0.5 * (a[0] + b[0]), 0.5 * (a[1] - b[1])];
        let o = [0.5 * (a[1] + b[1]), -0.5 * (a[0] - b[0])];
        let wo = cmul(o, w_at(k));
        let xr = e[0] + wo[0];
        let xi = e[1] + wo[1];
        xr * xr + xi * xi
    });
    // The inverse of the real even spectrum P (P[m-k] = P[k]) by the same
    // packing: y[j] = r[2j] + i r[2j+1] is the length-h inverse of
    // Y[k] = E[k] + i O[k] w^{-k}, E = (P[k] + P[k+h]) / 2,
    // O = (P[k] - P[k+h]) / 2.
    let pk = |k: usize| if k <= h { p[k] } else { p[m - k] };
    let mut y: Vec<C> = tabulate(h, par, |k| {
        let e = 0.5 * (pk(k) + pk(k + h));
        let o = 0.5 * (pk(k) - pk(k + h));
        // w^{-k} = conj(tw[k]) = (c, -s); i·o·(c - i s)... with s the
        // forward table's (negative-angle) sine: e^{+iθ} = (c, -s_fwd)
        let (c, s) = (tw[k][0], -tw[k][1]);
        [e - o * s, o * c]
    });
    fft(&mut y, &tw, true, par);
    // The unscaled length-h inverse of the half-weighted packing is
    // (m/2)·r_circ; the biased autocovariance divides by n.
    let scale = 1.0 / (h as f64) / n as f64;
    let lags = nlags.min(n - 1);
    (0..=lags).map(|k| y[k / 2][k % 2] * scale).collect()
}

/// Σ (x_i − mean)(x_{i+k} − mean) / n for k in 0..=lags, directly.
fn autocov_direct(x: &[f64], mean: f64, lags: usize, par: bool) -> Vec<f64> {
    let n = x.len();
    let d: Vec<f64> = x.iter().map(|v| v - mean).collect();
    let lag = |k: usize| -> f64 {
        let (a, b) = (&d[..n - k], &d[k..]);
        let mut acc = [0.0f64; 4];
        let mut ca = a.chunks_exact(4);
        let mut cb = b.chunks_exact(4);
        for (p, q) in (&mut ca).zip(&mut cb) {
            for j in 0..4 {
                acc[j] += p[j] * q[j];
            }
        }
        let tail: f64 = ca
            .remainder()
            .iter()
            .zip(cb.remainder())
            .map(|(p, q)| p * q)
            .sum();
        (acc[0] + acc[1] + acc[2] + acc[3] + tail) / n as f64
    };
    tabulate(lags + 1, par && n * (lags + 1) >= 1 << 18, lag)
}

/// z for a two-sided interval of `level` (0.95 → 1.959964).
fn z_of(level: f64) -> Result<f64> {
    crate::dist::norm_ppf(1.0 - (1.0 - level) / 2.0, 0.0, 1.0)
}

/// Lags 0..=nlags: the autocorrelation and, at `level`, Bartlett's
/// interval about it (lo, hi).
pub struct Correlogram {
    pub acf: Vec<f64>,
    pub lo: Vec<f64>,
    pub hi: Vec<f64>,
}

fn check_series(x: &[f64], nlags: usize, what: &str) -> Result<()> {
    if x.len() < 2 {
        return Err(bad(format!("{}: needs at least 2 values", what)));
    }
    if nlags >= x.len() {
        return Err(bad(format!(
            "{}: {} lags of a series of {} (at most {})",
            what,
            nlags,
            x.len(),
            x.len() - 1
        )));
    }
    if x.iter().any(|v| !v.is_finite()) {
        return Err(bad(format!("{}: the series holds a NaN or infinity", what)));
    }
    Ok(())
}

pub fn acf(x: &[f64], nlags: usize, level: f64) -> Result<Correlogram> {
    acf_par(x, nlags, level, false)
}

/// `acf`, its transforms on the pool when `par`.
pub fn acf_par(x: &[f64], nlags: usize, level: f64, par: bool) -> Result<Correlogram> {
    check_series(x, nlags, "acf")?;
    let cov = autocov_par(x, nlags, par);
    if cov[0] == 0.0 {
        return Err(bad("acf: the series is constant"));
    }
    let acf: Vec<f64> = cov.iter().map(|c| c / cov[0]).collect();
    let z = z_of(level)?;
    let n = x.len() as f64;
    let mut lo = Vec::with_capacity(acf.len());
    let mut hi = Vec::with_capacity(acf.len());
    let mut cum = 0.0;
    for (k, &r) in acf.iter().enumerate() {
        let var = match k {
            0 => 0.0,
            1 => 1.0 / n,
            _ => {
                cum += acf[k - 1] * acf[k - 1];
                (1.0 + 2.0 * cum) / n
            }
        };
        let half = z * var.sqrt();
        lo.push(r - half);
        hi.push(r + half);
    }
    Ok(Correlogram { acf, lo, hi })
}

/// Partial autocorrelations at lags 0..=nlags by Yule–Walker:
/// `adjusted` divides lag k's cross products by n − k (statsmodels'
/// "ywadjusted"), else by n ("ywmle").
pub fn pacf(x: &[f64], nlags: usize, adjusted: bool, level: f64) -> Result<Correlogram> {
    pacf_par(x, nlags, adjusted, level, false)
}

/// `pacf`, its transforms on the pool when `par`.
pub fn pacf_par(
    x: &[f64],
    nlags: usize,
    adjusted: bool,
    level: f64,
    par: bool,
) -> Result<Correlogram> {
    check_series(x, nlags, "pacf")?;
    let n = x.len();
    let mut r = autocov_par(x, nlags, par);
    if r[0] == 0.0 {
        return Err(bad("pacf: the series is constant"));
    }
    if adjusted {
        for (k, v) in r.iter_mut().enumerate() {
            *v *= n as f64 / (n - k) as f64;
        }
    }
    let r0 = r[0];
    let rho: Vec<f64> = r.iter().map(|v| v / r0).collect();
    // Levinson–Durbin
    let mut out = vec![1.0];
    let mut phi: Vec<f64> = Vec::new();
    let mut err = 1.0;
    for k in 1..=nlags {
        let acc: f64 = (0..phi.len()).map(|j| phi[j] * rho[k - 1 - j]).sum();
        let a = (rho[k] - acc) / err;
        let prev = phi.clone();
        for j in 0..prev.len() {
            phi[j] = prev[j] - a * prev[prev.len() - 1 - j];
        }
        phi.push(a);
        err *= 1.0 - a * a;
        out.push(a);
    }
    let half = z_of(level)? / (n as f64).sqrt();
    let lo = out
        .iter()
        .enumerate()
        .map(|(k, v)| if k == 0 { 1.0 } else { v - half })
        .collect();
    let hi = out
        .iter()
        .enumerate()
        .map(|(k, v)| if k == 0 { 1.0 } else { v + half })
        .collect();
    Ok(Correlogram { acf: out, lo, hi })
}

// ---------------------------------------------------------------------
// Decomposition
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    Additive,
    Multiplicative,
}

impl Model {
    pub fn parse(s: &str, what: &str) -> Result<Model> {
        match s {
            "additive" | "add" => Ok(Model::Additive),
            "multiplicative" | "mul" => Ok(Model::Multiplicative),
            other => Err(bad(format!(
                "{}: the model is \"additive\" or \"multiplicative\", got \"{}\"",
                what, other
            ))),
        }
    }
}

pub struct Decomposition {
    /// NaN where the moving average does not reach (the first and last
    /// half period).
    pub trend: Vec<f64>,
    pub seasonal: Vec<f64>,
    pub resid: Vec<f64>,
}

pub fn decompose(x: &[f64], period: usize, model: Model) -> Result<Decomposition> {
    let n = x.len();
    if period < 2 {
        return Err(bad("decompose: the period must be at least 2"));
    }
    if n < 2 * period {
        return Err(bad(format!(
            "decompose: needs two full periods ({} values), got {}",
            2 * period,
            n
        )));
    }
    if x.iter().any(|v| !v.is_finite()) {
        return Err(bad("decompose: the series holds a NaN or infinity"));
    }
    if model == Model::Multiplicative && x.iter().any(|v| *v <= 0.0) {
        return Err(bad(
            "decompose: a multiplicative model needs positive values",
        ));
    }
    // The centred moving average as running sums: an even period weights
    // its two ends by a half.
    let half = period / 2;
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0);
    for v in x {
        prefix.push(prefix.last().unwrap() + v);
    }
    let mut trend = vec![f64::NAN; n];
    let p = period as f64;
    for (t, tr) in trend.iter_mut().enumerate().take(n - half).skip(half) {
        *tr = if period % 2 == 1 {
            (prefix[t + half + 1] - prefix[t - half]) / p
        } else {
            let inner = prefix[t + half] - prefix[t - half + 1];
            (inner + 0.5 * (x[t - half] + x[t + half])) / p
        };
    }
    let detrended: Vec<f64> = x
        .iter()
        .zip(&trend)
        .map(|(v, t)| match model {
            Model::Additive => v - t,
            Model::Multiplicative => v / t,
        })
        .collect();
    let mut avg = vec![0.0; period];
    for (i, a) in avg.iter_mut().enumerate() {
        let (mut s, mut c) = (0.0, 0usize);
        let mut j = i;
        while j < n {
            if !detrended[j].is_nan() {
                s += detrended[j];
                c += 1;
            }
            j += period;
        }
        *a = s / c as f64;
    }
    let center = avg.iter().sum::<f64>() / p;
    for a in avg.iter_mut() {
        match model {
            Model::Additive => *a -= center,
            Model::Multiplicative => *a /= center,
        }
    }
    let seasonal: Vec<f64> = (0..n).map(|t| avg[t % period]).collect();
    let resid = (0..n)
        .map(|t| match model {
            Model::Additive => detrended[t] - seasonal[t],
            Model::Multiplicative => detrended[t] / seasonal[t],
        })
        .collect();
    Ok(Decomposition {
        trend,
        seasonal,
        resid,
    })
}

// ---------------------------------------------------------------------
// Holt-Winters
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Season {
    None,
    Add,
    Mul,
}

#[derive(Clone, Debug)]
pub struct HoltWinters {
    pub trend: bool,
    pub season: Season,
    pub m: usize,
    pub alpha: f64,
    pub beta: f64,
    pub gamma: f64,
    /// The state after the last observation.
    pub level: f64,
    pub slope: f64,
    /// The seasonal factors, `season_state[i]` for times ≡ i (mod m).
    pub season_state: Vec<f64>,
    /// One-step-ahead predictions; NaN for the values that set the
    /// initial state.
    pub fitted: Vec<f64>,
    pub sse: f64,
    pub n: usize,
    /// The first time the recursion predicts (the values before it set
    /// the initial state).
    pub start: usize,
}

/// The initial state and the first predicted time.
fn init_state(y: &[f64], trend: bool, season: Season, m: usize) -> (f64, f64, Vec<f64>, usize) {
    match season {
        Season::None => {
            let b = if trend { y[1] - y[0] } else { 0.0 };
            (y[0], b, Vec::new(), 1)
        }
        Season::Add | Season::Mul => {
            // The first season's mean is the level at its middle, and the
            // slope the step between the first two seasons' means; each of
            // the first values less the trend line there is its season.
            // The state is the level at the season's end.
            let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len() as f64;
            let l = mean(&y[..m]);
            let b = if trend {
                (mean(&y[m..2 * m]) - l) / m as f64
            } else {
                0.0
            };
            let mid = (m as f64 - 1.0) / 2.0;
            let s = y[..m]
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let line = l + b * (i as f64 - mid);
                    if season == Season::Add {
                        v - line
                    } else {
                        v / line
                    }
                })
                .collect();
            (l + b * mid, b, s, m)
        }
    }
}

struct Run {
    level: f64,
    slope: f64,
    season: Vec<f64>,
    sse: f64,
}

/// Winters' recursions from the initial state; `fitted` takes the
/// one-step predictions when given.
fn run(
    y: &[f64],
    trend: bool,
    season: Season,
    m: usize,
    (alpha, beta, gamma): (f64, f64, f64),
    mut fitted: Option<&mut [f64]>,
) -> Run {
    let (mut l, mut b, mut s, start) = init_state(y, trend, season, m);
    let mut sse = 0.0;
    for (t, &v) in y.iter().enumerate().skip(start) {
        let si = if season == Season::None {
            0.0
        } else {
            s[t % m]
        };
        let base = l + b;
        let pred = match season {
            Season::None => base,
            Season::Add => base + si,
            Season::Mul => base * si,
        };
        if let Some(f) = fitted.as_deref_mut() {
            f[t] = pred;
        }
        let e = v - pred;
        sse += e * e;
        let last = l;
        l = match season {
            Season::None => alpha * v + (1.0 - alpha) * base,
            Season::Add => alpha * (v - si) + (1.0 - alpha) * base,
            Season::Mul => alpha * (v / si) + (1.0 - alpha) * base,
        };
        if trend {
            b = beta * (l - last) + (1.0 - beta) * b;
        }
        match season {
            Season::None => {}
            Season::Add => s[t % m] = gamma * (v - l) + (1.0 - gamma) * si,
            Season::Mul => s[t % m] = gamma * (v / l) + (1.0 - gamma) * si,
        }
    }
    Run {
        level: l,
        slope: b,
        season: s,
        sse,
    }
}

/// Minimize `f` over the unit cube by Nelder–Mead, each point clamped
/// into the cube, from the best of a coarse grid.
pub fn nelder_mead_unit(dim: usize, f: &dyn Fn(&[f64]) -> f64) -> Vec<f64> {
    let clamp = |x: &[f64]| x.iter().map(|v| v.clamp(0.0, 1.0)).collect::<Vec<f64>>();
    let eval = |x: &[f64]| {
        let c = clamp(x);
        let v = f(&c);
        if v.is_finite() { v } else { f64::MAX }
    };
    if dim == 0 {
        return Vec::new();
    }
    // A coarse grid start.
    let grid = [0.05, 0.2, 0.5];
    let mut best = (f64::INFINITY, vec![0.2; dim]);
    let total = grid.len().pow(dim as u32);
    for code in 0..total {
        let mut c = code;
        let x: Vec<f64> = (0..dim)
            .map(|_| {
                let v = grid[c % grid.len()];
                c /= grid.len();
                v
            })
            .collect();
        let v = eval(&x);
        if v < best.0 {
            best = (v, x);
        }
    }
    let x0 = best.1;
    let mut pts: Vec<Vec<f64>> = vec![x0.clone()];
    for i in 0..dim {
        let mut p = x0.clone();
        p[i] = if p[i] + 0.1 <= 1.0 {
            p[i] + 0.1
        } else {
            p[i] - 0.1
        };
        pts.push(p);
    }
    let mut vals: Vec<f64> = pts.iter().map(|p| eval(p)).collect();
    for _ in 0..2000 {
        let mut order: Vec<usize> = (0..=dim).collect();
        order.sort_by(|&a, &b| vals[a].total_cmp(&vals[b]));
        pts = order.iter().map(|&i| pts[i].clone()).collect();
        vals = order.iter().map(|&i| vals[i]).collect();
        let spread = (vals[dim] - vals[0]).abs();
        let size = pts
            .iter()
            .skip(1)
            .map(|p| {
                p.iter()
                    .zip(&pts[0])
                    .map(|(a, b)| (a - b).abs())
                    .fold(0.0, f64::max)
            })
            .fold(0.0, f64::max);
        if spread <= 1e-12 * vals[0].abs().max(1e-300) && size < 1e-7 {
            break;
        }
        if size < 1e-10 {
            break;
        }
        let centroid: Vec<f64> = (0..dim)
            .map(|j| pts[..dim].iter().map(|p| p[j]).sum::<f64>() / dim as f64)
            .collect();
        let along = |t: f64| -> Vec<f64> {
            centroid
                .iter()
                .zip(&pts[dim])
                .map(|(c, w)| c + t * (w - c))
                .collect()
        };
        let xr = along(-1.0);
        let fr = eval(&xr);
        if fr < vals[0] {
            let xe = along(-2.0);
            let fe = eval(&xe);
            if fe < fr {
                pts[dim] = xe;
                vals[dim] = fe;
            } else {
                pts[dim] = xr;
                vals[dim] = fr;
            }
        } else if fr < vals[dim - 1] {
            pts[dim] = xr;
            vals[dim] = fr;
        } else {
            let (xc, fc) = if fr < vals[dim] {
                let x = along(-0.5);
                let v = eval(&x);
                (x, v)
            } else {
                let x = along(0.5);
                let v = eval(&x);
                (x, v)
            };
            if fc < vals[dim].min(fr) {
                pts[dim] = xc;
                vals[dim] = fc;
            } else {
                // shrink toward the best
                for i in 1..=dim {
                    let p: Vec<f64> = pts[i]
                        .iter()
                        .zip(&pts[0])
                        .map(|(a, b)| b + 0.5 * (a - b))
                        .collect();
                    vals[i] = eval(&p);
                    pts[i] = p;
                }
            }
        }
    }
    let i = (0..=dim)
        .min_by(|&a, &b| vals[a].total_cmp(&vals[b]))
        .unwrap_or(0);
    clamp(&pts[i])
}

impl HoltWinters {
    /// Fit to `y`: the rates given are kept, the others (`None`) chosen by
    /// minimizing the one-step squared error.
    pub fn fit(
        y: &[f64],
        m: usize,
        trend: bool,
        season: Season,
        alpha: Option<f64>,
        beta: Option<f64>,
        gamma: Option<f64>,
    ) -> Result<HoltWinters> {
        let need = match (season, trend) {
            (Season::None, false) => 2,
            (Season::None, true) => 3,
            (_, false) => m + 1,
            (_, true) => 2 * m,
        };
        if season != Season::None && m < 2 {
            return Err(bad("holt_winters: the season must be at least 2"));
        }
        if y.len() < need {
            return Err(bad(format!(
                "holt_winters: needs at least {} values for this model, got {}",
                need,
                y.len()
            )));
        }
        if y.iter().any(|v| !v.is_finite()) {
            return Err(bad("holt_winters: the series holds a NaN or infinity"));
        }
        if season == Season::Mul && y.iter().any(|v| *v <= 0.0) {
            return Err(bad(
                "holt_winters: a multiplicative season needs positive values",
            ));
        }
        for (name, v) in [("alpha", alpha), ("beta", beta), ("gamma", gamma)] {
            if let Some(v) = v
                && !(0.0..=1.0).contains(&v)
            {
                return Err(bad(format!(
                    "holt_winters: {} must lie in [0, 1], got {}",
                    name, v
                )));
            }
        }
        // Free rates, in order alpha, beta (with a trend), gamma (with a season).
        let mut free: Vec<usize> = Vec::new();
        if alpha.is_none() {
            free.push(0);
        }
        if trend && beta.is_none() {
            free.push(1);
        }
        if season != Season::None && gamma.is_none() {
            free.push(2);
        }
        let fixed = [
            alpha.unwrap_or(0.0),
            beta.unwrap_or(0.0),
            gamma.unwrap_or(0.0),
        ];
        let assemble = |x: &[f64]| {
            let mut p = fixed;
            for (k, &i) in free.iter().enumerate() {
                p[i] = x[k];
            }
            (p[0], p[1], p[2])
        };
        let best = nelder_mead_unit(free.len(), &|x| {
            run(y, trend, season, m, assemble(x), None).sse
        });
        let rates = assemble(&best);
        let mut fitted = vec![f64::NAN; y.len()];
        let r = run(y, trend, season, m, rates, Some(&mut fitted));
        let (_, _, _, start) = init_state(y, trend, season, m);
        Ok(HoltWinters {
            trend,
            season,
            m,
            alpha: rates.0,
            beta: if trend { rates.1 } else { f64::NAN },
            gamma: if season == Season::None {
                f64::NAN
            } else {
                rates.2
            },
            level: r.level,
            slope: r.slope,
            season_state: r.season,
            fitted,
            sse: r.sse,
            n: y.len(),
            start,
        })
    }

    /// The forecasts 1..=h steps past the last observation.
    pub fn forecast(&self, h: usize) -> Vec<f64> {
        (1..=h)
            .map(|k| {
                let base = self.level + k as f64 * self.slope;
                match self.season {
                    Season::None => base,
                    Season::Add => base + self.season_state[(self.n + k - 1) % self.m],
                    Season::Mul => base * self.season_state[(self.n + k - 1) % self.m],
                }
            })
            .collect()
    }
}

/// The last `season` values repeated over `h` steps.
pub fn seasonal_naive(y: &[f64], season: usize, h: usize) -> Result<Vec<f64>> {
    if season == 0 {
        return Err(bad("seasonal_naive: the season must be at least 1"));
    }
    if y.len() < season {
        return Err(bad(format!(
            "seasonal_naive: needs a full season ({} values), got {}",
            season,
            y.len()
        )));
    }
    let base = y.len() - season;
    Ok((0..h).map(|k| y[base + k % season]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct_autocov(x: &[f64], k: usize) -> f64 {
        let n = x.len();
        let m = x.iter().sum::<f64>() / n as f64;
        (0..n - k).map(|i| (x[i] - m) * (x[i + k] - m)).sum::<f64>() / n as f64
    }

    #[test]
    fn fft_autocov_is_the_direct_sum() {
        for n in [2usize, 3, 7, 16, 100, 1001] {
            let x: Vec<f64> = (0..n)
                .map(|i| ((i * 37) % 11) as f64 + (i as f64 * 0.3).sin())
                .collect();
            let lags = (n - 1).min(40);
            let got = autocov(&x, lags);
            assert_eq!(got, autocov_par(&x, lags, true));
            let fft_path = autocov_par(&x, n - 1, false);
            assert_eq!(fft_path, autocov_par(&x, n - 1, true));
            for k in 0..=lags {
                let want = direct_autocov(&x, k);
                assert!(
                    (got[k] - want).abs() < 1e-9 * want.abs().max(1.0),
                    "n {n} lag {k}: {} {}",
                    got[k],
                    want
                );
                assert!(
                    (fft_path[k] - want).abs() < 1e-9 * want.abs().max(1.0),
                    "fft n {n} lag {k}"
                );
            }
        }
    }

    #[test]
    fn pacf_of_an_ar1_cuts_off() {
        let mut x = vec![0.0; 5000];
        let mut s: u64 = 1;
        for i in 1..x.len() {
            s = crate::random::splitmix64(s);
            let e = (s >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
            x[i] = 0.7 * x[i - 1] + e;
        }
        let p = pacf(&x, 5, true, 0.95).unwrap();
        assert!((p.acf[1] - 0.7).abs() < 0.03);
        assert!(p.acf[2..].iter().all(|v| v.abs() < 0.05));
    }

    #[test]
    fn holt_winters_recovers_a_seasonal_line() {
        let m = 7;
        let y: Vec<f64> = (0..140)
            .map(|t| 100.0 + 0.5 * t as f64 + [5.0, -3.0, 2.0, 0.0, -6.0, 1.0, 1.0][t % m])
            .collect();
        let hw = HoltWinters::fit(&y, m, true, Season::Add, None, None, None).unwrap();
        let f = hw.forecast(7);
        for (k, v) in f.iter().enumerate() {
            let t = 140 + k;
            let want = 100.0 + 0.5 * t as f64 + [5.0, -3.0, 2.0, 0.0, -6.0, 1.0, 1.0][t % m];
            assert!((v - want).abs() < 1e-6, "{k}: {v} {want}");
        }
    }
}

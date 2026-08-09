//! Statistical inference kernels: correlation, t-tests, chi-squared
//! goodness of fit, and ordinary least squares.
//!
//! Everything composes the crate's own reductions plus the distribution
//! functions in [`crate::dist`]. Null handling follows the reductions:
//! pairwise statistics use *pairwise-complete* observations (positions
//! where both operands are valid — R's `use = "pairwise.complete.obs"`),
//! and `ols` drops any row with a null in `y` or any regressor (R's
//! `na.omit`), reporting how many rows survived.
//!
//! OLS solves the normal equations: the Gram matrix `X'X` is a
//! rayon-parallel pass over the data, and the k×k system goes through a
//! small in-crate Cholesky (k is the *column* count — tens, not
//! thousands — so this is 20 lines of textbook, not a worse LAPACK; the
//! heavyweight decompositions arrive with the Phase 3 matrix type).
//! Standard errors come from the diagonal of `(X'X)⁻¹`, p-values from
//! the t distribution.

use crate::{dist, OdsError, Scalar, Series};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

type Result<T> = std::result::Result<T, OdsError>;

// ---------------------------------------------------------------------
// Pairwise statistics
// ---------------------------------------------------------------------

/// Extract the pairwise-complete values of two equal-length series as
/// dense f64 vectors.
fn pairwise_complete(a: &Series, b: &Series) -> Result<(Vec<f64>, Vec<f64>)> {
    if a.len() != b.len() {
        return Err(OdsError::LengthMismatch {
            left: a.len(),
            right: b.len(),
        });
    }
    let mut xs = Vec::with_capacity(a.len());
    let mut ys = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        match (a.scalar_at(i), b.scalar_at(i)) {
            (Scalar::Null, _) | (_, Scalar::Null) => {}
            (x, y) => {
                xs.push(scalar_f64(x)?);
                ys.push(scalar_f64(y)?);
            }
        }
    }
    Ok((xs, ys))
}

fn scalar_f64(s: Scalar) -> Result<f64> {
    match s {
        Scalar::F64(x) => Ok(x),
        Scalar::I64(x) => Ok(x as f64),
        _ => Err(OdsError::TypeMismatch(
            "statistic requires numeric series".to_string(),
        )),
    }
}

/// A null-free series as an owned f64 buffer (i64 casts; one memcpy for
/// f64 — no per-element enum dispatch).
fn dense_f64(s: &Series) -> Result<Vec<f64>> {
    match s {
        Series::F64 { values, .. } => Ok(values.as_ref().clone()),
        Series::I64 { values, .. } => Ok(values.iter().map(|&x| x as f64).collect()),
        Series::Bool { .. } => Err(OdsError::TypeMismatch(
            "statistic requires numeric series".to_string(),
        )),
    }
}

fn mean_of(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Sample covariance (ddof = 1) over pairwise-complete observations.
pub fn cov(a: &Series, b: &Series) -> Result<Option<f64>> {
    let (xs, ys) = pairwise_complete(a, b)?;
    if xs.len() < 2 {
        return Ok(None);
    }
    let (mx, my) = (mean_of(&xs), mean_of(&ys));
    let s: f64 = xs
        .iter()
        .zip(ys.iter())
        .map(|(x, y)| (x - mx) * (y - my))
        .sum();
    Ok(Some(s / (xs.len() - 1) as f64))
}

/// Pearson correlation over pairwise-complete observations.
pub fn corr(a: &Series, b: &Series) -> Result<Option<f64>> {
    let (xs, ys) = pairwise_complete(a, b)?;
    if xs.len() < 2 {
        return Ok(None);
    }
    let (mx, my) = (mean_of(&xs), mean_of(&ys));
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in xs.iter().zip(ys.iter()) {
        let (dx, dy) = (x - mx, y - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx == 0.0 || syy == 0.0 {
        return Ok(None);
    }
    Ok(Some(sxy / (sxx * syy).sqrt()))
}

// ---------------------------------------------------------------------
// t-tests
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct TTest {
    pub t: f64,
    pub df: f64,
    pub p_value: f64,
    pub mean: f64,
    /// Second sample's mean for the two-sample test; NaN for one-sample.
    pub mean_other: f64,
}

fn valid_f64s(s: &Series) -> Result<Vec<f64>> {
    (0..s.len())
        .filter_map(|i| match s.scalar_at(i) {
            Scalar::Null => None,
            other => Some(scalar_f64(other)),
        })
        .collect()
}

fn sample_var(xs: &[f64], mean: f64) -> f64 {
    xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (xs.len() - 1) as f64
}

fn two_sided_t_p(t: f64, df: f64) -> Result<f64> {
    // Exact fits produce enormous (or infinite) t statistics whose p has
    // underflowed to zero long before statrs' incomplete beta gives out.
    if t.is_nan() {
        return Ok(f64::NAN);
    }
    if !t.is_finite() || t.abs() > 1e8 {
        return Ok(0.0);
    }
    Ok(2.0 * dist::t_cdf(-t.abs(), df)?)
}

/// One-sample t-test of H0: mean == mu0. Two-sided p.
pub fn t_test_one_sample(s: &Series, mu0: f64) -> Result<TTest> {
    let xs = valid_f64s(s)?;
    if xs.len() < 2 {
        return Err(OdsError::InvalidArgument(
            "t_test needs at least 2 valid observations".to_string(),
        ));
    }
    let n = xs.len() as f64;
    let mean = mean_of(&xs);
    let var = sample_var(&xs, mean);
    if var == 0.0 {
        return Err(OdsError::InvalidArgument(
            "t_test: sample has zero variance".to_string(),
        ));
    }
    let t = (mean - mu0) / (var / n).sqrt();
    let df = n - 1.0;
    Ok(TTest {
        t,
        df,
        p_value: two_sided_t_p(t, df)?,
        mean,
        mean_other: f64::NAN,
    })
}

/// Welch's two-sample t-test (unequal variances — R's `t.test` default).
/// Two-sided p.
pub fn t_test_welch(a: &Series, b: &Series) -> Result<TTest> {
    let xs = valid_f64s(a)?;
    let ys = valid_f64s(b)?;
    if xs.len() < 2 || ys.len() < 2 {
        return Err(OdsError::InvalidArgument(
            "t_test needs at least 2 valid observations per sample".to_string(),
        ));
    }
    let (na, nb) = (xs.len() as f64, ys.len() as f64);
    let (ma, mb) = (mean_of(&xs), mean_of(&ys));
    let (va, vb) = (sample_var(&xs, ma), sample_var(&ys, mb));
    let sa = va / na;
    let sb = vb / nb;
    if sa + sb == 0.0 {
        return Err(OdsError::InvalidArgument(
            "t_test: both samples have zero variance".to_string(),
        ));
    }
    let t = (ma - mb) / (sa + sb).sqrt();
    let df = (sa + sb) * (sa + sb) / (sa * sa / (na - 1.0) + sb * sb / (nb - 1.0));
    Ok(TTest {
        t,
        df,
        p_value: two_sided_t_p(t, df)?,
        mean: ma,
        mean_other: mb,
    })
}

// ---------------------------------------------------------------------
// Chi-squared goodness of fit
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Chi2Test {
    pub statistic: f64,
    pub df: f64,
    pub p_value: f64,
}

/// Pearson's chi-squared goodness-of-fit test of observed counts against
/// expected counts. Nulls are not meaningful in count data and refuse.
pub fn chi2_gof(observed: &Series, expected: &Series) -> Result<Chi2Test> {
    if observed.len() != expected.len() {
        return Err(OdsError::LengthMismatch {
            left: observed.len(),
            right: expected.len(),
        });
    }
    if observed.null_count() > 0 || expected.null_count() > 0 {
        return Err(OdsError::InvalidArgument(
            "chi2_test: counts must not contain nulls".to_string(),
        ));
    }
    if observed.len() < 2 {
        return Err(OdsError::InvalidArgument(
            "chi2_test needs at least 2 categories".to_string(),
        ));
    }
    let mut stat = 0.0;
    for i in 0..observed.len() {
        let o = scalar_f64(observed.scalar_at(i))?;
        let e = scalar_f64(expected.scalar_at(i))?;
        if e <= 0.0 {
            return Err(OdsError::InvalidArgument(
                "chi2_test: expected counts must be positive".to_string(),
            ));
        }
        stat += (o - e) * (o - e) / e;
    }
    let df = (observed.len() - 1) as f64;
    Ok(Chi2Test {
        statistic: stat,
        df,
        p_value: 1.0 - dist::chi2_cdf(stat, df)?,
    })
}

// ---------------------------------------------------------------------
// Ordinary least squares
// ---------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct OlsFit {
    /// Coefficients; index 0 is the intercept.
    pub coef: Vec<f64>,
    pub se: Vec<f64>,
    pub t: Vec<f64>,
    pub p_value: Vec<f64>,
    pub r2: f64,
    pub adj_r2: f64,
    /// Rows used after dropping any with a null.
    pub n: usize,
    pub df_resid: f64,
}

/// OLS of `y` on the given regressor columns, with an intercept.
/// Rows containing a null in `y` or any regressor are dropped.
pub fn ols(y: &Series, x_cols: &[&Series], par: bool) -> Result<OlsFit> {
    if x_cols.is_empty() {
        return Err(OdsError::InvalidArgument(
            "lm needs at least one regressor".to_string(),
        ));
    }
    let n_rows = y.len();
    for x in x_cols {
        if x.len() != n_rows {
            return Err(OdsError::LengthMismatch {
                left: n_rows,
                right: x.len(),
            });
        }
    }

    // Complete-case handling. The dense case (no nulls anywhere) is the
    // hot one — B4 runs it — and it skips the row scan and per-element
    // gather entirely, borrowing f64 buffers as-is.
    let all_dense = y.null_count() == 0 && x_cols.iter().all(|x| x.null_count() == 0);
    let k = x_cols.len() + 1; // + intercept
    let (n, yv, cols): (usize, Vec<f64>, Vec<Vec<f64>>) = if all_dense {
        let n = n_rows;
        let mut cols: Vec<Vec<f64>> = Vec::with_capacity(k);
        cols.push(vec![1.0; n]);
        for x in x_cols {
            cols.push(dense_f64(x)?);
        }
        (n, dense_f64(y)?, cols)
    } else {
        let keep: Vec<usize> = (0..n_rows)
            .filter(|&i| {
                !matches!(y.scalar_at(i), Scalar::Null)
                    && x_cols
                        .iter()
                        .all(|x| !matches!(x.scalar_at(i), Scalar::Null))
            })
            .collect();
        let n = keep.len();
        let yv: Vec<f64> = keep
            .iter()
            .map(|&i| scalar_f64(y.scalar_at(i)))
            .collect::<Result<_>>()?;
        let mut cols: Vec<Vec<f64>> = Vec::with_capacity(k);
        cols.push(vec![1.0; n]);
        for x in x_cols {
            cols.push(
                keep.iter()
                    .map(|&i| scalar_f64(x.scalar_at(i)))
                    .collect::<Result<_>>()?,
            );
        }
        (n, yv, cols)
    };
    if n <= k {
        return Err(OdsError::InvalidArgument(format!(
            "lm needs more complete rows ({}) than coefficients ({})",
            n, k
        )));
    }

    // Gram matrix G = X'X and moment vector b = X'y. The (i, j) pairs are
    // independent dot products — the O(n·k²) part of the fit.
    let pairs: Vec<(usize, usize)> = (0..k).flat_map(|i| (0..=i).map(move |j| (i, j))).collect();
    let dot = |a: &[f64], b: &[f64]| -> f64 {
        let mut acc = [0.0f64; 4];
        let ca = a.chunks_exact(4);
        let cb = b.chunks_exact(4);
        let rem: f64 = ca
            .remainder()
            .iter()
            .zip(cb.remainder())
            .map(|(x, y)| x * y)
            .sum();
        for (x4, y4) in ca.zip(cb) {
            for l in 0..4 {
                acc[l] += x4[l] * y4[l];
            }
        }
        acc.iter().sum::<f64>() + rem
    };
    let gram_at = |&(i, j): &(usize, usize)| dot(&cols[i], &cols[j]);
    let gram_vals: Vec<f64> = {
        #[cfg(feature = "parallel")]
        if par {
            pairs.par_iter().map(gram_at).collect()
        } else {
            pairs.iter().map(gram_at).collect()
        }
        #[cfg(not(feature = "parallel"))]
        {
            let _ = par;
            pairs.iter().map(gram_at).collect()
        }
    };
    let mut g = vec![vec![0.0f64; k]; k];
    for ((i, j), v) in pairs.iter().zip(gram_vals) {
        g[*i][*j] = v;
        g[*j][*i] = v;
    }
    let xty: Vec<f64> = cols.iter().map(|c| dot(c, &yv)).collect();

    // Solve G β = X'y and invert G via Cholesky.
    let l = cholesky(&g).ok_or_else(|| {
        OdsError::InvalidArgument(
            "lm: design matrix is singular (collinear or constant regressors?)".to_string(),
        )
    })?;
    let coef = chol_solve(&l, &xty);
    let g_inv_diag = chol_inverse_diag(&l);

    // Residual pass (explicit, not the b'X'y shortcut — one O(n·k) sweep
    // buys numerical honesty and, later, residual outputs).
    let mut sse = 0.0;
    for r in 0..n {
        let mut yhat = 0.0;
        for (j, c) in cols.iter().enumerate() {
            yhat += coef[j] * c[r];
        }
        let e = yv[r] - yhat;
        sse += e * e;
    }
    let ymean = mean_of(&yv);
    let sst: f64 = yv.iter().map(|v| (v - ymean) * (v - ymean)).sum();
    let df_resid = (n - k) as f64;
    let sigma2 = sse / df_resid;

    let se: Vec<f64> = g_inv_diag.iter().map(|d| (sigma2 * d).sqrt()).collect();
    let t: Vec<f64> = coef.iter().zip(se.iter()).map(|(c, s)| c / s).collect();
    let p_value: Vec<f64> = t
        .iter()
        .map(|&tv| two_sided_t_p(tv, df_resid))
        .collect::<Result<_>>()?;
    let r2 = if sst == 0.0 {
        f64::NAN
    } else {
        1.0 - sse / sst
    };
    let adj_r2 = 1.0 - (1.0 - r2) * (n as f64 - 1.0) / df_resid;

    Ok(OlsFit {
        coef,
        se,
        t,
        p_value,
        r2,
        adj_r2,
        n,
        df_resid,
    })
}

/// Cholesky factor L (lower) of a symmetric positive-definite matrix;
/// None when the matrix is not positive definite.
fn cholesky(g: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let k = g.len();
    let mut l = vec![vec![0.0f64; k]; k];
    for i in 0..k {
        for j in 0..=i {
            let mut s = g[i][j];
            s -= l[i][..j]
                .iter()
                .zip(&l[j][..j])
                .map(|(a, b)| a * b)
                .sum::<f64>();
            if i == j {
                if s <= 0.0 {
                    return None;
                }
                l[i][j] = s.sqrt();
            } else {
                l[i][j] = s / l[j][j];
            }
        }
    }
    Some(l)
}

/// Solve (L Lᵀ) x = b.
fn chol_solve(l: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let k = l.len();
    let mut z = vec![0.0f64; k];
    for i in 0..k {
        let mut s = b[i];
        for m in 0..i {
            s -= l[i][m] * z[m];
        }
        z[i] = s / l[i][i];
    }
    let mut x = vec![0.0f64; k];
    for i in (0..k).rev() {
        let mut s = z[i];
        for m in i + 1..k {
            s -= l[m][i] * x[m];
        }
        x[i] = s / l[i][i];
    }
    x
}

/// Diagonal of (L Lᵀ)⁻¹: solve for each unit vector, keep entry i.
fn chol_inverse_diag(l: &[Vec<f64>]) -> Vec<f64> {
    let k = l.len();
    (0..k)
        .map(|i| {
            let mut e = vec![0.0f64; k];
            e[i] = 1.0;
            chol_solve(l, &e)[i]
        })
        .collect()
}

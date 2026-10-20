//! Robust statistics and the bootstrap.
//!
//! - `mad`: the median absolute deviation, scaled to a normal's standard
//!   deviation by default (SciPy's `median_abs_deviation(scale="normal")`).
//! - `iqr`, `tukey_fences`: from linear-interpolation quantiles (NumPy's
//!   default, `Series::quantile`'s).
//! - `zscore`: by the mean and the sample sd, or robustly by the median
//!   and the scaled MAD.
//! - `bootstrap`: percentile intervals of a native statistic over
//!   resamples drawn in parallel, resample `i` from its own PCG stream —
//!   the same draws on any number of threads.
//!
//! Nulls are dropped (a statistic of the valid values); `zscore` keeps a
//! null where its input has one.

use crate::random::Pcg64;
use crate::stats::valid_f64s;
use crate::{OdsError, Series};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

type Result<T> = std::result::Result<T, OdsError>;

/// 1 / Φ⁻¹(3/4): the MAD of a normal sample times this is its sd.
pub const MAD_NORMAL: f64 = 1.482_602_218_505_602;

fn bad(msg: impl Into<String>) -> OdsError {
    OdsError::InvalidArgument(msg.into())
}

/// The `q` quantile of `xs` by linear interpolation, reordering `xs`
/// (a selection, O(n)).
pub fn quantile_mut(xs: &mut [f64], q: f64) -> f64 {
    let n = xs.len();
    if n == 0 {
        return f64::NAN;
    }
    let h = (n - 1) as f64 * q;
    let lo = h.floor() as usize;
    let frac = h - lo as f64;
    let (_, &mut a, rest) = xs.select_nth_unstable_by(lo, f64::total_cmp);
    if frac == 0.0 || rest.is_empty() {
        return a;
    }
    let b = rest.iter().copied().fold(f64::INFINITY, f64::min);
    a + (b - a) * frac
}

pub fn median_mut(xs: &mut [f64]) -> f64 {
    quantile_mut(xs, 0.5)
}

fn values(s: &Series, what: &str) -> Result<Vec<f64>> {
    let v = valid_f64s(s)?;
    if v.is_empty() {
        return Err(bad(format!("{}: no valid values", what)));
    }
    Ok(v)
}

/// The median absolute deviation times `scale`.
pub fn mad(s: &Series, scale: f64) -> Result<f64> {
    let mut v = values(s, "mad")?;
    let m = median_mut(&mut v);
    for x in v.iter_mut() {
        *x = (*x - m).abs();
    }
    Ok(median_mut(&mut v) * scale)
}

/// The interquartile range, q75 − q25.
pub fn iqr(s: &Series) -> Result<f64> {
    let mut v = values(s, "iqr")?;
    let q1 = quantile_mut(&mut v, 0.25);
    let q3 = quantile_mut(&mut v, 0.75);
    Ok(q3 - q1)
}

/// Tukey's fences: (q25 − k·IQR, q75 + k·IQR).
pub fn tukey_fences(s: &Series, k: f64) -> Result<(f64, f64)> {
    let mut v = values(s, "tukey_fences")?;
    let q1 = quantile_mut(&mut v, 0.25);
    let q3 = quantile_mut(&mut v, 0.75);
    let iqr = q3 - q1;
    Ok((q1 - k * iqr, q3 + k * iqr))
}

/// Standard scores: (x − mean) / sd, or robustly (x − median) / MAD
/// (normal-scaled). Nulls stay null.
pub fn zscore(s: &Series, robust: bool, par: bool) -> Result<Series> {
    let (center, spread) = if robust {
        let mut v = values(s, "zscore")?;
        let m = median_mut(&mut v);
        for x in v.iter_mut() {
            *x = (*x - m).abs();
        }
        (m, median_mut(&mut v) * MAD_NORMAL)
    } else {
        let m = s.mean(par)?.ok_or_else(|| bad("zscore: no valid values"))?;
        let sd = s
            .std(par)?
            .ok_or_else(|| bad("zscore: needs at least 2 valid values"))?;
        (m, sd)
    };
    if spread == 0.0 {
        return Err(bad(if robust {
            "zscore: the MAD is zero (more than half the values are equal)"
        } else {
            "zscore: the standard deviation is zero"
        }));
    }
    let (vals, validity): (Vec<f64>, Option<crate::bitmap::Bitmap>) = match s {
        Series::F64 { values, validity } => (
            values.iter().map(|x| (x - center) / spread).collect(),
            validity.clone(),
        ),
        Series::I64 { values, validity } => (
            values
                .iter()
                .map(|&x| (x as f64 - center) / spread)
                .collect(),
            validity.clone(),
        ),
        other => {
            return Err(OdsError::TypeMismatch(format!(
                "zscore needs a numeric column, got {}",
                other.dtype()
            )));
        }
    };
    Ok(Series::F64 {
        values: std::sync::Arc::new(vals),
        validity,
    })
}

// ---------------------------------------------------------------------
// The bootstrap
// ---------------------------------------------------------------------

/// A statistic the bootstrap computes natively.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stat {
    Mean,
    Median,
    Std,
    Var,
    Sum,
    Quantile(f64),
}

impl Stat {
    pub fn parse(name: &str) -> Option<Stat> {
        Some(match name {
            "mean" => Stat::Mean,
            "median" => Stat::Median,
            "std" => Stat::Std,
            "var" => Stat::Var,
            "sum" => Stat::Sum,
            _ => return None,
        })
    }

    /// The statistic of `xs`, which it may reorder.
    pub fn of(self, xs: &mut [f64]) -> f64 {
        let n = xs.len() as f64;
        match self {
            Stat::Mean => xs.iter().sum::<f64>() / n,
            Stat::Sum => xs.iter().sum(),
            Stat::Var | Stat::Std => {
                let m = xs.iter().sum::<f64>() / n;
                let v = xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n - 1.0);
                if self == Stat::Std { v.sqrt() } else { v }
            }
            Stat::Median => median_mut(xs),
            Stat::Quantile(q) => quantile_mut(xs, q),
        }
    }
}

/// How two samples' statistics combine: `a − b` or `a / b`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    Diff,
    Ratio,
}

#[derive(Clone, Debug)]
pub struct Bootstrap {
    /// The statistic of the data themselves.
    pub estimate: f64,
    pub lo: f64,
    pub hi: f64,
    /// The resamples' standard deviation.
    pub se: f64,
    /// One statistic per resample, in resample order.
    pub draws: Vec<f64>,
}

fn resample_into(rng: &mut Pcg64, xs: &[f64], buf: &mut Vec<f64>) {
    buf.clear();
    let n = xs.len() as u64;
    buf.extend((0..xs.len()).map(|_| xs[rng.below(n) as usize]));
}

/// `n` resamples of `a` (and of `b`, resampled on its own, when given),
/// the statistic of each, and the percentile interval at `level`.
/// Resample `i` draws from stream `i + 1` of `seed`.
pub fn bootstrap(
    a: &Series,
    b: Option<&Series>,
    stat: Stat,
    compare: Compare,
    n: usize,
    seed: u64,
    level: f64,
    par: bool,
) -> Result<Bootstrap> {
    if !(level > 0.0 && level < 1.0) {
        return Err(bad(format!(
            "bootstrap: level must lie in (0, 1), got {}",
            level
        )));
    }
    if n < 2 {
        return Err(bad("bootstrap: n must be at least 2"));
    }
    if let Stat::Quantile(q) = stat
        && !(0.0..=1.0).contains(&q)
    {
        return Err(bad(format!(
            "bootstrap: a quantile must lie in [0, 1], got {}",
            q
        )));
    }
    let xs = values(a, "bootstrap")?;
    let ys = match b {
        Some(b) => Some(values(b, "bootstrap")?),
        None => None,
    };
    let min_n = if matches!(stat, Stat::Std | Stat::Var) {
        2
    } else {
        1
    };
    if xs.len() < min_n || ys.as_ref().is_some_and(|y| y.len() < min_n) {
        return Err(bad("bootstrap: too few values for the statistic"));
    }
    let combine = |sa: f64, sb: f64| match compare {
        Compare::Diff => sa - sb,
        Compare::Ratio => sa / sb,
    };
    let estimate = {
        let sa = stat.of(&mut xs.clone());
        match &ys {
            Some(y) => combine(sa, stat.of(&mut y.clone())),
            None => sa,
        }
    };
    let one = |i: usize, buf: &mut Vec<f64>| -> f64 {
        let mut rng = Pcg64::with_stream(seed, i as u64 + 1);
        resample_into(&mut rng, &xs, buf);
        let sa = stat.of(buf);
        match &ys {
            Some(y) => {
                resample_into(&mut rng, y, buf);
                combine(sa, stat.of(buf))
            }
            None => sa,
        }
    };
    #[cfg(feature = "parallel")]
    let draws: Vec<f64> = if par {
        (0..n)
            .into_par_iter()
            .map_init(Vec::new, |buf, i| one(i, buf))
            .collect()
    } else {
        let mut buf = Vec::new();
        (0..n).map(|i| one(i, &mut buf)).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let draws: Vec<f64> = {
        let _ = par;
        let mut buf = Vec::new();
        (0..n).map(|i| one(i, &mut buf)).collect()
    };
    let mut sorted = draws.clone();
    let tail = (1.0 - level) / 2.0;
    let lo = quantile_mut(&mut sorted, tail);
    let hi = quantile_mut(&mut sorted, 1.0 - tail);
    let m = draws.iter().sum::<f64>() / n as f64;
    let se = (draws.iter().map(|d| (d - m) * (d - m)).sum::<f64>() / (n as f64 - 1.0)).sqrt();
    Ok(Bootstrap {
        estimate,
        lo,
        hi,
        se,
        draws,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_match_the_series_quantile() {
        let v: Vec<f64> = (0..37).map(|i| ((i * 7919) % 101) as f64 * 0.37).collect();
        let s = Series::from_f64(v.clone());
        for q in [0.0, 0.1, 0.25, 0.5, 0.77, 1.0] {
            let want = s.quantile(q).unwrap().unwrap();
            let got = quantile_mut(&mut v.clone(), q);
            assert!((want - got).abs() < 1e-12, "{q}: {want} {got}");
        }
    }

    #[test]
    fn mad_and_fences() {
        let s = Series::from_f64(vec![1.0, 2.0, 3.0, 4.0, 100.0]);
        assert_eq!(mad(&s, 1.0).unwrap(), 1.0);
        assert_eq!(iqr(&s).unwrap(), 2.0);
        assert_eq!(tukey_fences(&s, 1.5).unwrap(), (-1.0, 7.0));
    }

    #[test]
    fn bootstrap_is_the_same_in_parallel() {
        let s = Series::from_f64((0..500).map(|i| (i as f64).sin() * 10.0).collect());
        let a = bootstrap(&s, None, Stat::Median, Compare::Diff, 300, 4, 0.9, false).unwrap();
        let b = bootstrap(&s, None, Stat::Median, Compare::Diff, 300, 4, 0.9, true).unwrap();
        assert_eq!(a.draws, b.draws);
        assert!(a.lo < a.estimate && a.estimate < a.hi);
    }
}

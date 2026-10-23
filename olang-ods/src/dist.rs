//! Statistical distributions: thin, total wrappers over statrs.
//!
//! Four families cover Phase 2's inference needs — normal, Student's t,
//! chi-squared, and F. Each exposes pdf / cdf / ppf (inverse cdf).
//! Sampling is deliberately absent here: the language layer samples by
//! inverse transform, drawing uniforms from olang's own seeded RNG so
//! `random.seed(n)` governs statistical sampling too.

use crate::OdsError;
use statrs::distribution::{
    ChiSquared, Continuous, ContinuousCDF, FisherSnedecor, Normal, StudentsT,
};

type Result<T> = std::result::Result<T, OdsError>;

fn bad(msg: String) -> OdsError {
    OdsError::InvalidArgument(msg)
}

fn normal(mu: f64, sigma: f64) -> Result<Normal> {
    Normal::new(mu, sigma)
        .map_err(|_| bad(format!("normal: invalid sigma {} (must be > 0)", sigma)))
}

fn students_t(df: f64) -> Result<StudentsT> {
    StudentsT::new(0.0, 1.0, df).map_err(|_| bad(format!("t: invalid df {} (must be > 0)", df)))
}

fn chi_squared(df: f64) -> Result<ChiSquared> {
    ChiSquared::new(df).map_err(|_| bad(format!("chi2: invalid df {} (must be > 0)", df)))
}

fn fisher(d1: f64, d2: f64) -> Result<FisherSnedecor> {
    FisherSnedecor::new(d1, d2)
        .map_err(|_| bad(format!("f: invalid df ({}, {}) (must be > 0)", d1, d2)))
}

fn check_p(q: f64, what: &str) -> Result<()> {
    if !(0.0..=1.0).contains(&q) {
        return Err(bad(format!("{}: p must lie in [0, 1], got {}", what, q)));
    }
    Ok(())
}

pub fn norm_pdf(x: f64, mu: f64, sigma: f64) -> Result<f64> {
    Ok(normal(mu, sigma)?.pdf(x))
}

pub fn norm_cdf(x: f64, mu: f64, sigma: f64) -> Result<f64> {
    Ok(normal(mu, sigma)?.cdf(x))
}

pub fn norm_ppf(q: f64, mu: f64, sigma: f64) -> Result<f64> {
    check_p(q, "norm.ppf")?;
    Ok(normal(mu, sigma)?.inverse_cdf(q))
}

pub fn t_pdf(x: f64, df: f64) -> Result<f64> {
    Ok(students_t(df)?.pdf(x))
}

pub fn t_cdf(x: f64, df: f64) -> Result<f64> {
    Ok(students_t(df)?.cdf(x))
}

pub fn t_ppf(q: f64, df: f64) -> Result<f64> {
    check_p(q, "t.ppf")?;
    Ok(students_t(df)?.inverse_cdf(q))
}

pub fn chi2_pdf(x: f64, df: f64) -> Result<f64> {
    Ok(chi_squared(df)?.pdf(x))
}

pub fn chi2_cdf(x: f64, df: f64) -> Result<f64> {
    Ok(chi_squared(df)?.cdf(x))
}

pub fn chi2_ppf(q: f64, df: f64) -> Result<f64> {
    check_p(q, "chi2.ppf")?;
    Ok(chi_squared(df)?.inverse_cdf(q))
}

pub fn f_pdf(x: f64, d1: f64, d2: f64) -> Result<f64> {
    Ok(fisher(d1, d2)?.pdf(x))
}

pub fn f_cdf(x: f64, d1: f64, d2: f64) -> Result<f64> {
    Ok(fisher(d1, d2)?.cdf(x))
}

pub fn f_ppf(q: f64, d1: f64, d2: f64) -> Result<f64> {
    check_p(q, "f.ppf")?;
    Ok(fisher(d1, d2)?.inverse_cdf(q))
}

// ---------------------------------------------------------------------
// Every family, vectorized
// ---------------------------------------------------------------------

use statrs::distribution::{
    Beta as BetaD, Binomial, Discrete, DiscreteCDF, Gamma as GammaD, LogNormal, Poisson,
};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// `ln Γ(x)`.
pub fn ln_gamma(x: f64) -> f64 {
    statrs::function::gamma::ln_gamma(x)
}

/// A distribution with its parameters checked once: what a vectorized
/// call evaluates a million times.
#[derive(Clone, Debug)]
pub enum Dist {
    Norm(Normal),
    T(StudentsT),
    Chi2(ChiSquared),
    F(FisherSnedecor),
    /// `lognorm(mu, sigma)`: the log is normal with this mean and sd.
    LogNorm(LogNormal),
    /// `beta(a, b)`.
    Beta(BetaD),
    /// `gamma(shape, scale)` (SciPy's scale, the reciprocal of the rate).
    Gamma(GammaD),
    /// `poisson(lam)`.
    Poisson(Poisson),
    /// `binom(trials, p)`.
    Binom(Binomial),
}

/// What to evaluate: the density (a discrete family's pmf), the CDF, its
/// inverse, or the survival function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistFn {
    Pdf,
    Cdf,
    Ppf,
    Sf,
}

impl DistFn {
    pub fn parse(name: &str) -> Option<DistFn> {
        Some(match name {
            "pdf" | "pmf" => DistFn::Pdf,
            "cdf" => DistFn::Cdf,
            "ppf" => DistFn::Ppf,
            "sf" => DistFn::Sf,
            _ => return None,
        })
    }
}

/// The parameter names of each family, in order (what the language's
/// signatures list).
pub const FAMILIES: &[(&str, &[&str])] = &[
    ("norm", &["mean", "std"]),
    ("t", &["df"]),
    ("chi2", &["df"]),
    ("f", &["d1", "d2"]),
    ("lognorm", &["mu", "sigma"]),
    ("beta", &["a", "b"]),
    ("gamma", &["shape", "scale"]),
    ("poisson", &["lam"]),
    ("binom", &["trials", "p"]),
];

impl Dist {
    /// `family` with `params` in `FAMILIES`' order.
    pub fn new(family: &str, params: &[f64]) -> Result<Dist> {
        let p = |i: usize| params.get(i).copied().unwrap_or(f64::NAN);
        Ok(match family {
            "norm" => Dist::Norm(normal(p(0), p(1))?),
            "t" => Dist::T(students_t(p(0))?),
            "chi2" => Dist::Chi2(chi_squared(p(0))?),
            "f" => Dist::F(fisher(p(0), p(1))?),
            "lognorm" => Dist::LogNorm(
                LogNormal::new(p(0), p(1))
                    .map_err(|_| bad(format!("lognorm: invalid sigma {} (must be > 0)", p(1))))?,
            ),
            "beta" => Dist::Beta(BetaD::new(p(0), p(1)).map_err(|_| {
                bad(format!(
                    "beta: invalid shapes ({}, {}) (must be > 0)",
                    p(0),
                    p(1)
                ))
            })?),
            "gamma" => {
                if !(p(1) > 0.0 && p(1).is_finite()) {
                    return Err(bad(format!("gamma: invalid scale {} (must be > 0)", p(1))));
                }
                Dist::Gamma(
                    GammaD::new(p(0), 1.0 / p(1))
                        .map_err(|_| bad(format!("gamma: invalid shape {} (must be > 0)", p(0))))?,
                )
            }
            "poisson" => Dist::Poisson(
                Poisson::new(p(0))
                    .map_err(|_| bad(format!("poisson: invalid mean {} (must be > 0)", p(0))))?,
            ),
            "binom" => {
                let n = p(0);
                if !(n >= 0.0 && n.fract() == 0.0 && n.is_finite()) {
                    return Err(bad(format!(
                        "binom: trials must be a whole number ≥ 0, got {}",
                        n
                    )));
                }
                Dist::Binom(
                    Binomial::new(p(1), n as u64).map_err(|_| {
                        bad(format!("binom: invalid p {} (must lie in [0, 1])", p(1)))
                    })?,
                )
            }
            other => return Err(bad(format!("unknown distribution {}", other))),
        })
    }

    pub fn is_discrete(&self) -> bool {
        matches!(self, Dist::Poisson(_) | Dist::Binom(_))
    }

    /// One value. A ppf outside [0, 1] is NaN; a discrete family's pmf is
    /// zero off the integers, its CDF a step at each.
    pub fn eval(&self, f: DistFn, x: f64) -> f64 {
        if x.is_nan() {
            return f64::NAN;
        }
        if f == DistFn::Ppf && !(0.0..=1.0).contains(&x) {
            return f64::NAN;
        }
        macro_rules! cont {
            ($d:expr) => {
                match f {
                    DistFn::Pdf => $d.pdf(x),
                    DistFn::Cdf => $d.cdf(x),
                    DistFn::Sf => $d.sf(x),
                    DistFn::Ppf => $d.inverse_cdf(x),
                }
            };
        }
        match self {
            Dist::Norm(d) => cont!(d),
            Dist::T(d) => cont!(d),
            Dist::Chi2(d) => cont!(d),
            Dist::F(d) => cont!(d),
            Dist::LogNorm(d) => {
                if f == DistFn::Pdf && x <= 0.0 {
                    0.0
                } else {
                    cont!(d)
                }
            }
            Dist::Beta(d) => {
                if f == DistFn::Pdf && !(0.0..=1.0).contains(&x) {
                    0.0
                } else if f != DistFn::Ppf && !(0.0..=1.0).contains(&x) {
                    let below = x < 0.0;
                    match (f, below) {
                        (DistFn::Cdf, true) | (DistFn::Sf, false) => 0.0,
                        _ => 1.0,
                    }
                } else {
                    cont!(d)
                }
            }
            Dist::Gamma(d) => {
                if x < 0.0 && f != DistFn::Ppf {
                    match f {
                        DistFn::Sf => 1.0,
                        _ => 0.0,
                    }
                } else {
                    cont!(d)
                }
            }
            Dist::Poisson(d) => discrete(d, f, x, u64::MAX),
            Dist::Binom(d) => discrete(d, f, x, d.n()),
        }
    }

    /// Every value of `xs` (NaN for a null), in parallel when `par`.
    pub fn apply(&self, f: DistFn, xs: &[f64], par: bool) -> Vec<f64> {
        #[cfg(feature = "parallel")]
        if par {
            return xs
                .par_iter()
                .with_min_len(4096)
                .map(|&x| self.eval(f, x))
                .collect();
        }
        let _ = par;
        xs.iter().map(|&x| self.eval(f, x)).collect()
    }
}

/// A discrete family at `x`: the pmf at an integer (zero off them), the
/// CDF at `floor(x)`, the ppf the least k with CDF(k) ≥ p (SciPy's).
fn discrete<D>(d: &D, f: DistFn, x: f64, max: u64) -> f64
where
    D: Discrete<u64, f64> + DiscreteCDF<u64, f64>,
{
    match f {
        DistFn::Pdf => {
            if x < 0.0 || x.fract() != 0.0 || x > max as f64 {
                0.0
            } else {
                d.pmf(x as u64)
            }
        }
        DistFn::Cdf | DistFn::Sf => {
            let c = if x < 0.0 {
                0.0
            } else if x >= max as f64 {
                1.0
            } else {
                d.cdf(x.floor() as u64)
            };
            if f == DistFn::Cdf { c } else { 1.0 - c }
        }
        DistFn::Ppf => {
            if x == 0.0 {
                return -1.0; // SciPy: ppf(0) is one below the support
            }
            if x == 1.0 {
                return if max == u64::MAX {
                    f64::INFINITY
                } else {
                    max as f64
                };
            }
            // Bracket by doubling, then bisect on the integers.
            let mut hi: u64 = 1;
            while hi < max && d.cdf(hi) < x {
                hi = hi.saturating_mul(2).min(max);
            }
            let mut lo: u64 = 0;
            if d.cdf(0) >= x {
                return 0.0;
            }
            // cdf(lo) < x ≤ cdf(hi)
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if d.cdf(mid) >= x {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            hi as f64
        }
    }
}

// ---------------------------------------------------------------------
// The noncentral t (power)
// ---------------------------------------------------------------------

fn pnorm(x: f64) -> f64 {
    0.5 * statrs::function::erf::erfc(-x / std::f64::consts::SQRT_2)
}

/// The CDF of the noncentral t with `df` degrees of freedom and
/// noncentrality `delta` at `t`: Lenth's AS 243, as R's `pnt`.
pub fn nct_cdf(t: f64, df: f64, delta: f64) -> f64 {
    if df <= 0.0 || t.is_nan() || df.is_nan() || delta.is_nan() {
        return f64::NAN;
    }
    if delta == 0.0 {
        return StudentsT::new(0.0, 1.0, df)
            .map(|d| d.cdf(t))
            .unwrap_or(f64::NAN);
    }
    if t.is_infinite() {
        return if t > 0.0 { 1.0 } else { 0.0 };
    }
    let (tt, del, negdel) = if t >= 0.0 {
        (t, delta, false)
    } else {
        (-t, -delta, true)
    };
    // Large df or noncentrality: the normal approximation (R's).
    if df > 4e5 || del * del > 2.0 * std::f64::consts::LN_2 * 1021.0 {
        let s = 1.0 / (4.0 * df);
        let z = (tt * (1.0 - s) - del) / (1.0 + tt * tt * 2.0 * s).sqrt();
        let p = pnorm(z);
        return if negdel { 1.0 - p } else { p };
    }
    let x = t * t / (t * t + df);
    let mut tnc = 0.0;
    if x > 0.0 {
        let lambda = del * del;
        let mut p = 0.5 * (-0.5 * lambda).exp();
        let mut q = (2.0 / std::f64::consts::PI).sqrt() * p * del;
        let mut s = 0.5 - p;
        if s < 1e-7 {
            s = -0.5 * (-0.5 * lambda).exp_m1();
        }
        let mut a = 0.5;
        let b = 0.5 * df;
        let rxb = (1.0 - x).powf(b);
        let albeta = 0.5 * std::f64::consts::PI.ln() + ln_gamma(b) - ln_gamma(0.5 + b);
        let mut xodd = statrs::function::beta::beta_reg(a, b, x);
        let mut godd = 2.0 * rxb * (a * x.ln() - albeta).exp();
        let mut tnc_b = b * x;
        let mut xeven = if tnc_b < f64::EPSILON {
            tnc_b
        } else {
            1.0 - rxb
        };
        let mut geven = tnc_b * rxb;
        tnc_b = p * xodd + q * xeven;
        tnc = tnc_b;
        for it in 1..=1000 {
            a += 1.0;
            xodd -= godd;
            xeven -= geven;
            godd *= x * (a + b - 1.0) / a;
            geven *= x * (a + b - 0.5) / (a + 0.5);
            p *= lambda / (2.0 * it as f64);
            q *= lambda / (2.0 * it as f64 + 1.0);
            tnc += p * xodd + q * xeven;
            s -= p;
            if s < -1e-10 {
                break;
            }
            if s <= 0.0 && it > 1 {
                break;
            }
            let errbd = 2.0 * s * (xodd - godd);
            if errbd.abs() < 1e-12 {
                break;
            }
        }
    }
    tnc += pnorm(-del);
    let out = if negdel { 1.0 - tnc } else { tnc };
    out.clamp(0.0, 1.0)
}

/// The power of a t-test: `effect` the standardized difference (Cohen's
/// d), `n` the first group's size (`ratio` the second's over it) for two
/// samples, the sample's for one. statsmodels' `TTestIndPower` and
/// `TTestPower`.
pub fn power_t(
    effect: f64,
    n: f64,
    alpha: f64,
    alternative: &str,
    two_sample: bool,
    ratio: f64,
) -> Result<f64> {
    if !(alpha > 0.0 && alpha < 1.0) {
        return Err(bad(format!(
            "power_t: alpha must lie in (0, 1), got {}",
            alpha
        )));
    }
    let (df, nobs) = if two_sample {
        let n2 = n * ratio;
        (n + n2 - 2.0, 1.0 / (1.0 / n + 1.0 / n2))
    } else {
        (n - 1.0, n)
    };
    if df.is_nan() || df <= 0.0 {
        return Err(bad(format!("power_t: n too small ({})", n)));
    }
    let nc = effect * nobs.sqrt();
    let t = students_t(df)?;
    Ok(match alternative {
        "two-sided" => {
            let upp = t.inverse_cdf(1.0 - alpha / 2.0);
            let low = t.inverse_cdf(alpha / 2.0);
            (1.0 - nct_cdf(upp, df, nc)) + nct_cdf(low, df, nc)
        }
        "larger" | "greater" => 1.0 - nct_cdf(t.inverse_cdf(1.0 - alpha), df, nc),
        "smaller" | "less" => nct_cdf(t.inverse_cdf(alpha), df, nc),
        other => {
            return Err(bad(format!(
                "power_t: alternative must be \"two-sided\", \"greater\" or \"less\", got \"{}\"",
                other
            )));
        }
    })
}

/// The `n` at which `power_t` reaches `power` (a fraction, as
/// statsmodels' `solve_power`): bisection on n, to 1e-10 relative.
pub fn n_for_power(
    effect: f64,
    power: f64,
    alpha: f64,
    alternative: &str,
    two_sample: bool,
    ratio: f64,
) -> Result<f64> {
    if !(power > 0.0 && power < 1.0) {
        return Err(bad(format!(
            "n_for_power: power must lie in (0, 1), got {}",
            power
        )));
    }
    if effect == 0.0 || !effect.is_finite() {
        return Err(bad(
            "n_for_power: the effect must be a nonzero number".to_string()
        ));
    }
    let f = |n: f64| power_t(effect, n, alpha, alternative, two_sample, ratio).map(|p| p - power);
    // The smallest n with a degree of freedom.
    let mut lo = if two_sample {
        1.0 + 1.0 / (1.0 + ratio) + 1e-9
    } else {
        1.0 + 1e-9
    };
    lo = lo.max(1.000001);
    if f(lo)? >= 0.0 {
        return Ok(lo);
    }
    let mut hi = 2.0f64.max(lo * 2.0);
    while f(hi)? < 0.0 {
        hi *= 2.0;
        if hi > 1e12 {
            return Err(bad(format!(
                "n_for_power: no n up to 1e12 reaches power {} (is the effect in the alternative's direction?)",
                power
            )));
        }
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if f(mid)? < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo <= 1e-10 * hi {
            break;
        }
    }
    Ok(0.5 * (lo + hi))
}

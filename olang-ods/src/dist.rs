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

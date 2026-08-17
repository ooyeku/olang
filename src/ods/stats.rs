//! The `stats` namespace: L2 of the ods layer cake — thin tables over
//! the engine's inference kernels and distributions. No numeric loops
//! live here (the L0/L2 invariant in docs/design/ods.md); this file is
//! argument checking, Value↔engine translation, and result shaping.
//!
//! Results come back as olang maps (`#{...}`) so callers destructure
//! with the language's own tools: `stats.t_test(a, b)` is
//! `#{ t, df, p_value, mean_a, mean_b }`, `stats.lm(y, xs)` carries its
//! coefficient table as parallel Series.
//!
//! Sampling (`stats.norm.sample(n, mu, sigma)`, ...) is inverse-CDF over
//! uniforms drawn from the `random` module's seeded stream — so
//! `random.seed(k)` governs statistical sampling exactly as it governs
//! `random.gauss`.

use super::series::{make_series_value, series_of};
use crate::ast::{BuiltinFunction, Value};
use olang_ods::{Series, dist, stats};
use std::collections::HashMap;
use std::sync::Arc;

/// (name, arity) of the flat stats functions.
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("describe", 1),
    ("corr", 2),
    ("cov", 2),
    ("t_test", 2),
    ("chi2_test", 2),
    ("lm", 2),
];

/// (family, function, arity) of the nested distribution namespaces:
/// stats.norm.pdf(x, mu, sigma), stats.t.cdf(x, df), ...
pub const DIST_FUNCTIONS: &[(&str, &str, usize)] = &[
    ("norm", "pdf", 3),
    ("norm", "cdf", 3),
    ("norm", "ppf", 3),
    ("norm", "sample", 3),
    ("t", "pdf", 2),
    ("t", "cdf", 2),
    ("t", "ppf", 2),
    ("t", "sample", 2),
    ("chi2", "pdf", 2),
    ("chi2", "cdf", 2),
    ("chi2", "ppf", 2),
    ("chi2", "sample", 2),
    ("f", "pdf", 3),
    ("f", "cdf", 3),
    ("f", "ppf", 3),
    ("f", "sample", 3),
];

fn builtin(name: &str, arity: usize) -> Value {
    Value::Builtin(BuiltinFunction {
        name: format!("stats.{}", name),
        arity,
    })
}

/// The `stats` module value: flat functions plus one nested module per
/// distribution family.
pub fn namespace() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in FUNCTIONS {
        module.insert(name.to_string(), builtin(name, *arity));
    }
    for family in ["norm", "t", "chi2", "f"] {
        let mut fam = HashMap::new();
        for (f, func, arity) in DIST_FUNCTIONS {
            if *f == family {
                fam.insert(
                    func.to_string(),
                    builtin(&format!("{}.{}", family, func), *arity),
                );
            }
        }
        module.insert(
            family.to_string(),
            Value::Struct {
                type_name: "Module".to_string(),
                fields: std::sync::Arc::new(fam),
            },
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

pub fn handles(func: &str) -> bool {
    FUNCTIONS.iter().any(|(n, _)| *n == func)
        || DIST_FUNCTIONS
            .iter()
            .any(|(fam, f, _)| format!("{}.{}", fam, f) == func)
}

// ---------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------

fn num(func: &str, args: &[Value], idx: usize) -> Result<f64, String> {
    match args.get(idx) {
        Some(Value::Float(x)) => Ok(*x),
        Some(Value::Integer(x)) => Ok(*x as f64),
        other => Err(format!(
            "stats.{}: argument {} must be numeric, got {}",
            func,
            idx + 1,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

fn want_series<'a>(func: &str, args: &'a [Value], idx: usize) -> Result<&'a Series, String> {
    args.get(idx).and_then(series_of).ok_or_else(|| {
        format!(
            "stats.{}: argument {} must be a Series, got {}",
            func,
            idx + 1,
            args.get(idx).map(|v| v.type_name()).unwrap_or_default()
        )
    })
}

fn count(func: &str, args: &[Value], idx: usize) -> Result<usize, String> {
    match args.get(idx) {
        Some(Value::Integer(n)) if *n >= 0 => Ok(*n as usize),
        other => Err(format!(
            "stats.{}: argument {} must be a non-negative Int, got {}",
            func,
            idx + 1,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

fn map_value(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(Arc::new(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    ))
}

fn opt_float(o: Option<f64>) -> Value {
    o.map(Value::Float).unwrap_or(Value::Unit)
}

fn e(err: olang_ods::OdsError) -> String {
    err.to_string()
}

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

pub fn dispatch(func: &str, args: Vec<Value>) -> Result<Value, String> {
    // Distribution families first: "norm.pdf", "t.cdf", ...
    if let Some((family, inner)) = func.split_once('.') {
        return dispatch_dist(family, inner, args);
    }
    let expected = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, a)| *a)
        .expect("handles() checked membership");
    if args.len() != expected {
        return Err(format!(
            "stats.{} expects {} argument{}, got {}",
            func,
            expected,
            if expected == 1 { "" } else { "s" },
            args.len()
        ));
    }
    match func {
        "describe" => {
            let s = want_series(func, &args, 0)?;
            let par = crate::parallel::should_parallelize(s.len());
            Ok(map_value(vec![
                ("count", Value::Integer((s.len() - s.null_count()) as i64)),
                ("null_count", Value::Integer(s.null_count() as i64)),
                ("mean", opt_float(s.mean(par).map_err(e)?)),
                ("std", opt_float(s.std(par).map_err(e)?)),
                ("min", super::series::scalar_to_value(s.min().map_err(e)?)),
                ("q25", opt_float(s.quantile(0.25).map_err(e)?)),
                ("median", opt_float(s.quantile(0.5).map_err(e)?)),
                ("q75", opt_float(s.quantile(0.75).map_err(e)?)),
                ("max", super::series::scalar_to_value(s.max().map_err(e)?)),
            ]))
        }
        "corr" => {
            let a = want_series(func, &args, 0)?;
            let b = want_series(func, &args, 1)?;
            Ok(opt_float(stats::corr(a, b).map_err(e)?))
        }
        "cov" => {
            let a = want_series(func, &args, 0)?;
            let b = want_series(func, &args, 1)?;
            Ok(opt_float(stats::cov(a, b).map_err(e)?))
        }
        "t_test" => {
            let a = want_series(func, &args, 0)?;
            // Second argument decides the test: a Series is Welch's
            // two-sample, a number is the one-sample null mean.
            if let Some(b) = series_of(&args[1]) {
                let r = stats::t_test_welch(a, b).map_err(e)?;
                Ok(map_value(vec![
                    ("t", Value::Float(r.t)),
                    ("df", Value::Float(r.df)),
                    ("p_value", Value::Float(r.p_value)),
                    ("mean_a", Value::Float(r.mean)),
                    ("mean_b", Value::Float(r.mean_other)),
                ]))
            } else {
                let mu0 = num(func, &args, 1)?;
                let r = stats::t_test_one_sample(a, mu0).map_err(e)?;
                Ok(map_value(vec![
                    ("t", Value::Float(r.t)),
                    ("df", Value::Float(r.df)),
                    ("p_value", Value::Float(r.p_value)),
                    ("mean", Value::Float(r.mean)),
                ]))
            }
        }
        "chi2_test" => {
            let obs = want_series(func, &args, 0)?;
            let exp = want_series(func, &args, 1)?;
            let r = stats::chi2_gof(obs, exp).map_err(e)?;
            Ok(map_value(vec![
                ("chi2", Value::Float(r.statistic)),
                ("df", Value::Float(r.df)),
                ("p_value", Value::Float(r.p_value)),
            ]))
        }
        "lm" => {
            let y = want_series(func, &args, 0)?;
            let x_values: Vec<&Series> = match &args[1] {
                Value::List(items) => items
                    .iter()
                    .map(|v| {
                        series_of(v).ok_or_else(|| {
                            "stats.lm: regressors must be a Series or a list of Series".to_string()
                        })
                    })
                    .collect::<Result<_, _>>()?,
                other => vec![series_of(other).ok_or_else(|| {
                    format!(
                        "stats.lm: regressors must be a Series or a list of Series, got {}",
                        other.type_name()
                    )
                })?],
            };
            let par = crate::parallel::should_parallelize(y.len());
            let fit = stats::ols(y, &x_values, par).map_err(e)?;
            Ok(map_value(vec![
                ("coef", make_series_value(Series::from_f64(fit.coef))),
                ("se", make_series_value(Series::from_f64(fit.se))),
                ("t", make_series_value(Series::from_f64(fit.t))),
                ("p_value", make_series_value(Series::from_f64(fit.p_value))),
                ("r2", Value::Float(fit.r2)),
                ("adj_r2", Value::Float(fit.adj_r2)),
                ("n", Value::Integer(fit.n as i64)),
                ("df_resid", Value::Float(fit.df_resid)),
            ]))
        }
        _ => unreachable!("handles() checked membership"),
    }
}

fn dispatch_dist(family: &str, func: &str, args: Vec<Value>) -> Result<Value, String> {
    let full = format!("{}.{}", family, func);
    let expected = DIST_FUNCTIONS
        .iter()
        .find(|(fam, f, _)| *fam == family && *f == func)
        .map(|(_, _, a)| *a)
        .ok_or_else(|| format!("unknown stats function: stats.{}", full))?;
    if args.len() != expected {
        return Err(format!(
            "stats.{} expects {} argument{}, got {}",
            full,
            expected,
            if expected == 1 { "" } else { "s" },
            args.len()
        ));
    }

    if func == "sample" {
        let n = count(&full, &args, 0)?;
        // Inverse transform over the random module's seeded stream.
        // Clamp away from {0, 1} so ppf stays finite.
        let ppf: Box<dyn Fn(f64) -> Result<f64, olang_ods::OdsError>> = match family {
            "norm" => {
                let mu = num(&full, &args, 1)?;
                let sigma = num(&full, &args, 2)?;
                Box::new(move |u| dist::norm_ppf(u, mu, sigma))
            }
            "t" => {
                let df = num(&full, &args, 1)?;
                Box::new(move |u| dist::t_ppf(u, df))
            }
            "chi2" => {
                let df = num(&full, &args, 1)?;
                Box::new(move |u| dist::chi2_ppf(u, df))
            }
            "f" => {
                let d1 = num(&full, &args, 1)?;
                let d2 = num(&full, &args, 2)?;
                Box::new(move |u| dist::f_ppf(u, d1, d2))
            }
            _ => unreachable!("family checked above"),
        };
        let draws = crate::stdlib::random::draw_uniforms(n)
            .into_iter()
            .map(|u| ppf(u.clamp(1e-16, 1.0 - 1e-16)))
            .collect::<Result<Vec<f64>, _>>()
            .map_err(e)?;
        return Ok(make_series_value(Series::from_f64(draws)));
    }

    let x = num(&full, &args, 0)?;
    let out = match (family, func) {
        ("norm", _) => {
            let mu = num(&full, &args, 1)?;
            let sigma = num(&full, &args, 2)?;
            match func {
                "pdf" => dist::norm_pdf(x, mu, sigma),
                "cdf" => dist::norm_cdf(x, mu, sigma),
                _ => dist::norm_ppf(x, mu, sigma),
            }
        }
        ("t", _) => {
            let df = num(&full, &args, 1)?;
            match func {
                "pdf" => dist::t_pdf(x, df),
                "cdf" => dist::t_cdf(x, df),
                _ => dist::t_ppf(x, df),
            }
        }
        ("chi2", _) => {
            let df = num(&full, &args, 1)?;
            match func {
                "pdf" => dist::chi2_pdf(x, df),
                "cdf" => dist::chi2_cdf(x, df),
                _ => dist::chi2_ppf(x, df),
            }
        }
        ("f", _) => {
            let d1 = num(&full, &args, 1)?;
            let d2 = num(&full, &args, 2)?;
            match func {
                "pdf" => dist::f_pdf(x, d1, d2),
                "cdf" => dist::f_cdf(x, d1, d2),
                _ => dist::f_ppf(x, d1, d2),
            }
        }
        _ => return Err(format!("unknown stats function: stats.{}", full)),
    };
    out.map(Value::Float).map_err(e)
}

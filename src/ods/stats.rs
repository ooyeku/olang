//! The `stats` namespace: L2 of the ods layer cake — thin tables over
//! the engine's inference kernels and distributions. No numeric loops
//! live here (the L0/L2 invariant in docs/design/ods.md); this file is
//! argument checking, Value↔engine translation, and result shaping.
//!
//! Results come back as olang maps (`#{...}`) so callers destructure
//! with the language's own tools: `stats.t_test(a, b)` is
//! `#{ t, df, p_value, mean_a, mean_b, diff, se, ci, d }`, `stats.lm(y,
//! xs)` carries its coefficient table as parallel Series.
//!
//! Distributions are vectorized: `stats.norm.cdf(x)` of a number is a
//! number, of a Series a Series (nulls kept), one native pass. Sampling
//! (`stats.norm.sample(n, mu, sigma)`, ...) draws from a generator when
//! given `rng:` (a `random.rng` or an Int seed), else by inverse CDF over
//! uniforms from the `random` module's seeded stream — so `random.seed(k)`
//! governs statistical sampling exactly as it governs `random.gauss`.
//!
//! `stats.ts.*` is `super::ts`.

use super::rng::with_source;
use super::series::{make_series_value, series_of};
use crate::ast::{BuiltinFunction, Value};
use olang_ods::dist::{Dist, DistFn};
use olang_ods::robust::{self, Compare, Stat};
use olang_ods::{ParClass, Series, dist, stats};
use std::sync::Arc;

/// (name, fewest arguments) of the flat stats functions.
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("describe", 1),
    ("corr", 2),
    ("cov", 2),
    ("t_test", 2),
    ("chi2_test", 2),
    ("lm", 2),
    ("power_t", 2),
    ("n_for_power", 1),
    ("mann_whitney", 2),
    ("spearman", 2),
    ("mad", 1),
    ("iqr", 1),
    ("zscore", 1),
    ("tukey_fences", 1),
    ("bootstrap", 1),
];

/// (family, its functions). A discrete family has a pmf where a
/// continuous one has a pdf.
pub const FAMILIES: &[(&str, &[&str])] = &[
    ("norm", &["pdf", "cdf", "ppf", "sample"]),
    ("t", &["pdf", "cdf", "ppf", "sample"]),
    ("chi2", &["pdf", "cdf", "ppf", "sample"]),
    ("f", &["pdf", "cdf", "ppf", "sample"]),
    ("lognorm", &["pdf", "cdf", "ppf", "sample"]),
    ("beta", &["pdf", "cdf", "ppf", "sample"]),
    ("gamma", &["pdf", "cdf", "ppf", "sample"]),
    ("poisson", &["pmf", "cdf", "ppf", "sample"]),
    ("binom", &["pmf", "cdf", "ppf", "sample"]),
];

/// The parameters of a family, with their defaults (`None`: required).
fn family_params(family: &str) -> &'static [(&'static str, Option<f64>)] {
    match family {
        "norm" => &[("mean", Some(0.0)), ("std", Some(1.0))],
        "t" | "chi2" => &[("df", None)],
        "f" => &[("d1", None), ("d2", None)],
        "lognorm" => &[("mu", Some(0.0)), ("sigma", Some(1.0))],
        "beta" => &[("a", None), ("b", None)],
        "gamma" => &[("shape", None), ("scale", Some(1.0))],
        "poisson" => &[("lam", None)],
        "binom" => &[("trials", None), ("p", None)],
        _ => &[],
    }
}

fn builtin(name: &str, arity: usize) -> Value {
    Value::Builtin(BuiltinFunction {
        name: format!("stats.{}", name),
        arity,
    })
}

fn module_value(fields: crate::ast::ValueMap) -> Value {
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(fields),
    }
}

/// The `stats` module value: flat functions, one nested module per
/// distribution family, and `stats.ts`.
pub fn namespace() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in FUNCTIONS {
        module.insert(name.to_string(), builtin(name, *arity));
    }
    for (family, funcs) in FAMILIES {
        let mut fam = crate::ast::ValueMap::default();
        let required = family_params(family)
            .iter()
            .filter(|p| p.1.is_none())
            .count();
        for func in funcs.iter() {
            fam.insert(
                func.to_string(),
                builtin(&format!("{}.{}", family, func), 1 + required),
            );
        }
        module.insert(family.to_string(), module_value(fam));
    }
    let mut ts = crate::ast::ValueMap::default();
    for (name, arity) in super::ts::FUNCTIONS {
        ts.insert(name.to_string(), builtin(&format!("ts.{}", name), *arity));
    }
    module.insert("ts".to_string(), module_value(ts));
    module_value(module)
}

pub fn handles(func: &str) -> bool {
    if let Some(inner) = func.strip_prefix("ts.") {
        return super::ts::FUNCTIONS.iter().any(|(n, _)| *n == inner);
    }
    if let Some((fam, f)) = func.split_once('.') {
        return FAMILIES
            .iter()
            .any(|(name, funcs)| *name == fam && funcs.contains(&f));
    }
    FUNCTIONS.iter().any(|(n, _)| *n == func)
}

// ---------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------

fn given(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !matches!(v, Value::Unit))
}

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

/// A numeric argument, or `default` when absent or `()`.
fn num_or(func: &str, args: &[Value], idx: usize, default: f64) -> Result<f64, String> {
    match given(args.get(idx)) {
        None => Ok(default),
        Some(_) => num(func, args, idx),
    }
}

fn text_or<'a>(
    func: &str,
    what: &str,
    v: Option<&'a Value>,
    default: &'a str,
) -> Result<&'a str, String> {
    match given(v) {
        None => Ok(default),
        Some(Value::String(s)) => Ok(s.as_str()),
        Some(other) => Err(format!(
            "stats.{}: {} must be a String, got {}",
            func,
            what,
            other.type_name()
        )),
    }
}

fn bool_or(func: &str, what: &str, v: Option<&Value>, default: bool) -> Result<bool, String> {
    match given(v) {
        None => Ok(default),
        Some(Value::Boolean(b)) => Ok(*b),
        Some(other) => Err(format!(
            "stats.{}: {} must be a Bool, got {}",
            func,
            what,
            other.type_name()
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

pub(super) fn map_value(entries: Vec<(&str, Value)>) -> Value {
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

fn pair(lo: f64, hi: f64) -> Value {
    Value::Tuple(Arc::new(vec![Value::Float(lo), Value::Float(hi)]))
}

fn e(err: olang_ods::OdsError) -> String {
    err.to_string()
}

fn alternative<'a>(func: &str, v: Option<&'a Value>) -> Result<&'a str, String> {
    let a = text_or(func, "alternative", v, "two-sided")?;
    match a {
        "two-sided" | "greater" | "less" => Ok(a),
        other => Err(format!(
            "stats.{}: alternative must be \"two-sided\", \"greater\" or \"less\", got \"{}\"",
            func, other
        )),
    }
}

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

pub fn dispatch(func: &str, args: Vec<Value>) -> Result<Value, String> {
    if let Some(inner) = func.strip_prefix("ts.") {
        return super::ts::dispatch(inner, &args);
    }
    if let Some((family, inner)) = func.split_once('.') {
        return dispatch_dist(family, inner, args);
    }
    let fewest = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, a)| *a)
        .expect("handles() checked membership");
    if args.len() < fewest {
        return Err(format!(
            "stats.{} expects at least {} argument{}, got {}",
            func,
            fewest,
            if fewest == 1 { "" } else { "s" },
            args.len()
        ));
    }
    let par_of = |s: &Series| super::series::par(ParClass::Reduce, s.len());
    match func {
        "describe" => {
            let s = want_series(func, &args, 0)?;
            let par = par_of(s);
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
        "t_test" => t_test(&args),
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
            let par = par_of(y);
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
        "power_t" | "n_for_power" => power(func, &args),
        "mann_whitney" => {
            let a = want_series(func, &args, 0)?;
            let b = want_series(func, &args, 1)?;
            let alt = alternative(func, args.get(2))?;
            let r = stats::mann_whitney(a, b, alt).map_err(e)?;
            Ok(map_value(vec![
                ("u", Value::Float(r.u)),
                ("z", Value::Float(r.z)),
                ("p_value", Value::Float(r.p_value)),
                ("effect", Value::Float(r.effect)),
            ]))
        }
        "spearman" => {
            let a = want_series(func, &args, 0)?;
            let b = want_series(func, &args, 1)?;
            let r = stats::spearman(a, b).map_err(e)?;
            Ok(map_value(vec![
                ("rho", Value::Float(r.rho)),
                ("p_value", Value::Float(r.p_value)),
                ("n", Value::Integer(r.n as i64)),
            ]))
        }
        "mad" => {
            let s = want_series(func, &args, 0)?;
            let scale = match given(args.get(1)) {
                None => robust::MAD_NORMAL,
                Some(Value::String(t)) if t.as_str() == "normal" => robust::MAD_NORMAL,
                Some(Value::Float(x)) => *x,
                Some(Value::Integer(x)) => *x as f64,
                Some(other) => {
                    return Err(format!(
                        "stats.mad: scale is \"normal\" or a number, got {}",
                        other
                    ));
                }
            };
            Ok(Value::Float(robust::mad(s, scale).map_err(e)?))
        }
        "iqr" => Ok(Value::Float(
            robust::iqr(want_series(func, &args, 0)?).map_err(e)?,
        )),
        "zscore" => {
            let s = want_series(func, &args, 0)?;
            let robust_ = bool_or(func, "robust", args.get(1), false)?;
            robust::zscore(s, robust_, par_of(s))
                .map(make_series_value)
                .map_err(e)
        }
        "tukey_fences" => {
            let s = want_series(func, &args, 0)?;
            let k = num_or(func, &args, 1, 1.5)?;
            let (lo, hi) = robust::tukey_fences(s, k).map_err(e)?;
            Ok(pair(lo, hi))
        }
        "bootstrap" => bootstrap(&args),
        _ => unreachable!("handles() checked membership"),
    }
}

/// `stats.t_test(a, b, level = 0.95, alternative = "two-sided")`.
fn t_test(args: &[Value]) -> Result<Value, String> {
    let func = "t_test";
    let a = want_series(func, args, 0)?;
    let level = num_or(func, args, 2, 0.95)?;
    if !(level > 0.0 && level < 1.0) {
        return Err(format!(
            "stats.t_test: level must lie in (0, 1), got {}",
            level
        ));
    }
    let alt = alternative(func, args.get(3))?;
    // The second argument decides the test: a Series is Welch's
    // two-sample, a number the one-sample null mean.
    let (r, two, mu0) = match series_of(&args[1]) {
        Some(b) => (stats::t_test_welch(a, b).map_err(e)?, true, 0.0),
        None => {
            let mu0 = num(func, args, 1)?;
            (stats::t_test_one_sample(a, mu0).map_err(e)?, false, mu0)
        }
    };
    let diff = if two {
        r.mean - r.mean_other
    } else {
        r.mean - mu0
    };
    // The interval is of the difference, or for one sample of the mean
    // (SciPy's and R's).
    let center = if two { diff } else { r.mean };
    let tdist = |q: f64| dist::t_ppf(q, r.df).map_err(e);
    let (p, lo, hi) = match alt {
        "two-sided" => {
            let q = tdist(1.0 - (1.0 - level) / 2.0)?;
            (r.p_value, center - q * r.se, center + q * r.se)
        }
        "greater" => {
            let q = tdist(level)?;
            (
                1.0 - dist::t_cdf(r.t, r.df).map_err(e)?,
                center - q * r.se,
                f64::INFINITY,
            )
        }
        _ => {
            let q = tdist(level)?;
            (
                dist::t_cdf(r.t, r.df).map_err(e)?,
                f64::NEG_INFINITY,
                center + q * r.se,
            )
        }
    };
    let mut entries = vec![
        ("t", Value::Float(r.t)),
        ("df", Value::Float(r.df)),
        ("p_value", Value::Float(p)),
    ];
    if two {
        entries.push(("mean_a", Value::Float(r.mean)));
        entries.push(("mean_b", Value::Float(r.mean_other)));
    } else {
        entries.push(("mean", Value::Float(r.mean)));
    }
    entries.extend([
        ("diff", Value::Float(diff)),
        ("se", Value::Float(r.se)),
        ("ci", pair(lo, hi)),
        ("d", Value::Float(r.d)),
    ]);
    Ok(map_value(entries))
}

/// `stats.power_t(effect, n, alpha, alternative, kind, ratio)` and
/// `stats.n_for_power(effect, power, alpha, alternative, kind, ratio)`.
fn power(func: &str, args: &[Value]) -> Result<Value, String> {
    let effect = num(func, args, 0)?;
    let second = if func == "power_t" {
        num(func, args, 1)?
    } else {
        num_or(func, args, 1, 0.8)?
    };
    let alpha = num_or(func, args, 2, 0.05)?;
    let alt = alternative(func, args.get(3))?;
    let two = match text_or(func, "kind", args.get(4), "two-sample")? {
        "two-sample" => true,
        "one-sample" | "paired" => false,
        other => {
            return Err(format!(
                "stats.{}: kind must be \"two-sample\", \"one-sample\" or \"paired\", got \"{}\"",
                func, other
            ));
        }
    };
    let ratio = num_or(func, args, 5, 1.0)?;
    if !(ratio > 0.0) {
        return Err(format!(
            "stats.{}: ratio must be positive, got {}",
            func, ratio
        ));
    }
    let r = if func == "power_t" {
        dist::power_t(effect, second, alpha, alt, two, ratio)
    } else {
        dist::n_for_power(effect, second, alpha, alt, two, ratio)
    };
    r.map(Value::Float).map_err(e)
}

/// `stats.bootstrap(s, stat = "mean", n = 2000, seed?, rng?, level = 0.95,
/// compare = "diff")`: `s` one Series, or a list of two (each resampled on
/// its own, their statistics compared).
fn bootstrap(args: &[Value]) -> Result<Value, String> {
    let func = "bootstrap";
    let (a, b) = match &args[0] {
        Value::List(items) if items.len() == 2 => {
            match (series_of(&items[0]), series_of(&items[1])) {
                (Some(a), Some(b)) => (a, Some(b)),
                _ => return Err("stats.bootstrap: a list of samples holds two Series".to_string()),
            }
        }
        v => match series_of(v) {
            Some(s) => (s, None),
            None => {
                return Err(format!(
                    "stats.bootstrap: the sample is a Series or a list of two, got {}",
                    v.type_name()
                ));
            }
        },
    };
    let stat = match given(args.get(1)) {
        None => Stat::Mean,
        Some(Value::String(t)) => Stat::parse(t).ok_or_else(|| {
            format!(
                "stats.bootstrap: stat is \"mean\", \"median\", \"std\", \"var\", \"sum\" or a quantile (a number in [0, 1]), got \"{}\"",
                t
            )
        })?,
        Some(Value::Float(q)) => Stat::Quantile(*q),
        Some(other) => {
            return Err(format!(
                "stats.bootstrap: stat is a name or a quantile, got {}",
                other.type_name()
            ));
        }
    };
    let n = match given(args.get(2)) {
        None => 2000,
        Some(_) => count(func, args, 2)?,
    };
    let level = num_or(func, args, 5, 0.95)?;
    let compare = match text_or(func, "compare", args.get(6), "diff")? {
        "diff" => Compare::Diff,
        "ratio" => Compare::Ratio,
        other => {
            return Err(format!(
                "stats.bootstrap: compare is \"diff\" or \"ratio\", got \"{}\"",
                other
            ));
        }
    };
    let seed = with_source("stats.bootstrap", args.get(4), args.get(3), |g| {
        g.draw_seed()
    })?;
    let work = n.saturating_mul(a.len() + b.map_or(0, |b| b.len()));
    let par = super::series::par(ParClass::Reduce, work);
    let r = robust::bootstrap(a, b, stat, compare, n, seed, level, par).map_err(e)?;
    Ok(map_value(vec![
        ("estimate", Value::Float(r.estimate)),
        ("lo", Value::Float(r.lo)),
        ("hi", Value::Float(r.hi)),
        ("ci", pair(r.lo, r.hi)),
        ("se", Value::Float(r.se)),
        ("draws", make_series_value(Series::from_f64(r.draws))),
    ]))
}

// ---------------------------------------------------------------------
// Distributions
// ---------------------------------------------------------------------

/// A Series as f64 values (NaN at a null) and its validity, for a
/// vectorized evaluation.
fn as_f64(func: &str, s: &Series) -> Result<(Vec<f64>, Option<olang_ods::bitmap::Bitmap>), String> {
    match s {
        Series::F64 { values, validity } => Ok((values.as_ref().clone(), validity.clone())),
        Series::I64 { values, validity } => {
            Ok((values.iter().map(|&x| x as f64).collect(), validity.clone()))
        }
        other => Err(format!(
            "stats.{}: needs a numeric Series, got a {} column",
            func,
            other.dtype()
        )),
    }
}

fn dispatch_dist(family: &str, func: &str, args: Vec<Value>) -> Result<Value, String> {
    let full = format!("{}.{}", family, func);
    let params = family_params(family);
    let required = params.iter().filter(|p| p.1.is_none()).count();
    let max = 1 + params.len() + usize::from(func == "sample");
    if args.len() < 1 + required || args.len() > max {
        return Err(format!(
            "stats.{} expects {} to {} arguments, got {}",
            full,
            1 + required,
            max,
            args.len()
        ));
    }
    let mut values = Vec::with_capacity(params.len());
    for (i, (name, default)) in params.iter().enumerate() {
        let v = match (given(args.get(1 + i)), default) {
            (None, Some(d)) => *d,
            (None, None) => {
                return Err(format!(
                    "stats.{}: missing {} (argument {})",
                    full,
                    name,
                    i + 2
                ));
            }
            _ => num(&full, &args, 1 + i)?,
        };
        values.push(v);
    }
    let d = Dist::new(family, &values).map_err(e)?;

    if func == "sample" {
        let n = count(&full, &args, 0)?;
        let par = super::series::par(ParClass::Sort, n);
        let rng = given(args.get(1 + params.len()));
        if let Some(rng) = rng {
            use olang_ods::random as k;
            let out: Series =
                with_source(
                    &format!("stats.{}", full),
                    Some(rng),
                    None,
                    |g| match family {
                        "norm" => Series::from_f64(k::normal(g, n, values[0], values[1], par)),
                        "lognorm" => {
                            Series::from_f64(k::lognormal(g, n, values[0], values[1], par))
                        }
                        "poisson" => Series::from_i64(k::poisson(g, n, values[0], par)),
                        "binom" => {
                            Series::from_i64(k::binomial(g, n, values[0] as i64, values[1], par))
                        }
                        _ => Series::from_f64(k::by_ppf(g, n, par, |u| d.eval(DistFn::Ppf, u))),
                    },
                )?;
            return Ok(make_series_value(out));
        }
        // Inverse transform over the random module's seeded stream.
        // Clamp away from {0, 1} so ppf stays finite.
        let u: Vec<f64> = crate::stdlib::random::draw_uniforms(n)
            .into_iter()
            .map(|u| u.clamp(1e-16, 1.0 - 1e-16))
            .collect();
        let draws = d.apply(DistFn::Ppf, &u, dist_par(n));
        return Ok(make_series_value(if d.is_discrete() {
            Series::from_i64(draws.into_iter().map(|x| x as i64).collect())
        } else {
            Series::from_f64(draws)
        }));
    }

    let which =
        DistFn::parse(func).ok_or_else(|| format!("unknown stats function: stats.{}", full))?;
    match &args[0] {
        Value::Float(x) => Ok(Value::Float(checked(&d, which, *x, &full)?)),
        Value::Integer(x) => Ok(Value::Float(checked(&d, which, *x as f64, &full)?)),
        v => match series_of(v) {
            Some(s) => {
                let (xs, validity) = as_f64(&full, s)?;
                let par = dist_par(xs.len());
                // A Float column for every family: a count's ppf can be ±inf.
                let out = d.apply(which, &xs, par);
                Ok(make_series_value(Series::F64 {
                    values: Arc::new(out),
                    validity,
                }))
            }
            None => Err(format!(
                "stats.{}: x must be a number or a Series, got {}",
                full,
                v.type_name()
            )),
        },
    }
}

/// Whether a distribution over `n` values fans out: its functions cost
/// tens of nanoseconds a value (a special function each), so the pool
/// pays from a few thousand values.
fn dist_par(n: usize) -> bool {
    n >= 16_384 && crate::parallel::should_parallelize(n) && olang_ods::parallel_enabled()
}

/// One value: a ppf outside [0, 1] raises (a column's is NaN).
fn checked(d: &Dist, which: DistFn, x: f64, full: &str) -> Result<f64, String> {
    if which == DistFn::Ppf && !(0.0..=1.0).contains(&x) {
        return Err(format!(
            "{}.ppf: p must lie in [0, 1], got {}",
            full.split('.').next().unwrap_or(full),
            x
        ));
    }
    Ok(d.eval(which, x))
}

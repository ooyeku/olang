//! `stats.ts`: the time-series functions, over `olang_ods::ts`.
//!
//! A series is a Series of numbers with no nulls (a gap has no lag to
//! stand at: `interpolate` or `fill_null` first). Where time is natural —
//! `decompose`, `holt_winters`, `seasonal_naive` — a `time:` column (Date
//! or Datetime, one per value) makes the answer carry it: the components
//! beside their times, and a forecast as a Frame of the times that follow
//! (the series' own step: a day, an hour, a calendar month).
//!
//! `holt_winters` returns a `HoltWinters` value: its methods are this
//! namespace's (`hw.forecast(14)` is `stats.ts.forecast(hw, 14)`), its
//! fields `fitted`, `resid`, `params`, `sse`, `level`, `slope`, `season`,
//! `n` and `m`.

use super::series::{make_series_value, series_of};
use super::stats::map_value;
use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use olang_ods::ts::{self, HoltWinters, Model, Season};
use olang_ods::{DType, Series};
use std::any::Any;
use std::sync::Arc;

/// (name, fewest arguments).
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("acf", 1),
    ("pacf", 1),
    ("decompose", 2),
    ("holt_winters", 1),
    ("forecast", 2),
    ("seasonal_naive", 3),
];

fn given(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !matches!(v, Value::Unit))
}

fn e(err: olang_ods::OdsError) -> String {
    err.to_string()
}

/// The values of a series, dense.
fn values(func: &str, v: Option<&Value>) -> Result<Vec<f64>, String> {
    let s = v.and_then(series_of).ok_or_else(|| {
        format!(
            "stats.ts.{}: the series must be a Series, got {}",
            func,
            v.map(|v| v.type_name()).unwrap_or_default()
        )
    })?;
    if s.null_count() > 0 {
        return Err(format!(
            "stats.ts.{}: the series holds {} null{} — fill or interpolate it first",
            func,
            s.null_count(),
            if s.null_count() == 1 { "" } else { "s" }
        ));
    }
    match s {
        Series::F64 { values, .. } => Ok(values.as_ref().clone()),
        Series::I64 { values, .. } => Ok(values.iter().map(|&x| x as f64).collect()),
        other => Err(format!(
            "stats.ts.{}: needs a numeric series, got a {} column",
            func,
            other.dtype()
        )),
    }
}

fn int(func: &str, what: &str, v: Option<&Value>) -> Result<Option<usize>, String> {
    match given(v) {
        None => Ok(None),
        Some(Value::Integer(n)) if *n >= 0 => Ok(Some(*n as usize)),
        Some(other) => Err(format!(
            "stats.ts.{}: {} must be a non-negative Int, got {}",
            func, what, other
        )),
    }
}

fn need_int(func: &str, what: &str, v: Option<&Value>) -> Result<usize, String> {
    int(func, what, v)?.ok_or_else(|| format!("stats.ts.{}: missing {}", func, what))
}

fn level(func: &str, v: Option<&Value>) -> Result<f64, String> {
    let l = match given(v) {
        None => 0.95,
        Some(Value::Float(x)) => *x,
        Some(other) => {
            return Err(format!(
                "stats.ts.{}: ci is a level in (0, 1), got {}",
                func, other
            ));
        }
    };
    if !(l > 0.0 && l < 1.0) {
        return Err(format!(
            "stats.ts.{}: ci must lie in (0, 1), got {}",
            func, l
        ));
    }
    Ok(l)
}

fn text<'a>(
    func: &str,
    what: &str,
    v: Option<&'a Value>,
    default: &'a str,
) -> Result<&'a str, String> {
    match given(v) {
        None => Ok(default),
        Some(Value::String(s)) => Ok(s.as_str()),
        Some(other) => Err(format!(
            "stats.ts.{}: {} must be a String, got {}",
            func,
            what,
            other.type_name()
        )),
    }
}

fn rate(func: &str, what: &str, v: Option<&Value>) -> Result<Option<f64>, String> {
    match given(v) {
        None => Ok(None),
        Some(Value::Float(x)) => Ok(Some(*x)),
        Some(Value::Integer(x)) => Ok(Some(*x as f64)),
        Some(other) => Err(format!(
            "stats.ts.{}: {} must be a number in [0, 1], got {}",
            func, what, other
        )),
    }
}

fn floats(v: Vec<f64>) -> Series {
    // NaN (an undefined component) reads as a null.
    if v.iter().any(|x| x.is_nan()) {
        Series::from_f64_options(v.into_iter().map(|x| (!x.is_nan()).then_some(x)).collect())
    } else {
        Series::from_f64(v)
    }
}

fn frame(cols: Vec<(&str, Series)>) -> Result<Value, String> {
    super::frame::from_columns(cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

// ── Time ────────────────────────────────────────────────────────────
/// A Date or Datetime column naming each value's time.
#[derive(Clone, Debug)]
pub struct Times {
    dtype: DType,
    /// Days (a Date) or microseconds (a Datetime).
    values: Vec<i64>,
}

impl Times {
    fn of(func: &str, v: Option<&Value>, n: usize) -> Result<Option<Times>, String> {
        let Some(v) = given(v) else { return Ok(None) };
        let s = series_of(v).ok_or_else(|| {
            format!(
                "stats.ts.{}: time must be a Date or Datetime Series, got {}",
                func,
                v.type_name()
            )
        })?;
        if s.null_count() > 0 {
            return Err(format!("stats.ts.{}: the time column holds a null", func));
        }
        let values: Vec<i64> = match s {
            Series::Date { values, .. } => values.iter().map(|&d| d as i64).collect(),
            Series::Datetime { values, .. } => values.as_ref().clone(),
            other => {
                return Err(format!(
                    "stats.ts.{}: time must be a Date or Datetime column, got {} (to_date / to_datetime reads text)",
                    func,
                    other.dtype()
                ));
            }
        };
        if values.len() != n {
            return Err(format!(
                "stats.ts.{}: {} times for {} values",
                func,
                values.len(),
                n
            ));
        }
        Ok(Some(Times {
            dtype: s.dtype(),
            values,
        }))
    }

    fn series(&self, values: Vec<i64>) -> Series {
        match self.dtype {
            DType::Date => Series::from_dates(values.into_iter().map(|d| d as i32).collect()),
            _ => Series::from_datetimes(values),
        }
    }

    fn column(&self) -> Series {
        self.series(self.values.clone())
    }

    /// The `h` times after the last: by calendar month when the series is
    /// monthly dates, else by the median step.
    fn after(&self, h: usize) -> Vec<i64> {
        let v = &self.values;
        let Some(&last) = v.last() else {
            return Vec::new();
        };
        let mut diffs: Vec<i64> = v.windows(2).map(|w| w[1] - w[0]).collect();
        diffs.sort_unstable();
        let step = diffs.get(diffs.len() / 2).copied().unwrap_or(1).max(1);
        let monthly = self.dtype == DType::Date
            && !diffs.is_empty()
            && diffs.iter().all(|d| (28..=31).contains(d));
        (1..=h as i64)
            .map(|k| {
                if monthly {
                    olang_ods::temporal::add_months(last, k).unwrap_or(last + 30 * k)
                } else {
                    last + k * step
                }
            })
            .collect()
    }
}

// ── The fitted model ────────────────────────────────────────────────
#[derive(Debug)]
pub struct OdsHoltWinters {
    pub model: Arc<HoltWinters>,
    times: Option<Times>,
    y: Arc<Vec<f64>>,
}

impl NativeObject for OdsHoltWinters {
    fn module(&self) -> &'static str {
        "stats.ts"
    }

    fn type_name(&self) -> &'static str {
        "HoltWinters"
    }

    fn display(&self) -> String {
        let m = &self.model;
        let mut parts = vec![format!("α {:.3}", m.alpha)];
        if m.trend {
            parts.push(format!("β {:.3}", m.beta));
        }
        if m.season != Season::None {
            parts.push(format!("γ {:.3}", m.gamma));
        }
        format!(
            "<stats.ts.holt_winters {}{} on {} values: {}>",
            match m.season {
                Season::None => "no season".to_string(),
                Season::Add => format!("additive season {}", m.m),
                Season::Mul => format!("multiplicative season {}", m.m),
            },
            if m.trend { ", additive trend" } else { "" },
            m.n,
            parts.join(", ")
        )
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<OdsHoltWinters>()
            .is_some_and(|o| Arc::ptr_eq(&o.model, &self.model))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn field(&self, name: &str) -> Option<Result<Value, String>> {
        let m = &self.model;
        let opt = |x: f64| {
            if x.is_nan() {
                Value::Unit
            } else {
                Value::Float(x)
            }
        };
        Some(Ok(match name {
            "fitted" => make_series_value(floats(m.fitted.clone())),
            "resid" => make_series_value(floats(
                m.fitted
                    .iter()
                    .zip(self.y.iter())
                    .map(|(f, y)| y - f)
                    .collect(),
            )),
            "params" => map_value(vec![
                ("alpha", Value::Float(m.alpha)),
                ("beta", opt(m.beta)),
                ("gamma", opt(m.gamma)),
            ]),
            "sse" => Value::Float(m.sse),
            "level" => Value::Float(m.level),
            "slope" => Value::Float(m.slope),
            "season" => make_series_value(Series::from_f64(
                // in the order of the times that follow the series
                (0..m.season_state.len())
                    .map(|k| m.season_state[(m.n + k) % m.m])
                    .collect(),
            )),
            "n" => Value::Integer(m.n as i64),
            "m" => Value::Integer(if m.season == Season::None {
                0
            } else {
                m.m as i64
            }),
            _ => return None,
        }))
    }
}

fn model_of(v: Option<&Value>) -> Option<&OdsHoltWinters> {
    match v {
        Some(Value::Native(h)) => h.0.as_any().downcast_ref::<OdsHoltWinters>(),
        _ => None,
    }
}

/// A forecast: a Series, or with times a Frame of `time` and `forecast`.
fn forecast_value(values: Vec<f64>, times: Option<&Times>) -> Result<Value, String> {
    match times {
        None => Ok(make_series_value(Series::from_f64(values))),
        Some(t) => {
            let at = t.series(t.after(values.len()));
            frame(vec![("time", at), ("forecast", Series::from_f64(values))])
        }
    }
}

// ── Dispatch ────────────────────────────────────────────────────────
pub fn dispatch(func: &str, args: &[Value]) -> Result<Value, String> {
    let a = |i: usize| args.get(i);
    match func {
        "acf" | "pacf" => {
            let y = values(func, a(0))?;
            let default_lags =
                ((10.0 * (y.len() as f64).log10()) as usize).min(y.len().saturating_sub(1));
            let lags = int(func, "lags", a(1))?.unwrap_or(default_lags);
            let lv = level(func, a(2))?;
            let par = super::series::par(olang_ods::ParClass::Sort, y.len());
            let c = if func == "acf" {
                ts::acf_par(&y, lags, lv, par).map_err(e)?
            } else {
                let adjusted = match text(func, "method", a(3), "yw")? {
                    "yw" | "ywadjusted" => true,
                    "ywm" | "ywmle" => false,
                    other => {
                        return Err(format!(
                            "stats.ts.pacf: method is \"yw\" (adjusted) or \"ywm\" (maximum likelihood), got \"{}\"",
                            other
                        ));
                    }
                };
                ts::pacf_par(&y, lags, adjusted, lv, par).map_err(e)?
            };
            frame(vec![
                ("lag", Series::from_i64((0..c.acf.len() as i64).collect())),
                (func, Series::from_f64(c.acf)),
                ("lo", Series::from_f64(c.lo)),
                ("hi", Series::from_f64(c.hi)),
            ])
        }
        "decompose" => {
            let y = values(func, a(0))?;
            let period = need_int(func, "period", a(1))?;
            let model = Model::parse(text(func, "model", a(2), "additive")?, "stats.ts.decompose")
                .map_err(e)?;
            let times = Times::of(func, a(3), y.len())?;
            let d = ts::decompose(&y, period, model).map_err(e)?;
            let mut cols = Vec::new();
            if let Some(t) = &times {
                cols.push(("time", t.column()));
            }
            cols.push(("observed", Series::from_f64(y)));
            cols.push(("trend", floats(d.trend)));
            cols.push(("seasonal", Series::from_f64(d.seasonal)));
            cols.push(("resid", floats(d.resid)));
            frame(cols)
        }
        "holt_winters" => {
            let y = values(func, a(0))?;
            let season = int(func, "season", a(1))?;
            let trend = match given(a(2)) {
                None => true,
                Some(Value::String(s)) if s.as_str() == "add" || s.as_str() == "additive" => true,
                Some(Value::String(s)) if s.as_str() == "none" => false,
                Some(other) => {
                    return Err(format!(
                        "stats.ts.holt_winters: trend is \"add\" or \"none\", got {}",
                        other
                    ));
                }
            };
            let seasonal = match (season, text(func, "seasonal", a(3), "add")?) {
                (None, _) | (_, "none") => Season::None,
                (Some(_), "add" | "additive") => Season::Add,
                (Some(_), "mul" | "multiplicative") => Season::Mul,
                (_, other) => {
                    return Err(format!(
                        "stats.ts.holt_winters: seasonal is \"add\", \"mul\" or \"none\", got \"{}\"",
                        other
                    ));
                }
            };
            let fit = match given(a(4)) {
                None => true,
                Some(Value::Boolean(b)) => *b,
                Some(other) => {
                    return Err(format!(
                        "stats.ts.holt_winters: fit must be a Bool, got {}",
                        other.type_name()
                    ));
                }
            };
            let alpha = rate(func, "alpha", a(5))?;
            let beta = rate(func, "beta", a(6))?;
            let gamma = rate(func, "gamma", a(7))?;
            if !fit {
                let missing: Vec<&str> = [
                    ("alpha", alpha.is_none()),
                    ("beta", trend && beta.is_none()),
                    ("gamma", seasonal != Season::None && gamma.is_none()),
                ]
                .into_iter()
                .filter(|(_, m)| *m)
                .map(|(n, _)| n)
                .collect();
                if !missing.is_empty() {
                    return Err(format!(
                        "stats.ts.holt_winters: fit: false needs every rate; missing {}",
                        missing.join(", ")
                    ));
                }
            }
            let times = Times::of(func, a(8), y.len())?;
            let model =
                HoltWinters::fit(&y, season.unwrap_or(1), trend, seasonal, alpha, beta, gamma)
                    .map_err(e)?;
            Ok(Value::Native(NativeHandle::new(OdsHoltWinters {
                model: Arc::new(model),
                times,
                y: Arc::new(y),
            })))
        }
        "forecast" => {
            let m = model_of(a(0)).ok_or_else(|| {
                format!(
                    "stats.ts.forecast: the model must come from stats.ts.holt_winters, got {}",
                    a(0).map(|v| v.type_name()).unwrap_or_default()
                )
            })?;
            let h = need_int(func, "h", a(1))?;
            forecast_value(m.model.forecast(h), m.times.as_ref())
        }
        "seasonal_naive" => {
            let y = values(func, a(0))?;
            let season = need_int(func, "season", a(1))?;
            let h = need_int(func, "h", a(2))?;
            let times = Times::of(func, a(3), y.len())?;
            let f = ts::seasonal_naive(&y, season, h).map_err(e)?;
            forecast_value(f, times.as_ref())
        }
        other => Err(format!("unknown stats function: stats.ts.{}", other)),
    }
}

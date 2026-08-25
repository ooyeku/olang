//! Series: the language-facing face of the olang-ods engine.
//!
//! Everything here is translation — olang `Value`s to engine `Scalar`s and
//! back, argument checking, and parallelism policy (the configured
//! threshold via `crate::parallel::should_parallelize`). No numeric logic
//! lives in this file; that is the engine's job (L0/L2 split in
//! docs/design/ods.md).
//!
//! Operator semantics chosen for Phase 1 (revisit in the language
//! reference when the surface stabilizes):
//! - `+ - * /` on Series⊕Series and Series⊕scalar are elementwise with
//!   broadcast, nulls propagating.
//! - `< <= > >=` produce Bool-series masks; so do `==`/`!=` **against a
//!   scalar** (there is no structural reading of Series == 2.0).
//! - `==`/`!=` **between two Series** stay structural, like every other
//!   olang collection (same dtype, length, null pattern, values); the
//!   elementwise masks are `ods.eq(a, b)` / `ods.ne(a, b)`.

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use olang_ods::{AggOp, ArithOp, CmpOp, DType, RankMethod, Scalar, Series};
use std::any::Any;

#[derive(Debug)]
pub struct OdsSeries {
    /// The materialized series. Empty only while a fused chain is
    /// pending; every access goes through `series()`, which forces.
    cell: std::sync::OnceLock<Series>,
    /// The pending fused chain, when this value was built lazily from
    /// an operator chain (see `binary_op`). `(expr, len, depth)`.
    pending: Option<(olang_ods::fuse::Expr, usize, usize)>,
}

impl OdsSeries {
    fn from_series(series: Series) -> Self {
        let cell = std::sync::OnceLock::new();
        let _ = cell.set(series);
        OdsSeries {
            cell,
            pending: None,
        }
    }

    fn from_expr(expr: olang_ods::fuse::Expr, len: usize, depth: usize) -> Self {
        OdsSeries {
            cell: std::sync::OnceLock::new(),
            pending: Some((expr, len, depth)),
        }
    }

    /// The materialized series, forcing a pending chain on first use.
    /// The fused evaluation is bit-identical to the eager kernels the
    /// chain replaced, so nothing observable depends on when this runs.
    pub fn series(&self) -> &Series {
        self.cell.get_or_init(|| {
            let (expr, len, _) = self
                .pending
                .as_ref()
                .expect("an unmaterialized OdsSeries always carries its chain");
            expr.eval(*len, crate::parallel::should_parallelize(*len))
        })
    }

    pub fn into_value(series: Series) -> Value {
        Value::Native(NativeHandle::new(OdsSeries::from_series(series)))
    }
}

impl NativeObject for OdsSeries {
    fn module(&self) -> &'static str {
        "ods"
    }

    fn type_name(&self) -> &'static str {
        "Series"
    }

    fn display(&self) -> String {
        const SHOWN: usize = 8;
        let n = self.series().len();
        let mut parts = Vec::with_capacity(SHOWN.min(n));
        for i in 0..n.min(SHOWN) {
            parts.push(match self.series().scalar_at(i) {
                Scalar::F64(x) => Value::Float(x).to_string(),
                Scalar::I64(x) => x.to_string(),
                Scalar::Bool(b) => b.to_string(),
                Scalar::Str(s) => format!("\"{}\"", s),
                Scalar::Null => "null".to_string(),
            });
        }
        let ellipsis = if n > SHOWN { ", …" } else { "" };
        format!(
            "Series[{}; {}] [{}{}]",
            self.series().dtype(),
            n,
            parts.join(", "),
            ellipsis
        )
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<OdsSeries>()
            .is_some_and(|o| self.series().series_eq(o.series()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `s[3]` is the element, `s[-1]` counts from the end — the same rule
    /// lists and strings already follow, so a column reads like the list
    /// it stands in for. Nulls come back as unit.
    fn index(&self, key: &Value) -> Option<Result<Value, String>> {
        Some(match key {
            Value::Integer(i) => {
                let len = self.series().len() as i64;
                let pos = if *i < 0 { len + *i } else { *i };
                if pos < 0 || pos >= len {
                    Err(format!(
                        "index {} out of bounds for a Series of length {}",
                        i, len
                    ))
                } else {
                    Ok(scalar_to_value(self.series().scalar_at(pos as usize)))
                }
            }
            Value::String(name) => Err(format!(
                "a Series is indexed by position, not by name. Reach the \
                 column from its Frame first: f[\"{}\"]",
                name
            )),
            other => Err(format!(
                "a Series is indexed by an Int, got {}",
                other.type_name()
            )),
        })
    }
}

// ---------------------------------------------------------------------
// Value ↔ engine translation
// ---------------------------------------------------------------------

pub(super) fn scalar_to_value(s: Scalar) -> Value {
    match s {
        Scalar::F64(x) => Value::Float(x),
        Scalar::I64(x) => Value::Integer(x),
        Scalar::Bool(b) => Value::Boolean(b),
        Scalar::Str(s) => Value::String(std::sync::Arc::new(s)),
        Scalar::Null => Value::Unit,
    }
}

pub(super) fn value_to_scalar(v: &Value) -> Option<Scalar> {
    match v {
        Value::Float(x) => Some(Scalar::F64(*x)),
        Value::Integer(x) => Some(Scalar::I64(*x)),
        Value::Boolean(b) => Some(Scalar::Bool(*b)),
        Value::String(s) => Some(Scalar::Str(s.as_ref().clone())),
        Value::Unit => Some(Scalar::Null),
        _ => None,
    }
}

/// A Bool Series argument — the shape every mask operation needs.
fn want_bool_series<'a>(func: &str, args: &'a [Value], idx: usize) -> Result<&'a Series, String> {
    let series = want_series(func, args, idx)?;
    if series.dtype() != DType::Bool {
        return Err(format!(
            "ods.{}: argument {} must be a Bool mask, got a {} Series. A mask \
             comes from a comparison, like f[\"amount\"] > 100.0",
            func,
            idx + 1,
            series.dtype()
        ));
    }
    Ok(series)
}

/// The list of masks `all_of` / `any_of` combine, checked for shape.
fn mask_list<'a>(func: &str, args: &'a [Value]) -> Result<Vec<&'a Series>, String> {
    let items = match args.first() {
        Some(Value::List(items)) => items,
        other => {
            return Err(format!(
                "ods.{}: expects a list of Bool masks, got {}",
                func,
                other.map(|v| v.type_name()).unwrap_or_default()
            ));
        }
    };
    if items.is_empty() {
        // There is no length to give the answer, so there is no honest
        // result — `all_of([])` cannot be a mask over anything.
        return Err(format!("ods.{}: needs at least one mask", func));
    }
    let mut masks = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        masks.push(
            want_bool_series(func, std::slice::from_ref(item), 0).map_err(|_| {
                format!(
                    "ods.{}: item {} must be a Bool mask, got {}",
                    func,
                    i + 1,
                    item.type_name()
                )
            })?,
        );
    }
    let len = masks[0].len();
    if let Some(bad) = masks.iter().position(|m| m.len() != len) {
        return Err(format!(
            "ods.{}: mask {} has {} rows but the first has {} — every mask \
             must come from the same Frame",
            func,
            bad + 1,
            masks[bad].len(),
            len
        ));
    }
    Ok(masks)
}

/// Three-valued conjunction or disjunction, the same logic SQL uses and
/// the same that `filter` already assumes when it treats a null as false:
/// one `false` settles an `all_of` even if another entry is unknown, and
/// one `true` settles an `any_of`.
fn combine(all: bool, masks: &[&Series]) -> Result<Series, String> {
    let len = masks[0].len();
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let mut unknown = false;
        let mut settled = false;
        for mask in masks {
            match mask.scalar_at(i) {
                Scalar::Bool(b) if b != all => {
                    settled = true;
                    break;
                }
                Scalar::Bool(_) => {}
                _ => unknown = true,
            }
        }
        out.push(if settled {
            Some(!all)
        } else if unknown {
            None
        } else {
            Some(all)
        });
    }
    Ok(Series::from_bool_options(out))
}

pub fn series_of(value: &Value) -> Option<&Series> {
    match value {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsSeries>().map(|s| s.series()),
        _ => None,
    }
}

/// Build a series from an olang list. Dtype inference: all Int → Int;
/// any Float among numerics → Float (ints widen); all Bool → Bool;
/// `()` is null and defers to the rest; an all-null or empty list is a
/// Float series (the least surprising default for numeric work).
pub(super) fn series_from_list(items: &[Value]) -> Result<Series, String> {
    let mut saw_float = false;
    let mut saw_int = false;
    let mut saw_bool = false;
    let mut saw_str = false;
    for item in items {
        match item {
            Value::Float(_) => saw_float = true,
            Value::Integer(_) => saw_int = true,
            Value::Boolean(_) => saw_bool = true,
            Value::String(_) => saw_str = true,
            Value::Unit => {}
            other => {
                return Err(format!(
                    "ods.series: list elements must be Int, Float, Bool, String, or () for null — got {}",
                    other.type_name()
                ));
            }
        }
    }
    if saw_str && (saw_float || saw_int || saw_bool) {
        return Err("ods.series: cannot mix String with other element types".to_string());
    }
    if saw_str {
        let opts = items
            .iter()
            .map(|v| match v {
                Value::String(s) => Some(s.as_ref().clone()),
                _ => None,
            })
            .collect();
        return Ok(Series::from_str_options(opts));
    }
    if saw_bool && (saw_float || saw_int) {
        return Err("ods.series: cannot mix Bool with numeric elements".to_string());
    }
    if saw_bool {
        let opts = items
            .iter()
            .map(|v| match v {
                Value::Boolean(b) => Some(*b),
                _ => None,
            })
            .collect();
        return Ok(Series::from_bool_options(opts));
    }
    if saw_int && !saw_float {
        let opts = items
            .iter()
            .map(|v| match v {
                Value::Integer(x) => Some(*x),
                _ => None,
            })
            .collect();
        return Ok(Series::from_i64_options(opts));
    }
    let opts = items
        .iter()
        .map(|v| match v {
            Value::Float(x) => Some(*x),
            Value::Integer(x) => Some(*x as f64),
            _ => None,
        })
        .collect();
    Ok(Series::from_f64_options(opts))
}

fn par_for(s: &Series) -> bool {
    crate::parallel::should_parallelize(s.len())
}

// ---------------------------------------------------------------------
// Namespace and dispatch
// ---------------------------------------------------------------------

/// (name, arity) of every series function in the `ods` namespace.
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("series", 1),
    ("zeros", 1),
    ("linspace", 3),
    ("to_list", 1),
    ("len", 1),
    ("get", 2),
    ("null_count", 1),
    ("is_null", 1),
    ("fill_null", 2),
    ("sum", 1),
    ("mean", 1),
    ("var", 1),
    ("std", 1),
    ("min", 1),
    ("max", 1),
    ("quantile", 2),
    ("median", 1),
    ("unique", 1),
    ("n_unique", 1),
    ("value_counts", 1),
    ("cast", 2),
    ("sample", 2),
    ("shift", 2),
    ("cum_max", 1),
    ("cum_min", 1),
    ("rank", 2),
    ("rolling", 3),
    ("cumsum", 1),
    ("map", 2),
    ("dot", 2),
    ("sort", 1),
    ("argsort", 1),
    ("take", 2),
    ("filter", 2),
    ("eq", 2),
    ("all_of", 1),
    ("any_of", 1),
    ("not", 1),
    ("ne", 2),
];

/// The ranking methods, in the order the error lists them.
const RANK_METHODS: &[&str] = &["min", "max", "average", "ordinal", "dense"];

/// The type names `cast` accepts, which are the names `schema` reports.
const DTYPE_NAMES: &[&str] = &["Float", "Int", "Bool", "String"];

fn parse_dtype(name: &str) -> Result<DType, String> {
    match name {
        "Float" => Ok(DType::F64),
        "Int" => Ok(DType::I64),
        "Bool" => Ok(DType::Bool),
        "String" => Ok(DType::Str),
        other => Err(format!(
            "ods.cast: '{}' is not a column type. Expected one of {}",
            other,
            DTYPE_NAMES.join(", ")
        )),
    }
}

fn want_series<'a>(func: &str, args: &'a [Value], idx: usize) -> Result<&'a Series, String> {
    args.get(idx).and_then(series_of).ok_or_else(|| {
        format!(
            "ods.{}: argument {} must be a Series, got {}",
            func,
            idx + 1,
            args.get(idx).map(|v| v.type_name()).unwrap_or_default()
        )
    })
}

fn arity(func: &str, args: &[Value], expected: usize) -> Result<(), String> {
    if args.len() != expected {
        return Err(format!(
            "ods.{} expects {} argument{}, got {}",
            func,
            expected,
            if expected == 1 { "" } else { "s" },
            args.len()
        ));
    }
    Ok(())
}

pub fn dispatch(func: &str, args: Vec<Value>) -> Option<Result<Value, String>> {
    let expected = FUNCTIONS.iter().find(|(n, _)| *n == func)?.1;
    Some(dispatch_inner(func, args, expected))
}

fn dispatch_inner(func: &str, mut args: Vec<Value>, expected: usize) -> Result<Value, String> {
    // "rank" unqualified means the competition ranking (1, 2, 2, 4) —
    // the one a reader assumes when no method is named. Stated here
    // rather than inside the arm, so the arity check and the default
    // cannot drift apart.
    if func == "rank" && args.len() + 1 == expected {
        args.push(Value::String(std::sync::Arc::new("min".to_string())));
    }
    arity(func, &args, expected)?;
    let e = |err: olang_ods::OdsError| err.to_string();
    match func {
        "series" => match &args[0] {
            Value::List(items) => Ok(OdsSeries::into_value(series_from_list(items)?)),
            Value::Range {
                start,
                end,
                inclusive,
            } => Ok(OdsSeries::into_value(Series::from_range(
                *start, *end, *inclusive,
            ))),
            other => Err(format!(
                "ods.series expects a list or range, got {}",
                other.type_name()
            )),
        },
        "zeros" => match &args[0] {
            Value::Integer(n) if *n >= 0 => Ok(OdsSeries::into_value(Series::zeros(*n as usize))),
            other => Err(format!(
                "ods.zeros expects a non-negative Int, got {}",
                other
            )),
        },
        "linspace" => {
            let as_f64 = |v: &Value, name: &str| match v {
                Value::Float(x) => Ok(*x),
                Value::Integer(x) => Ok(*x as f64),
                other => Err(format!(
                    "ods.linspace: {} must be numeric, got {}",
                    name,
                    other.type_name()
                )),
            };
            let start = as_f64(&args[0], "start")?;
            let stop = as_f64(&args[1], "stop")?;
            let num = match &args[2] {
                Value::Integer(n) if *n >= 0 => *n as usize,
                other => {
                    return Err(format!(
                        "ods.linspace: num must be a non-negative Int, got {}",
                        other
                    ));
                }
            };
            Series::linspace(start, stop, num)
                .map(OdsSeries::into_value)
                .map_err(e)
        }
        "to_list" => {
            let s = want_series(func, &args, 0)?;
            let items: Vec<Value> = (0..s.len())
                .map(|i| scalar_to_value(s.scalar_at(i)))
                .collect();
            Ok(Value::List(items.into()))
        }
        "len" => Ok(Value::Integer(want_series(func, &args, 0)?.len() as i64)),
        "get" => {
            let s = want_series(func, &args, 0)?;
            match &args[1] {
                Value::Integer(i) => s.get(*i).map(scalar_to_value).map_err(e),
                other => Err(format!(
                    "ods.get: index must be an Int, got {}",
                    other.type_name()
                )),
            }
        }
        "null_count" => Ok(Value::Integer(
            want_series(func, &args, 0)?.null_count() as i64
        )),
        "is_null" => Ok(OdsSeries::into_value(
            want_series(func, &args, 0)?.is_null(),
        )),
        "fill_null" => {
            let s = want_series(func, &args, 0)?.clone();
            let fill = value_to_scalar(&args[1]).ok_or_else(|| {
                format!(
                    "ods.fill_null: fill value must be Int, Float, or Bool, got {}",
                    args[1].type_name()
                )
            })?;
            s.fill_null(fill).map(OdsSeries::into_value).map_err(e)
        }
        "sum" => {
            let s = want_series(func, &args, 0)?;
            s.sum(par_for(s)).map(scalar_to_value).map_err(e)
        }
        "mean" | "var" | "std" => {
            let s = want_series(func, &args, 0)?;
            let par = par_for(s);
            let out = match func {
                "mean" => s.mean(par),
                "var" => s.var(par),
                _ => s.std(par),
            };
            out.map(|o| o.map(Value::Float).unwrap_or(Value::Unit))
                .map_err(e)
        }
        "min" => want_series(func, &args, 0)?
            .min()
            .map(scalar_to_value)
            .map_err(e),
        "max" => want_series(func, &args, 0)?
            .max()
            .map(scalar_to_value)
            .map_err(e),
        // Defined as quantile(0.5) rather than reimplemented, so the two
        // cannot disagree — and so `describe`, whose "median" row is the
        // same call, reports the same number this does.
        "median" => want_series(func, &args, 0)?
            .quantile(0.5)
            .map(|v| v.map(Value::Float).unwrap_or(Value::Unit))
            .map_err(e),
        "unique" => want_series(func, &args, 0)?
            .unique()
            .map(make_series_value)
            .map_err(e),
        "n_unique" => Ok(Value::Integer(
            want_series(func, &args, 0)?.n_unique() as i64
        )),
        "value_counts" => {
            let (values, counts) = want_series(func, &args, 0)?.value_counts().map_err(e)?;
            crate::ods::frame::from_columns(vec![
                ("value".to_string(), values),
                ("count".to_string(), counts),
            ])
        }
        "cast" => {
            let s = want_series(func, &args, 0)?;
            let to = match &args[1] {
                Value::String(name) => parse_dtype(name)?,
                other => {
                    return Err(format!(
                        "ods.cast: the target type must be a String — one of {}. Got {}",
                        DTYPE_NAMES.join(", "),
                        other.type_name()
                    ));
                }
            };
            s.cast(to, &crate::ast::format_float)
                .map(make_series_value)
                .map_err(e)
        }
        "sample" => {
            let s = want_series(func, &args, 0)?;
            let idx = crate::ods::frame::sample_indices(func, s.len(), args.get(1))?;
            s.take(&idx).map(make_series_value).map_err(e)
        }
        // ── windows ──
        "shift" => {
            let s = want_series(func, &args, 0)?;
            match &args[1] {
                Value::Integer(by) => s.shift(*by).map(make_series_value).map_err(e),
                other => Err(format!(
                    "ods.shift: the offset must be an Int, got {}",
                    other.type_name()
                )),
            }
        }
        "cum_max" | "cum_min" => want_series(func, &args, 0)?
            .cum_extreme(func == "cum_max")
            .map(make_series_value)
            .map_err(e),
        "rank" => {
            let s = want_series(func, &args, 0)?;
            let name = match &args[1] {
                Value::String(name) => name.as_ref().clone(),
                other => {
                    return Err(format!(
                        "ods.rank: the method must be a String — one of {}. Got {}",
                        RANK_METHODS.join(", "),
                        other.type_name()
                    ));
                }
            };
            let method = RankMethod::parse(&name).ok_or_else(|| {
                format!(
                    "ods.rank: '{}' is not a ranking method. Expected one of {}",
                    name,
                    RANK_METHODS.join(", ")
                )
            })?;
            s.rank(method).map(make_series_value).map_err(e)
        }
        "rolling" => {
            let s = want_series(func, &args, 0)?;
            let window = match &args[1] {
                Value::Integer(w) if *w >= 1 => *w as usize,
                Value::Integer(w) => {
                    return Err(format!(
                        "ods.rolling: the window must be at least 1, got {}",
                        w
                    ));
                }
                other => {
                    return Err(format!(
                        "ods.rolling: the window must be an Int, got {}",
                        other.type_name()
                    ));
                }
            };
            let op_name = match &args[2] {
                Value::String(name) => name.as_ref().clone(),
                other => {
                    return Err(format!(
                        "ods.rolling: the aggregation must be a String, got {}",
                        other.type_name()
                    ));
                }
            };
            let op = AggOp::parse(&op_name).ok_or_else(|| {
                format!(
                    "ods.rolling: '{}' is not an aggregation. Expected one of \
                     count, sum, mean, min, max",
                    op_name
                )
            })?;
            s.rolling(window, op).map(make_series_value).map_err(e)
        }
        "quantile" => {
            let s = want_series(func, &args, 0)?;
            let q = match &args[1] {
                Value::Float(q) => *q,
                Value::Integer(q) => *q as f64,
                other => {
                    return Err(format!(
                        "ods.quantile: q must be numeric, got {}",
                        other.type_name()
                    ));
                }
            };
            s.quantile(q)
                .map(|o| o.map(Value::Float).unwrap_or(Value::Unit))
                .map_err(e)
        }
        "cumsum" => want_series(func, &args, 0)?
            .cumsum()
            .map(OdsSeries::into_value)
            .map_err(e),
        "map" => {
            // ods.map(series, "sin") — a whole elementwise math transform
            // in the kernel, no per-element lambda crossing. The name is
            // a String (not a closure) precisely so the loop can stay
            // native; compose with Series arithmetic for full vectorized
            // expressions.
            let ser = want_series(func, &args, 0)?;
            let name = match args.get(1) {
                Some(Value::String(s)) => s.as_ref().clone(),
                other => {
                    return Err(format!(
                        "ods.map: argument 2 must be a function name String like \"sin\", got {}",
                        other.map(|v| v.type_name()).unwrap_or_default()
                    ));
                }
            };
            ser.map_unary(&name).map(OdsSeries::into_value).map_err(e)
        }
        "dot" => {
            let a = want_series(func, &args, 0)?;
            let b = want_series(func, &args, 1)?;
            a.dot(b, par_for(a)).map(Value::Float).map_err(e)
        }
        "sort" => {
            let s = want_series(func, &args, 0)?;
            s.sort(par_for(s)).map(OdsSeries::into_value).map_err(e)
        }
        "argsort" => want_series(func, &args, 0)?
            .argsort()
            .map(OdsSeries::into_value)
            .map_err(e),
        "take" => {
            let s = want_series(func, &args, 0)?;
            let idx = want_series(func, &args, 1)?;
            s.take(idx).map(OdsSeries::into_value).map_err(e)
        }
        "filter" => {
            let s = want_series(func, &args, 0)?;
            let mask = want_series(func, &args, 1)?;
            s.filter(mask).map(OdsSeries::into_value).map_err(e)
        }
        "eq" | "ne" => {
            let a = want_series(func, &args, 0)?;
            let op = if func == "eq" { CmpOp::Eq } else { CmpOp::Ne };
            // The `==` operator already accepts a scalar on the right, so
            // refusing one here was an inconsistency that cost a
            // confusing error on the most common filter there is.
            match args.get(1) {
                Some(other) if series_of(other).is_some() => a
                    .compare(op, series_of(other).expect("checked"))
                    .map(OdsSeries::into_value)
                    .map_err(e),
                Some(other) => match value_to_scalar(other) {
                    Some(scalar) => a
                        .compare_scalar(op, scalar, false)
                        .map(OdsSeries::into_value)
                        .map_err(e),
                    None => Err(format!(
                        "ods.{}: argument 2 must be a Series or a scalar, got {}",
                        func,
                        other.type_name()
                    )),
                },
                None => Err(format!("ods.{}: expects 2 arguments", func)),
            }
        }
        // Mask combination. Filtering on more than one condition is the
        // normal case, and `&&` cannot serve: the language compiles it to
        // conditional jumps for short-circuiting, which has no elementwise
        // reading. These take a list rather than two arguments because a
        // real filter usually has three or four conditions, not two.
        "all_of" | "any_of" => {
            let masks = mask_list(func, &args)?;
            Ok(OdsSeries::into_value(combine(func == "all_of", &masks)?))
        }
        "not" => {
            let mask = want_bool_series(func, &args, 0)?;
            let flipped: Vec<Option<bool>> = (0..mask.len())
                .map(|i| match mask.scalar_at(i) {
                    Scalar::Bool(b) => Some(!b),
                    // Unknown stays unknown, as everywhere else nulls
                    // meet arithmetic.
                    _ => None,
                })
                .collect();
            Ok(OdsSeries::into_value(Series::from_bool_options(flipped)))
        }
        _ => unreachable!("dispatch() checked membership"),
    }
}

// ---------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------

/// One side of a fusable operator chain: its expression, its length
/// (None for a broadcast scalar), and its tree depth.
fn fuse_side(v: &Value) -> Option<(olang_ods::fuse::Expr, Option<usize>, usize)> {
    use olang_ods::fuse::Expr;
    if let Value::Float(x) = v {
        return Some((Expr::Const(*x), None, 0));
    }
    let Value::Native(h) = v else { return None };
    let s = h.0.as_any().downcast_ref::<OdsSeries>()?;
    // A still-pending chain extends without materializing; a forced or
    // eagerly built series joins as a leaf when it is exactly the shape
    // the fused evaluator is proven identical on: null-free F64.
    if let Some(m) = s.cell.get() {
        return match m {
            Series::F64 {
                values,
                validity: None,
            } => Some((Expr::Leaf(values.clone()), Some(values.len()), 0)),
            _ => None,
        };
    }
    let (expr, len, depth) = s.pending.as_ref()?;
    Some((expr.clone(), Some(*len), *depth))
}

/// Chains deeper than this materialize instead of growing — a bound on
/// the fused evaluator's recursion and per-level chunk buffers.
const MAX_FUSED_DEPTH: usize = 16;

/// Build a lazy fused series for `lhs op rhs`, when both sides fit the
/// proven-identical shape (see `olang_ods::fuse`). None falls back to
/// the eager kernels; `Div` never fuses because it pre-checks divisors
/// and must error at its own expression, not at force time.
fn try_fuse(op: ArithOp, lhs: &Value, rhs: &Value) -> Option<Value> {
    use olang_ods::fuse::Expr;
    if matches!(op, ArithOp::Div) {
        return None;
    }
    let (a, alen, adepth) = fuse_side(lhs)?;
    let (b, blen, bdepth) = fuse_side(rhs)?;
    let len = match (alen, blen) {
        (Some(x), Some(y)) if x == y => x,
        (Some(x), None) | (None, Some(x)) => x,
        // Two scalars never reach binary_op; a length mismatch takes
        // the eager path, which raises the engine's own error.
        _ => return None,
    };
    let depth = adepth.max(bdepth) + 1;
    if depth > MAX_FUSED_DEPTH {
        return None;
    }
    let expr = Expr::Bin(op, std::sync::Arc::new(a), std::sync::Arc::new(b));
    Some(Value::Native(NativeHandle::new(OdsSeries::from_expr(
        expr, len, depth,
    ))))
}

pub fn binary_op(
    op: &crate::ast::BinaryOp,
    lhs: &Value,
    rhs: &Value,
) -> Option<Result<Value, String>> {
    use crate::ast::BinaryOp as B;
    // Elementwise chains fuse lazily: `a * b + 1.0` materializes once,
    // in one pass, instead of once per operator.
    if let Some(fop) = match op {
        B::Add => Some(ArithOp::Add),
        B::Subtract => Some(ArithOp::Sub),
        B::Multiply => Some(ArithOp::Mul),
        _ => None,
    } && let Some(v) = try_fuse(fop, lhs, rhs)
    {
        return Some(Ok(v));
    }
    let arith = |o: ArithOp| -> Option<Result<Value, String>> {
        match (series_of(lhs), series_of(rhs)) {
            (Some(a), Some(b)) => Some(
                a.arith(o, b, par_for(a))
                    .map(OdsSeries::into_value)
                    .map_err(|e| e.to_string()),
            ),
            (Some(a), None) => {
                let s = value_to_scalar(rhs)?;
                Some(
                    a.arith_scalar(o, s, false, par_for(a))
                        .map(OdsSeries::into_value)
                        .map_err(|e| e.to_string()),
                )
            }
            (None, Some(b)) => {
                let s = value_to_scalar(lhs)?;
                Some(
                    b.arith_scalar(o, s, true, par_for(b))
                        .map(OdsSeries::into_value)
                        .map_err(|e| e.to_string()),
                )
            }
            (None, None) => None,
        }
    };
    let mask = |o: CmpOp| -> Option<Result<Value, String>> {
        match (series_of(lhs), series_of(rhs)) {
            (Some(a), Some(b)) => Some(
                a.compare(o, b)
                    .map(OdsSeries::into_value)
                    .map_err(|e| e.to_string()),
            ),
            (Some(a), None) => {
                let s = value_to_scalar(rhs)?;
                Some(
                    a.compare_scalar(o, s, false)
                        .map(OdsSeries::into_value)
                        .map_err(|e| e.to_string()),
                )
            }
            (None, Some(b)) => {
                let s = value_to_scalar(lhs)?;
                Some(
                    b.compare_scalar(o, s, true)
                        .map(OdsSeries::into_value)
                        .map_err(|e| e.to_string()),
                )
            }
            (None, None) => None,
        }
    };
    match op {
        B::Add => arith(ArithOp::Add),
        B::Subtract => arith(ArithOp::Sub),
        B::Multiply => arith(ArithOp::Mul),
        B::Divide => arith(ArithOp::Div),
        B::LessThan => mask(CmpOp::Lt),
        B::LessThanEqual => mask(CmpOp::Le),
        B::GreaterThan => mask(CmpOp::Gt),
        B::GreaterThanEqual => mask(CmpOp::Ge),
        // Series == Series stays structural (the registry's native_eq
        // fallback); against a scalar there is no structural reading, so
        // ==/!= produce masks like the ordered comparisons.
        B::Equal | B::NotEqual => {
            let both_series = series_of(lhs).is_some() && series_of(rhs).is_some();
            if both_series {
                return None;
            }
            let o = if matches!(op, B::Equal) {
                CmpOp::Eq
            } else {
                CmpOp::Ne
            };
            mask(o)
        }
        _ => None,
    }
}

/// Convenience used by tests: wrap a series value.
pub fn make_series_value(series: Series) -> Value {
    OdsSeries::into_value(series)
}

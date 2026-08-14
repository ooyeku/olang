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
use olang_ods::{ArithOp, CmpOp, Scalar, Series};
use std::any::Any;

#[derive(Debug)]
pub struct OdsSeries(pub Series);

impl OdsSeries {
    pub fn into_value(series: Series) -> Value {
        Value::Native(NativeHandle::new(OdsSeries(series)))
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
        let n = self.0.len();
        let mut parts = Vec::with_capacity(SHOWN.min(n));
        for i in 0..n.min(SHOWN) {
            parts.push(match self.0.scalar_at(i) {
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
            self.0.dtype(),
            n,
            parts.join(", "),
            ellipsis
        )
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<OdsSeries>()
            .is_some_and(|o| self.0.series_eq(&o.0))
    }

    fn as_any(&self) -> &dyn Any {
        self
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

pub fn series_of(value: &Value) -> Option<&Series> {
    match value {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsSeries>().map(|s| &s.0),
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
    ("cumsum", 1),
    ("map", 2),
    ("dot", 2),
    ("sort", 1),
    ("argsort", 1),
    ("take", 2),
    ("filter", 2),
    ("eq", 2),
    ("ne", 2),
];

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

fn dispatch_inner(func: &str, args: Vec<Value>, expected: usize) -> Result<Value, String> {
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
            let b = want_series(func, &args, 1)?;
            let op = if func == "eq" { CmpOp::Eq } else { CmpOp::Ne };
            a.compare(op, b).map(OdsSeries::into_value).map_err(e)
        }
        _ => unreachable!("dispatch() checked membership"),
    }
}

// ---------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------

pub fn binary_op(
    op: &crate::ast::BinaryOp,
    lhs: &Value,
    rhs: &Value,
) -> Option<Result<Value, String>> {
    use crate::ast::BinaryOp as B;
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

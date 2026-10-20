//! The column functions of the data-ergonomics round: math, nulls,
//! conditionals, constructors, positions and differences. Elementwise
//! ones build expressions (`expr::build`), so they fuse with operators
//! and with each other, take an Expr as readily as a Series, and raise
//! only where an operation can; the rest are eager kernels over a
//! Series, and become call nodes when given an Expr.

use super::expr::{self, Operand, build, call_node, holds_expr, operand};
use super::series::{OdsSeries, scalar_to_value, series_of, value_to_scalar};
use crate::ast::Value;
use olang_ods::column::{self, Unmapped};
use olang_ods::fuse::{BinOp, Closed, Expr, Math, Op, Reduce};
use olang_ods::{DType, Scalar, Series};
use std::sync::Arc;

/// The math functions `ods` has by name (each also an `ods.map` name
/// where `map` had it).
pub const MATH: &[&str] = &[
    "abs", "exp", "exp2", "expm1", "ln", "ln1p", "log2", "log10", "sqrt", "cbrt", "sin", "cos",
    "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "sigmoid", "logit", "sign", "floor",
    "ceil", "trunc",
];

/// (name, arity) of this file's functions; the arity is the most a
/// call takes (the fewer forms are in `MIN_ARGS`).
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("abs", 1),
    ("exp", 1),
    ("exp2", 1),
    ("expm1", 1),
    ("ln", 1),
    ("ln1p", 1),
    ("log2", 1),
    ("log10", 1),
    ("sqrt", 1),
    ("cbrt", 1),
    ("sin", 1),
    ("cos", 1),
    ("tan", 1),
    ("asin", 1),
    ("acos", 1),
    ("atan", 1),
    ("sinh", 1),
    ("cosh", 1),
    ("tanh", 1),
    ("sigmoid", 1),
    ("logit", 1),
    ("sign", 1),
    ("floor", 1),
    ("ceil", 1),
    ("trunc", 1),
    ("pow", 2),
    ("round", 2),
    ("clip", 3),
    ("where", 3),
    ("coalesce", 2),
    ("between", 4),
    ("not_null", 1),
    ("is_nan", 1),
    ("interpolate", 2),
    ("recode", 3),
    ("full", 2),
    ("ones", 1),
    ("arange", 3),
    ("full_like", 2),
    ("repeat", 2),
    ("tile", 2),
    ("argmax", 1),
    ("argmin", 1),
    ("diff", 2),
    ("pct_change", 2),
    ("cumprod", 1),
    ("mode", 1),
    ("any", 1),
    ("all", 1),
    ("searchsorted", 3),
    ("count", 1),
    ("first", 1),
    ("last", 1),
    ("col", 1),
    ("lit", 1),
    ("with", 2),
    ("over", 2),
    ("alias", 2),
];

fn want_n(func: &str, args: &[Value], lo: usize, hi: usize) -> Result<(), String> {
    if args.len() < lo || args.len() > hi {
        let want = if lo == hi {
            format!("{}", lo)
        } else if hi == usize::MAX {
            format!("at least {}", lo)
        } else {
            format!("{} to {}", lo, hi)
        };
        return Err(format!(
            "ods.{} expects {} argument{}, got {}",
            func,
            want,
            if hi == 1 { "" } else { "s" },
            args.len()
        ));
    }
    Ok(())
}

fn opnd(func: &str, args: &[Value], i: usize) -> Result<Operand, String> {
    operand(&args[i]).ok_or_else(|| {
        format!(
            "ods.{}: argument {} must be a Series, an Expr or a value, got {}",
            func,
            i + 1,
            args[i].type_name()
        )
    })
}

fn null() -> Operand {
    Operand::Eng(Expr::lit(Scalar::Null), None, 0, vec![])
}

fn lit(s: Scalar) -> Operand {
    Operand::Eng(Expr::lit(s), None, 0, vec![])
}

fn want_int(func: &str, what: &str, v: &Value) -> Result<i64, String> {
    match v {
        Value::Integer(n) => Ok(*n),
        other => Err(format!(
            "ods.{}: {} must be an Int, got {}",
            func,
            what,
            other.type_name()
        )),
    }
}

fn want_count(func: &str, what: &str, v: &Value) -> Result<usize, String> {
    let n = want_int(func, what, v)?;
    usize::try_from(n).map_err(|_| format!("ods.{}: {} cannot be negative, got {}", func, what, n))
}

fn want_series<'a>(func: &str, args: &'a [Value], i: usize) -> Result<&'a Series, String> {
    args.get(i).and_then(series_of).ok_or_else(|| {
        format!(
            "ods.{}: argument {} must be a Series, got {}",
            func,
            i + 1,
            args.get(i).map(|v| v.type_name()).unwrap_or_default()
        )
    })
}

fn sv(s: Series) -> Value {
    OdsSeries::into_value(s)
}

fn e(err: olang_ods::OdsError) -> String {
    err.to_string()
}

/// A `fill_null` method name.
const METHODS: &[&str] = &["ffill", "bfill", "linear", "mean", "median"];

fn is_method(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::String(s)) if METHODS.contains(&s.as_str()))
}

/// The elementwise functions: an Expr, a lazy Series, or a scalar.
/// None when `func` (with these arguments) is not one.
pub fn elementwise(func: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let r = (|| -> Result<Value, String> {
        if let Some(m) = Math::parse(func).filter(|_| MATH.contains(&func)) {
            want_n(func, args, 1, 1)?;
            return build(func, Op::Math(m), vec![opnd(func, args, 0)?]);
        }
        match func {
            // `ods.map(s, "exp")`: the same kernel as `ods.exp(s)`.
            "map" if args.len() == 2
                && matches!(&args[1], Value::String(n) if Math::parse(n).is_some()) =>
            {
                let Value::String(name) = &args[1] else {
                    unreachable!()
                };
                let m = Math::parse(name).expect("checked");
                // `map` always answered a Float column, Int widened.
                if let Some(t) = series_of_unforced(&args[0])
                    && !matches!(t, DType::F64 | DType::I64)
                {
                    return Err(format!("ods.map: a {} series is not numeric", t));
                }
                let wide = build(func, Op::Cast(DType::F64), vec![opnd(func, args, 0)?])?;
                build(func, Op::Math(m), vec![operand(&wide).expect("a built value")])
            }
            "pow" => {
                want_n(func, args, 2, 2)?;
                build(func, Op::Bin(BinOp::Pow), vec![opnd(func, args, 0)?, opnd(func, args, 1)?])
            }
            "round" => {
                want_n(func, args, 1, 2)?;
                let d = match args.get(1) {
                    None => 0,
                    Some(v) => want_int(func, "the number of digits", v)?,
                };
                let d = i32::try_from(d).map_err(|_| format!("ods.round: {} digits is too many", d))?;
                build(func, Op::Round(d), vec![opnd(func, args, 0)?])
            }
            "clip" => {
                want_n(func, args, 2, 3)?;
                let hi = if args.len() == 3 { opnd(func, args, 2)? } else { null() };
                build(func, Op::Clip, vec![opnd(func, args, 0)?, opnd(func, args, 1)?, hi])
            }
            "where" => {
                want_n(func, args, 3, 3)?;
                build(
                    func,
                    Op::Where,
                    vec![opnd(func, args, 0)?, opnd(func, args, 1)?, opnd(func, args, 2)?],
                )
            }
            "coalesce" => {
                want_n(func, args, 1, usize::MAX)?;
                let ops = (0..args.len())
                    .map(|i| opnd(func, args, i))
                    .collect::<Result<Vec<_>, _>>()?;
                if ops.len() == 1 {
                    return Ok(args[0].clone());
                }
                build(func, Op::Coalesce, ops)
            }
            "fill_null" => {
                want_n(func, args, 2, 2)?;
                build(func, Op::Coalesce, vec![opnd(func, args, 0)?, opnd(func, args, 1)?])
            }
            "between" => {
                want_n(func, args, 2, 4)?;
                // `s.between([lo, hi], closed: "left")` leaves `hi` a gap.
                let ranged = args.len() == 2
                    || (matches!(args.get(2), Some(Value::Unit))
                        && matches!(&args[1], Value::Unit | Value::List(_) | Value::Tuple(_)));
                let (lo, hi, closed_at) = match (if ranged { 2 } else { args.len() }, &args[1]) {
                    // A range as one value: a [lo, hi] pair (`ui.brush`'s),
                    // or () for no range at all.
                    (2, Value::Unit) => (null(), null(), args.get(3)),
                    (2, Value::List(p)) | (2, Value::Tuple(p)) if p.len() == 2 => {
                        let side = |v: &Value| {
                            operand(v).ok_or_else(|| {
                                format!("ods.between: a bound is a number, got {}", v.type_name())
                            })
                        };
                        (side(&p[0])?, side(&p[1])?, args.get(3))
                    }
                    (2, other) => {
                        return Err(format!(
                            "ods.between: give the bounds as lo, hi or as one [lo, hi] range, got {}",
                            other.type_name()
                        ));
                    }
                    _ => (opnd(func, args, 1)?, opnd(func, args, 2)?, args.get(3)),
                };
                let closed = match closed_at {
                    None => Closed::Both,
                    Some(Value::String(c)) => Closed::parse(c).ok_or_else(|| {
                        format!(
                            "ods.between: closed is \"both\", \"left\", \"right\" or \"neither\", got \"{}\"",
                            c
                        )
                    })?,
                    Some(other) => {
                        return Err(format!(
                            "ods.between: closed is a String, got {}",
                            other.type_name()
                        ));
                    }
                };
                build(func, Op::Between(closed), vec![opnd(func, args, 0)?, lo, hi])
            }
            "is_null" | "not_null" | "is_nan" => {
                want_n(func, args, 1, 1)?;
                let op = match func {
                    "is_null" => Op::IsNull,
                    "not_null" => Op::NotNull,
                    _ => Op::IsNan,
                };
                build(func, op, vec![opnd(func, args, 0)?])
            }
            "not" => {
                want_n(func, args, 1, 1)?;
                if let Some(s) = series_of_unforced(&args[0])
                    && s != DType::Bool
                {
                    return Err(format!(
                        "ods.not: argument 1 must be a Bool mask, got a {} Series. A mask \
                         comes from a comparison, like f[\"amount\"] > 100.0",
                        s
                    ));
                }
                build(func, Op::Not, vec![opnd(func, args, 0)?])
            }
            "eq" | "ne" => {
                want_n(func, args, 2, 2)?;
                let op = if func == "eq" { BinOp::Eq } else { BinOp::Ne };
                build(func, Op::Bin(op), vec![opnd(func, args, 0)?, opnd(func, args, 1)?])
            }
            "select" => {
                // np.select: the first mask that holds picks its value.
                want_n(func, args, 2, 3)?;
                let (Value::List(masks), Value::List(values)) = (&args[0], &args[1]) else {
                    return Err(
                        "ods.select: expects a list of masks, a list of values and a default"
                            .to_string(),
                    );
                };
                if masks.len() != values.len() {
                    return Err(format!(
                        "ods.select: {} masks but {} values",
                        masks.len(),
                        values.len()
                    ));
                }
                let mut acc = match args.get(2) {
                    Some(_) => opnd(func, args, 2)?,
                    None => null(),
                };
                for (m, v) in masks.iter().zip(values.iter()).rev() {
                    let m = operand(m).ok_or("ods.select: each mask is a Bool Series or Expr")?;
                    let v = operand(v).ok_or_else(|| {
                        format!("ods.select: a value must be a Series, an Expr or a value, got {}", v.type_name())
                    })?;
                    // A null mask does not hold.
                    let held = build(func, Op::Coalesce, vec![m, lit(Scalar::Bool(false))])?;
                    let held = operand(&held).expect("a built value is an operand");
                    let next = build(func, Op::Where, vec![held, v, acc])?;
                    acc = operand(&next).expect("a built value is an operand");
                }
                match acc {
                    Operand::Lazy(e) => Ok(expr_value(e)),
                    Operand::Eng(x, len, d, deps) => expr::finish(func, x, len, d, deps),
                }
            }
            "all_of" | "any_of" => {
                let Some(Value::List(items)) = args.first() else {
                    return Err(format!("ods.{}: expects a list of Bool masks", func));
                };
                if items.is_empty() {
                    return Err(format!("ods.{}: needs at least one mask", func));
                }
                if !holds_expr(items) {
                    // The long-standing checks and their words.
                    super::series::check_masks(func, items)?;
                }
                if items.len() == 1 {
                    return Ok(items[0].clone());
                }
                let op = if func == "all_of" { BinOp::And } else { BinOp::Or };
                let mut acc = operand(&items[0]).ok_or("a mask")?;
                for m in &items[1..] {
                    let v = build(func, Op::Bin(op), vec![acc, operand(m).ok_or("a mask")?])?;
                    acc = operand(&v).expect("a built value is an operand");
                }
                match acc {
                    Operand::Lazy(e) => Ok(expr_value(e)),
                    Operand::Eng(x, len, d, deps) => expr::finish(func, x, len, d, deps),
                }
            }
            _ => Err(String::new()),
        }
    })();
    match r {
        Err(msg) if msg.is_empty() => None,
        r => Some(r),
    }
}

fn expr_value(e: Arc<expr::LExpr>) -> Value {
    Value::Native(crate::native::NativeHandle::new(expr::OdsExpr(e)))
}

/// A Series argument's dtype without forcing it.
fn series_of_unforced(v: &Value) -> Option<DType> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsSeries>().map(|s| s.dtype()),
        _ => None,
    }
}

/// The eager column functions. None when `func` is not one.
pub fn eager(func: &str, args: &[Value]) -> Option<Result<Value, String>> {
    if !FUNCTIONS.iter().any(|(n, _)| *n == func) && func != "fill_null" {
        return None;
    }
    if holds_expr(args) {
        return Some(Ok(call_node(func, args)));
    }
    Some((|| -> Result<Value, String> {
        match func {
            "full" => {
                want_n(func, args, 2, 2)?;
                let n = want_count(func, "the length", &args[0])?;
                let v = value_to_scalar(&args[1]).ok_or_else(|| {
                    format!("ods.full: the value must be a number, String, Bool or (), got {}", args[1].type_name())
                })?;
                Ok(sv(column::full(n, &v)))
            }
            "ones" => {
                want_n(func, args, 1, 1)?;
                let n = want_count(func, "the length", &args[0])?;
                Ok(sv(column::full(n, &Scalar::F64(1.0))))
            }
            "full_like" => {
                want_n(func, args, 2, 2)?;
                let s = want_series(func, args, 0)?;
                let v = value_to_scalar(&args[1]).ok_or_else(|| {
                    format!("ods.full_like: the value must be a number, String, Bool or (), got {}", args[1].type_name())
                })?;
                Ok(sv(s.full_like(&v)))
            }
            "arange" => {
                want_n(func, args, 1, 3)?;
                let (a, b, step) = match args.len() {
                    1 => (Value::Integer(0), args[0].clone(), Value::Integer(1)),
                    2 => (args[0].clone(), args[1].clone(), Value::Integer(1)),
                    _ => (args[0].clone(), args[1].clone(), args[2].clone()),
                };
                match (&a, &b, &step) {
                    (Value::Integer(a), Value::Integer(b), Value::Integer(s)) => {
                        column::arange_i64(*a, *b, *s).map(sv).map_err(|x| format!("ods.{}", x))
                    }
                    _ => {
                        let f = |v: &Value| match v {
                            Value::Integer(x) => Ok(*x as f64),
                            Value::Float(x) => Ok(*x),
                            other => Err(format!("ods.arange: start, stop and step are numbers, got {}", other.type_name())),
                        };
                        column::arange_f64(f(&a)?, f(&b)?, f(&step)?).map(sv).map_err(|x| format!("ods.{}", x))
                    }
                }
            }
            "repeat" => {
                want_n(func, args, 2, 2)?;
                let s = want_series(func, args, 0)?;
                match &args[1] {
                    Value::Integer(_) => Ok(sv(s.repeat(want_count(func, "times", &args[1])?))),
                    v => {
                        let counts: Vec<i64> = match (series_of(v), v) {
                            (Some(Series::I64 { values, validity: None }), _) => values.as_ref().clone(),
                            (_, Value::List(items)) => items
                                .iter()
                                .map(|x| want_int(func, "each count", x))
                                .collect::<Result<_, _>>()?,
                            _ => {
                                return Err(format!(
                                    "ods.repeat: times is an Int, or an Int per element, got {}",
                                    v.type_name()
                                ));
                            }
                        };
                        s.repeat_each(&counts).map(sv).map_err(|x| format!("ods.repeat: {}", x))
                    }
                }
            }
            "tile" => {
                want_n(func, args, 2, 2)?;
                Ok(sv(want_series(func, args, 0)?.tile(want_count(func, "n", &args[1])?)))
            }
            "argmax" | "argmin" => {
                want_n(func, args, 1, 1)?;
                let s = want_series(func, args, 0)?;
                Ok(s.arg_extreme(func == "argmax")
                    .map_err(e)?
                    .map(|i| Value::Integer(i as i64))
                    .unwrap_or(Value::Unit))
            }
            "diff" | "pct_change" => {
                want_n(func, args, 1, 2)?;
                let s = want_series(func, args, 0)?;
                let lag = match args.get(1) {
                    None => 1,
                    Some(v) => want_int(func, "the lag", v)?,
                };
                let r = if func == "diff" { s.diff(lag) } else { s.pct_change(lag) };
                r.map(sv).map_err(|x| format!("ods.{}: {}", func, x))
            }
            "cumprod" => {
                want_n(func, args, 1, 1)?;
                want_series(func, args, 0)?.cumprod().map(sv).map_err(e)
            }
            "mode" => {
                want_n(func, args, 1, 1)?;
                want_series(func, args, 0)?.mode().map(scalar_to_value).map_err(e)
            }
            "any" | "all" => {
                want_n(func, args, 1, 1)?;
                want_series(func, args, 0)?
                    .any_all(func == "all")
                    .map(Value::Boolean)
                    .map_err(|x| format!("ods.{}", x))
            }
            "searchsorted" => {
                want_n(func, args, 2, 3)?;
                let s = want_series(func, args, 0)?;
                let right = match args.get(2) {
                    None => false,
                    Some(Value::String(side)) if side.as_str() == "left" => false,
                    Some(Value::String(side)) if side.as_str() == "right" => true,
                    Some(other) => {
                        return Err(format!(
                            "ods.searchsorted: side is \"left\" or \"right\", got {}",
                            other
                        ));
                    }
                };
                let one = |v: &Scalar| s.searchsorted(v, right).map_err(|x| format!("ods.{}", x));
                if let Some(vs) = series_of(&args[1]) {
                    let out = (0..vs.len())
                        .map(|i| one(&vs.scalar_at(i)).map(|p| p as i64))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(sv(Series::from_i64(out)));
                }
                if let Value::List(items) = &args[1] {
                    let out = items
                        .iter()
                        .map(|v| {
                            let sc = value_to_scalar(v).ok_or("ods.searchsorted: a value is a number or a String")?;
                            one(&sc).map(|p| Value::Integer(p as i64))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(Value::List(Arc::new(out)));
                }
                let v = value_to_scalar(&args[1]).ok_or_else(|| {
                    format!("ods.searchsorted: the value is a number, a String, a list or a Series, got {}", args[1].type_name())
                })?;
                Ok(Value::Integer(one(&v)? as i64))
            }
            "count" => {
                want_n(func, args, 1, 1)?;
                if let Some(r) = expr::reduce_pending(&args[0], Reduce::Count) {
                    return r;
                }
                Ok(Value::Integer(want_series(func, args, 0)?.count() as i64))
            }
            "first" | "last" => {
                want_n(func, args, 1, 1)?;
                let s = want_series(func, args, 0)?;
                Ok(scalar_to_value(if func == "first" { s.first() } else { s.last() }))
            }
            "interpolate" => {
                want_n(func, args, 1, 2)?;
                let s = want_series(func, args, 0)?;
                let limit = limit_arg(func, args.get(1))?;
                s.interpolate(limit).map(sv).map_err(|x| format!("ods.{}", x))
            }
            "fill_null" => {
                // The method forms (the value forms are elementwise).
                want_n(func, args, 2, 3)?;
                let s = want_series(func, args, 0)?;
                let Some(Value::String(m)) = args.get(1) else {
                    unreachable!("a method")
                };
                let limit = limit_arg(func, args.get(2))?;
                match m.as_str() {
                    "ffill" => Ok(sv(s.fill_direction(true, limit))),
                    "bfill" => Ok(sv(s.fill_direction(false, limit))),
                    "linear" => s.interpolate(limit).map(sv).map_err(|x| format!("ods.fill_null: {}", x)),
                    _ => {
                        if limit.is_some() {
                            return Err(format!("ods.fill_null: \"{}\" takes no limit", m));
                        }
                        s.fill_stat(m.as_str() == "median")
                            .map(sv)
                            .map_err(|x| format!("ods.fill_null: {}", x))
                    }
                }
            }
            "recode" => {
                want_n(func, args, 2, 3)?;
                let s = want_series(func, args, 0)?;
                let (from, to) = recode_map(s, &args[1])?;
                let unmapped = match args.get(2) {
                    None => Unmapped::Keep,
                    Some(Value::Unit) => Unmapped::Null,
                    Some(v) => Unmapped::Value(value_to_scalar(v).ok_or_else(|| {
                        format!("ods.recode: the default is a value, got {}", v.type_name())
                    })?),
                };
                s.recode(&from, &to, &unmapped).map(sv).map_err(|x| format!("ods.recode: {}", x))
            }
            _ => Err(format!("ods.{} is handled elsewhere", func)),
        }
    })())
}

fn limit_arg(func: &str, v: Option<&Value>) -> Result<Option<usize>, String> {
    match v {
        None | Some(Value::Unit) => Ok(None),
        Some(v) => want_count(func, "limit", v).map(Some),
    }
}

/// A recode map as two columns: an object or map (its keys text, read
/// as numbers for a numeric column) or a list of `[from, to]` pairs.
fn recode_map(s: &Series, m: &Value) -> Result<(Series, Series), String> {
    let pairs: Vec<(Value, Value)> = match m {
        Value::Map(map) => map
            .iter()
            .map(|(k, v)| (Value::String(Arc::new(k.clone())), v.clone()))
            .collect(),
        Value::Struct { fields, .. } => fields
            .iter()
            .map(|(k, v)| (Value::String(Arc::new(k.clone())), v.clone()))
            .collect(),
        Value::List(items) => items
            .iter()
            .map(|p| match p {
                Value::List(kv) | Value::Tuple(kv) if kv.len() == 2 => Ok((kv[0].clone(), kv[1].clone())),
                other => Err(format!("ods.recode: each pair is [from, to], got {}", other)),
            })
            .collect::<Result<_, _>>()?,
        other => {
            return Err(format!(
                "ods.recode: the map is a map, an object or a list of [from, to] pairs, got {}",
                other.type_name()
            ));
        }
    };
    let key = |v: &Value| -> Result<Value, String> {
        Ok(match (s.dtype(), v) {
            (DType::I64, Value::String(t)) => Value::Integer(t.trim().parse().map_err(|_| {
                format!("ods.recode: the key \"{}\" is not an Int, as the column is", t)
            })?),
            (DType::F64, Value::String(t)) => Value::Float(t.trim().parse().map_err(|_| {
                format!("ods.recode: the key \"{}\" is not a number, as the column is", t)
            })?),
            (DType::Bool, Value::String(t)) => match t.as_str() {
                "true" => Value::Boolean(true),
                "false" => Value::Boolean(false),
                _ => return Err(format!("ods.recode: the key \"{}\" is not a Bool", t)),
            },
            (_, v) => v.clone(),
        })
    };
    let froms = pairs.iter().map(|(k, _)| key(k)).collect::<Result<Vec<_>, _>>()?;
    let tos: Vec<Value> = pairs.into_iter().map(|(_, v)| v).collect();
    let list = |items: Vec<Value>| -> Result<Series, String> {
        super::series::series_from_list(&items).map_err(|x| x.replace("ods.series", "ods.recode"))
    };
    Ok((list(froms)?, list(tos)?))
}

/// The fused reductions: a pending Series folds into its sum, mean,
/// minimum or maximum without being materialized. None otherwise.
pub fn reduction(func: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let kind = match func {
        "sum" => Reduce::Sum,
        "mean" => Reduce::Mean,
        "min" => Reduce::Min,
        "max" => Reduce::Max,
        "dot" if args.len() == 2 => return expr::dot_fused(&args[0], &args[1]),
        _ => return None,
    };
    if args.len() != 1 {
        return None;
    }
    expr::reduce_pending(&args[0], kind)
}

/// Methods of `fill_null` route to the eager kernels.
pub fn fill_method(args: &[Value]) -> bool {
    is_method(args.get(1))
}

//! Text, dates and categories (data ergonomics D2): the String
//! column's methods, a Date or Datetime column's parts and arithmetic,
//! and Category columns with their indicator columns.
//!
//! A date's parts, its truncation and its month arithmetic are the
//! expression engine's (`fuse::TOp`): they fuse with the operators and
//! comparisons around them, and take an Expr as readily as a Series.
//! Text and category kernels are eager (`olang_ods::text`,
//! `olang_ods::category`), over spans and codes, and become call nodes
//! when given an Expr.

use super::expr::{Operand, build, call_node, holds_expr, operand};
use super::frame::{OdsFrame, frame_of};
use super::series::{OdsSeries, series_of};
use crate::ast::Value;
use olang_ods::fuse::{BinOp, Expr, Op, TOp};
use olang_ods::temporal::US_PER_DAY;
use olang_ods::time::Every;
use olang_ods::{DType, Frame, Scalar, Series, category, text};
use std::sync::Arc;

/// (name, most arguments) of this file's functions.
pub const FUNCTIONS: &[(&str, usize)] = &[
    // text
    ("lower", 1),
    ("upper", 1),
    ("trim", 1),
    ("trim_start", 1),
    ("trim_end", 1),
    ("replace", 3),
    ("replace_re", 3),
    ("contains", 3),
    ("starts_with", 2),
    ("ends_with", 2),
    ("len_chars", 1),
    ("substring", 3),
    ("split_n", 3),
    ("pad_start", 3),
    ("pad_end", 3),
    ("to_int", 1),
    ("to_float", 1),
    ("to_date", 2),
    ("to_datetime", 2),
    ("format", 2),
    // dates
    ("year", 1),
    ("quarter", 1),
    ("month", 1),
    ("day", 1),
    ("weekday", 1),
    ("day_of_year", 1),
    ("week", 1),
    ("hour", 1),
    ("minute", 1),
    ("second", 1),
    ("is_weekend", 1),
    ("date", 1),
    ("day_name", 1),
    ("month_name", 1),
    ("months_between", 2),
    ("add_days", 2),
    ("add_months", 2),
    // categories
    ("to_category", 2),
    ("levels", 1),
    ("codes", 1),
    ("dummies", 5),
    ("interact", 3),
];

/// Whether `func` is one of this file's (or `floor` with a unit, or a
/// Category's `fill_null` with a level).
pub fn handles(func: &str, args: &[Value]) -> bool {
    FUNCTIONS.iter().any(|(n, _)| *n == func)
        || (func == "floor" && args.len() == 2)
        || category_fill(func, args)
}

/// `cat.fill_null("none")`: a Category stays one (a new level is added).
fn category_fill(func: &str, args: &[Value]) -> bool {
    func == "fill_null"
        && args.len() == 2
        && matches!(args[1], Value::String(_))
        && matches!(&args[0], Value::Native(h) if h.0.as_any().downcast_ref::<OdsSeries>().is_some_and(|s| s.dtype() == DType::Category))
}

fn want_n(func: &str, args: &[Value], lo: usize, hi: usize) -> Result<(), String> {
    if args.len() < lo || args.len() > hi {
        let want = if lo == hi {
            lo.to_string()
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

fn want_text<'a>(func: &str, what: &str, v: Option<&'a Value>) -> Result<&'a str, String> {
    match v {
        Some(Value::String(s)) => Ok(s.as_str()),
        other => Err(format!(
            "ods.{}: {} must be a String, got {}",
            func,
            what,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

fn opt_text<'a>(func: &str, what: &str, v: Option<&'a Value>) -> Result<Option<&'a str>, String> {
    match v {
        None | Some(Value::Unit) => Ok(None),
        v => want_text(func, what, v).map(Some),
    }
}

fn want_int(func: &str, what: &str, v: Option<&Value>) -> Result<i64, String> {
    match v {
        Some(Value::Integer(n)) => Ok(*n),
        other => Err(format!(
            "ods.{}: {} must be an Int, got {}",
            func,
            what,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

fn want_count(func: &str, what: &str, v: Option<&Value>) -> Result<usize, String> {
    let n = want_int(func, what, v)?;
    usize::try_from(n).map_err(|_| format!("ods.{}: {} cannot be negative, got {}", func, what, n))
}

fn sv(s: Series) -> Value {
    OdsSeries::into_value(s)
}

fn e(func: &str) -> impl Fn(olang_ods::OdsError) -> String + '_ {
    move |err| {
        let m = err.to_string();
        if m.starts_with(func) {
            format!("ods.{}", m)
        } else {
            format!("ods.{}: {}", func, m)
        }
    }
}

fn fmt_float(x: f64) -> String {
    crate::ast::format_float(x)
}

/// The type names `dtype:` and `dtypes:` take.
pub fn dtype_named(name: &str) -> Option<DType> {
    Some(match name {
        "Float" => DType::F64,
        "Int" => DType::I64,
        "Bool" => DType::Bool,
        "String" => DType::Str,
        "Date" => DType::Date,
        "Datetime" => DType::Datetime,
        "Category" => DType::Category,
        _ => return None,
    })
}

/// The elementwise date functions: an engine expression (lazy Series)
/// or an Expr. None when `func` (with these arguments) is not one.
pub fn elementwise(func: &str, args: &[Value]) -> Option<Result<Value, String>> {
    let temporal = |t: TOp| -> Result<Value, String> {
        want_n(func, args, 1, 1)?;
        build(func, Op::Temporal(t, false), vec![opnd(func, args, 0)?])
    };
    Some(match func {
        "year" | "quarter" | "month" | "day" | "weekday" | "day_of_year" | "week" | "hour"
        | "minute" | "second" | "is_weekend" | "date" => temporal(TOp::part(func).expect("a part")),
        "floor" if args.len() == 2 => (|| {
            let every = Every::parse(want_text(func, "the unit", args.get(1))?)
                .map_err(|m| format!("ods.floor: {}", m))?;
            build(
                func,
                Op::Temporal(TOp::Floor(every), false),
                vec![opnd(func, args, 0)?],
            )
        })(),
        "add_months" => (|| {
            want_n(func, args, 2, 2)?;
            // A constant, or an Int a row (a column or an Expr).
            if let Some(Value::Integer(n)) = args.get(1) {
                return build(
                    func,
                    Op::Temporal(TOp::AddMonths(*n), false),
                    vec![opnd(func, args, 0)?],
                );
            }
            if !matches!(args.get(1), Some(Value::Native(_))) {
                want_int(func, "the number of months", args.get(1))?;
            }
            build(
                func,
                Op::Temporal(TOp::AddMonthsBy, false),
                vec![opnd(func, args, 0)?, opnd(func, args, 1)?],
            )
        })(),
        "months_between" => (|| {
            want_n(func, args, 2, 2)?;
            build(
                func,
                Op::Temporal(TOp::MonthsBetween, false),
                vec![opnd(func, args, 0)?, opnd(func, args, 1)?],
            )
        })(),
        "add_days" => (|| {
            want_n(func, args, 2, 2)?;
            let d = opnd(func, args, 0)?;
            let n = opnd(func, args, 1)?;
            // A Datetime moves by whole days of microseconds.
            if operand_dtype(&d) == Some(DType::Datetime) {
                let raw = build(func, Op::Temporal(TOp::Raw, false), vec![d])?;
                let us = build(
                    func,
                    Op::Bin(BinOp::Mul),
                    vec![
                        n,
                        Operand::Eng(Expr::lit(Scalar::I64(US_PER_DAY)), None, 0, vec![]),
                    ],
                )?;
                let sum = build(
                    func,
                    Op::Bin(BinOp::Add),
                    vec![operand(&raw).expect("built"), operand(&us).expect("built")],
                )?;
                return build(
                    func,
                    Op::Temporal(TOp::AsDatetime, false),
                    vec![operand(&sum).expect("built")],
                );
            }
            build(func, Op::Bin(BinOp::Add), vec![d, n])
        })(),
        _ => return None,
    })
}

/// A concrete operand's dtype (None for an Expr, which is resolved
/// later, or a null).
fn operand_dtype(o: &Operand) -> Option<DType> {
    match o {
        Operand::Eng(e, ..) => e.dtype().ok().flatten(),
        Operand::Lazy(_) => None,
    }
}

/// The eager text, date and category functions. None when `func` is not
/// one; a call node when an argument is an Expr.
pub fn eager(func: &str, args: &[Value]) -> Option<Result<Value, String>> {
    if category_fill(func, args) {
        let s = series_of(&args[0]).expect("a category").clone();
        let Value::String(t) = &args[1] else {
            unreachable!("checked")
        };
        return Some(
            s.fill_null(Scalar::Str(t.to_string()))
                .map(sv)
                .map_err(e(func)),
        );
    }
    if !FUNCTIONS.iter().any(|(n, _)| *n == func) {
        return None;
    }
    if holds_expr(args) {
        return Some(Ok(call_node(func, args)));
    }
    Some(run(func, args))
}

fn run(func: &str, args: &[Value]) -> Result<Value, String> {
    let max = FUNCTIONS
        .iter()
        .find(|(n, _)| *n == func)
        .map(|(_, a)| *a)
        .expect("one of ours");
    match func {
        "dummies" => return dummies(args),
        "interact" => {
            want_n(func, args, 2, 3)?;
            let a = want_series(func, args, 0)?;
            let b = want_series(func, args, 1)?;
            let sep = opt_text(func, "sep", args.get(2))?.unwrap_or(":");
            return category::interact(a, b, sep, &fmt_float)
                .map(sv)
                .map_err(e(func));
        }
        _ => {}
    }
    want_n(func, args, 1, max)?;
    let s = want_series(func, args, 0)?;
    let out = match func {
        "lower" => text::lower(s),
        "upper" => text::upper(s),
        "trim" => text::trim(s, true, true),
        "trim_start" => text::trim(s, true, false),
        "trim_end" => text::trim(s, false, true),
        "replace" => {
            let from = want_text(func, "the text to replace", args.get(1))?;
            let to = want_text(func, "the replacement", args.get(2))?;
            text::replace(s, from, to)
        }
        "replace_re" => {
            let pat = want_text(func, "the pattern", args.get(1))?;
            let to = want_text(func, "the replacement", args.get(2))?;
            return replace_re(s, pat, to);
        }
        "contains" => {
            let t = want_text(func, "the text", args.get(1))?;
            let case = match args.get(2) {
                None => true,
                Some(Value::Boolean(b)) => *b,
                Some(other) => {
                    return Err(format!(
                        "ods.contains: case must be a Bool, got {}",
                        other.type_name()
                    ));
                }
            };
            text::contains(s, t, case)
        }
        "starts_with" => text::starts_with(s, want_text(func, "the prefix", args.get(1))?),
        "ends_with" => text::ends_with(s, want_text(func, "the suffix", args.get(1))?),
        "len_chars" => text::len_chars(s),
        "substring" => {
            let a = want_count(func, "the start", args.get(1))?;
            let b = match args.get(2) {
                None | Some(Value::Unit) => None,
                v => Some(want_count(func, "the end", v)?),
            };
            text::substring(s, a, b)
        }
        "split_n" => {
            let sep = want_text(func, "the separator", args.get(1))?;
            let i = want_int(func, "the piece", args.get(2))?;
            text::split_n(s, sep, i)
        }
        "pad_start" | "pad_end" => {
            let w = want_count(func, "the width", args.get(1))?;
            let fill = opt_text(func, "the fill", args.get(2))?.unwrap_or(" ");
            text::pad(s, w, fill, func == "pad_start")
        }
        "to_int" => text::to_int(s),
        "to_float" => text::to_float(s),
        "to_date" | "to_datetime" => {
            let fmt = opt_text(func, "the format", args.get(1))?;
            // A column that already is one is converted, not parsed.
            match (s.dtype(), func) {
                (DType::Date | DType::Datetime, "to_date") if fmt.is_none() => {
                    s.cast(DType::Date, &fmt_float)
                }
                (DType::Date | DType::Datetime, _) if fmt.is_none() => {
                    s.cast(DType::Datetime, &fmt_float)
                }
                _ if func == "to_date" => text::to_date(s, fmt),
                _ => text::to_datetime(s, fmt),
            }
        }
        "format" => text::format(s, want_text(func, "the format", args.get(1))?),
        "day_name" | "month_name" => return names(func, s),
        "to_category" => {
            let levels = match args.get(1) {
                None | Some(Value::Unit) => None,
                Some(v) => Some(level_list(v)?),
            };
            category::to_category(s, levels.as_deref(), &fmt_float)
        }
        "levels" => category::levels(s),
        "codes" => category::codes(s),
        _ => unreachable!("one of ours: {}", func),
    };
    out.map(sv).map_err(e(func))
}

fn level_list(v: &Value) -> Result<Vec<String>, String> {
    if let Some(s) = series_of(v) {
        return Ok((0..s.len())
            .map(|i| super::table::scalar_text(s.scalar_at(i)).unwrap_or_default())
            .collect());
    }
    match v {
        Value::List(items) => items
            .iter()
            .map(|x| match x {
                Value::String(t) => Ok(t.to_string()),
                Value::Integer(n) => Ok(n.to_string()),
                Value::Float(f) => Ok(fmt_float(*f)),
                Value::Boolean(b) => Ok(b.to_string()),
                other => Err(format!(
                    "ods.to_category: a level is a String (or a number), got {}",
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "ods.to_category: levels are a list, got {}",
            other.type_name()
        )),
    }
}

/// A date's weekday or month name, as a Category whose levels are the
/// calendar's (Monday … Sunday, January … December), so it sorts and
/// plots in calendar order.
fn names(func: &str, s: &Series) -> Result<Value, String> {
    use olang_ods::temporal::{DAY_NAMES, MONTH_NAMES};
    let t = if func == "day_name" {
        TOp::Weekday
    } else {
        TOp::Month
    };
    let part = Expr::op(Op::Temporal(t, false), vec![Expr::col(s.clone())])
        .eval(s.len(), false)
        .map_err(e(func))?;
    let Series::I64 { values, validity } = part else {
        unreachable!("a part is Int")
    };
    let names: &[&str] = if func == "day_name" {
        &DAY_NAMES
    } else {
        &MONTH_NAMES
    };
    Ok(sv(Series::category(
        values.iter().map(|&k| (k.max(1) - 1) as u32).collect(),
        olang_ods::StrCol::from_strings(names.iter()),
        validity,
    )))
}

#[cfg(feature = "regex-module")]
fn replace_re(s: &Series, pat: &str, to: &str) -> Result<Value, String> {
    let re = regex::Regex::new(pat)
        .map_err(|err| format!("ods.replace_re: invalid pattern: {}", err))?;
    text::transform("replace_re", s, |t, out| {
        out.push_str(&re.replace_all(t, to));
        true
    })
    .map(sv)
    .map_err(e("replace_re"))
}

#[cfg(not(feature = "regex-module"))]
fn replace_re(_s: &Series, _pat: &str, _to: &str) -> Result<Value, String> {
    Err(
        "ods.replace_re: this build has no regular expressions (the `regex-module` feature)"
            .to_string(),
    )
}

/// `ods.dummies(s | f, columns?, drop_first, prefix?, dtype)`: indicator
/// columns as a Frame. A Frame's String and Category columns (or those
/// named) are each replaced, where they stand, by theirs.
fn dummies(args: &[Value]) -> Result<Value, String> {
    let func = "dummies";
    want_n(func, args, 1, 5)?;
    let drop_first = match args.get(2) {
        None | Some(Value::Unit) => true,
        Some(Value::Boolean(b)) => *b,
        Some(other) => {
            return Err(format!(
                "ods.dummies: drop_first must be a Bool, got {}",
                other.type_name()
            ));
        }
    };
    let prefix = opt_text(func, "prefix", args.get(3))?;
    let dtype = match opt_text(func, "dtype", args.get(4))? {
        None => DType::F64,
        Some(t) => match dtype_named(t) {
            Some(d @ (DType::F64 | DType::I64 | DType::Bool)) => d,
            _ => {
                return Err(format!(
                    "ods.dummies: dtype is \"Float\", \"Int\" or \"Bool\", got \"{}\"",
                    t
                ));
            }
        },
    };
    let one = |s: &Series, p: Option<&str>| {
        category::dummies(s, p, drop_first, dtype, &fmt_float).map_err(e(func))
    };
    if let Some(s) = args.first().and_then(series_of) {
        if !matches!(args.get(1), None | Some(Value::Unit)) {
            return Err(
                "ods.dummies: columns are a Frame's; a Series takes prefix: instead".to_string(),
            );
        }
        let cols = one(s, prefix)?;
        return Frame::new(cols)
            .map(OdsFrame::into_value)
            .map_err(|x| x.to_string());
    }
    let Some(f) = args.first().and_then(frame_of) else {
        return Err(format!(
            "ods.dummies: argument 1 must be a Series or a Frame, got {}",
            args.first().map(|v| v.type_name()).unwrap_or_default()
        ));
    };
    let wanted: Vec<String> = match args.get(1) {
        None | Some(Value::Unit) => f
            .names()
            .iter()
            .zip(f.columns())
            .filter(|(_, c)| matches!(c.dtype(), DType::Str | DType::Category))
            .map(|(n, _)| n.clone())
            .collect(),
        Some(Value::String(n)) => vec![n.to_string()],
        Some(Value::List(items)) => items
            .iter()
            .map(|v| match v {
                Value::String(n) => Ok(n.to_string()),
                other => Err(format!(
                    "ods.dummies: a column name is a String, got {}",
                    other.type_name()
                )),
            })
            .collect::<Result<_, _>>()?,
        Some(other) => {
            return Err(format!(
                "ods.dummies: columns are a name or a list of names, got {}",
                other.type_name()
            ));
        }
    };
    for w in &wanted {
        if f.column(w).is_err() {
            return Err(format!(
                "ods.dummies: no column '{}' in this Frame. It has: {}",
                w,
                f.names().join(", ")
            ));
        }
    }
    let mut pairs: Vec<(String, Series)> = Vec::new();
    for (name, col) in f.names().iter().zip(f.columns()) {
        if wanted.contains(name) {
            pairs.extend(one(col, Some(prefix.unwrap_or(name)))?);
        } else {
            pairs.push((name.clone(), col.clone()));
        }
    }
    Frame::new(pairs)
        .map(OdsFrame::into_value)
        .map_err(|x| format!("ods.dummies: {}", x))
}

/// A String operand standing against a Date or Datetime in a comparison
/// or a subtraction is read as one (`d > "2024-01-01"`, `t - t.min()`,
/// whose minimum is ISO text); text that is no date is an error naming
/// it.
pub fn dates_from_text(op: Op, kids: Vec<Arc<Expr>>) -> Result<Vec<Arc<Expr>>, String> {
    let cmp = matches!(
        op,
        Op::Bin(BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Sub)
            | Op::Between(_)
            | Op::Where
            | Op::Coalesce
    );
    if !cmp {
        return Ok(kids);
    }
    let want = kids.iter().find_map(|k| match k.dtype() {
        Ok(Some(d @ (DType::Date | DType::Datetime))) => Some(d),
        _ => None,
    });
    let Some(want) = want else { return Ok(kids) };
    kids.into_iter()
        .map(|k| match k.as_ref() {
            Expr::Lit(Scalar::Str(t)) => {
                let read = olang_ods::temporal::parse_iso(t.trim()).map(|(us, _)| us);
                match (read, want) {
                    (Some(us), DType::Date) => Ok(Expr::lit(Scalar::Date(
                        olang_ods::temporal::day_of_us(us) as i32,
                    ))),
                    (Some(us), _) => Ok(Expr::lit(Scalar::Datetime(us))),
                    (None, _) => Err(format!(
                        "\"{}\" is not an ISO date (YYYY-MM-DD[THH:MM[:SS]]) to {} a {} with",
                        t,
                        if op == Op::Bin(BinOp::Sub) {
                            "subtract from"
                        } else {
                            "compare"
                        },
                        want
                    )),
                }
            }
            _ => Ok(k),
        })
        .collect()
}

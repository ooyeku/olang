//! Frames: shape, group, join, time (docs/ods.md).
//!
//! - **GroupBy**, a native value: `ods.group_by(f, keys)` with no
//!   aggregations computes the group ids once (the engine's `Grouping`)
//!   and every verb over it reuses them: `agg` with expressions, the
//!   whole-Frame reductions (`mean`, `sum`, …), `size`, `transform`,
//!   `apply` (one olang call per group), `groups`.
//! - **Aggregation expressions**: an aggregation call node
//!   (`col("x").sum()`, `ods.count()`, `col("x").quantile(0.9)`) reduces
//!   within each group, and arithmetic over aggregations is evaluated
//!   per group; the same evaluator answers `expr.over(keys)`.
//! - **Shape and access**: `slice`, `row`, `lookup` (a hash index cached
//!   with the Frame), `frame` from an object, `rename` with an object.
//! - **Crosstab, runs, pivot `fill`**, **joins** on several keys with
//!   `how`, `suffix` and `validate`, and **time**: `resample` and the
//!   rolling options.

use super::expr::{self, LArg, LExpr, call_node, expr_of, holds_expr, named};
use super::frame::{OdsFrame, frame_of, parse_aggs, string_list, want_frame};
use super::series::{OdsSeries, scalar_to_value, series_from_list, series_of, value_to_scalar};
use crate::ast::{Value, ValueMap};
use crate::native::{NativeHandle, NativeObject};
use olang_ods::fuse::Expr;
use olang_ods::group::{Grouping, Normalize};
use olang_ods::time::Every;
use olang_ods::window::Rolling;
use olang_ods::{AggOp, AsOf, DType, Frame, JoinHow, JoinSpec, Scalar, Series, Validate};
use std::any::Any;
use std::sync::Arc;

/// (name, arity) of this file's functions; the arity is the most a call
/// takes.
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("agg", 2),
    ("size", 1),
    ("transform", 2),
    ("apply", 2),
    ("groups", 1),
    ("slice", 3),
    ("row", 2),
    ("lookup", 4),
    ("crosstab", 6),
    ("runs", 1),
    ("run_id", 1),
    ("resample", 5),
    ("asof_join", 7),
];

/// The aggregations a GroupBy answers as a method over its columns.
const REDUCTIONS: &[&str] = &[
    "sum", "mean", "min", "max", "count", "median", "std", "var", "first", "last", "n_unique",
    "any", "all", "argmax", "argmin", "quantile",
];

// ---------------------------------------------------------------------
// The GroupBy value
// ---------------------------------------------------------------------

/// A Frame grouped by key columns: the group ids, computed once.
#[derive(Debug)]
pub struct OdsGroupBy {
    frame: Frame,
    keys: Vec<String>,
    g: Arc<Grouping>,
}

impl NativeObject for OdsGroupBy {
    fn module(&self) -> &'static str {
        "ods"
    }

    fn type_name(&self) -> &'static str {
        "GroupBy"
    }

    fn display(&self) -> String {
        format!(
            "<GroupBy by {}: {} group{} of {} row{}>",
            self.keys.join(", "),
            self.g.n_groups(),
            if self.g.n_groups() == 1 { "" } else { "s" },
            self.frame.n_rows(),
            if self.frame.n_rows() == 1 { "" } else { "s" }
        )
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<OdsGroupBy>()
            .is_some_and(|o| o.keys == self.keys && Arc::ptr_eq(&o.g, &self.g))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `len(by)`: the number of groups.
    fn length(&self) -> Option<usize> {
        Some(self.g.n_groups())
    }
}

fn group_by_of(v: &Value) -> Option<&OdsGroupBy> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsGroupBy>(),
        _ => None,
    }
}

fn want_group_by<'a>(func: &str, args: &'a [Value]) -> Result<&'a OdsGroupBy, String> {
    args.first().and_then(group_by_of).ok_or_else(|| {
        format!(
            "ods.{}: argument 1 must be a GroupBy (from ods.group_by(f, keys)), got {}",
            func,
            args.first().map(|v| v.type_name()).unwrap_or_default()
        )
    })
}

fn new_group_by(f: &Frame, keys: Vec<String>) -> Result<OdsGroupBy, String> {
    let cols: Vec<&Series> = keys
        .iter()
        .map(|k| {
            f.column(k).map_err(|_| {
                format!(
                    "ods.group_by: no column '{}' in this Frame. It has: {}",
                    k,
                    f.names().join(", ")
                )
            })
        })
        .collect::<Result<_, _>>()?;
    if cols.is_empty() {
        return Err("ods.group_by needs at least one key column".to_string());
    }
    let g = Grouping::new(&cols).map_err(|e| e.to_string())?;
    Ok(OdsGroupBy {
        frame: f.clone(),
        keys,
        g: Arc::new(g),
    })
}

impl OdsGroupBy {
    /// The key columns, one value per group.
    fn key_columns(&self) -> Result<Vec<(String, Series)>, String> {
        self.keys
            .iter()
            .map(|k| {
                let c = self.frame.column(k).map_err(|e| e.to_string())?;
                Ok((k.clone(), self.g.keys_of(c).map_err(|e| e.to_string())?))
            })
            .collect()
    }

    fn with_keys(&self, cols: Vec<(String, Series)>) -> Result<Value, String> {
        let mut pairs = self.key_columns()?;
        pairs.extend(cols);
        Frame::new(pairs)
            .map(OdsFrame::into_value)
            .map_err(|e| e.to_string())
    }

    /// `agg`'s outputs: named aggregation expressions, or the
    /// long-standing `[out, op, col]` triples.
    fn agg(&self, func: &str, spec: &Value) -> Result<Value, String> {
        let triples = matches!(spec, Value::List(items)
            if items.iter().all(|i| matches!(i, Value::List(_))) && !items.is_empty());
        if triples {
            let specs = parse_aggs(spec)?;
            let mut out = Vec::with_capacity(specs.len());
            for s in specs {
                let col = if s.op == AggOp::Count {
                    self.g.size()
                } else {
                    let c = self.frame.column(&s.col).map_err(|_| {
                        format!(
                            "ods.{}: no column '{}' in this Frame. It has: {}",
                            func,
                            s.col,
                            self.frame.names().join(", ")
                        )
                    })?;
                    self.g
                        .aggregate(c, s.op)
                        .map_err(|e| format!("ods.{}: {}", func, e))?
                };
                out.push((s.out_name, col));
            }
            return self.with_keys(out);
        }
        let items = named(func, spec)?;
        let cols = aggregate_named(func, &self.frame, &self.keys, &self.g, &items)?;
        self.with_keys(cols)
    }

    /// One reduction over every column it applies to (keys left out).
    fn reduce_all(&self, func: &str, q: Option<f64>) -> Result<Value, String> {
        let op = match q {
            Some(q) => AggOp::quantile(q),
            None => expr::agg_of(func).expect("a reduction"),
        };
        let mut out = Vec::new();
        for (name, col) in self.frame.names().iter().zip(self.frame.columns()) {
            if self.keys.contains(name) {
                continue;
            }
            let fits = match op {
                AggOp::Count | AggOp::First | AggOp::Last | AggOp::NUnique => true,
                AggOp::Any | AggOp::All => col.dtype() == DType::Bool,
                _ => matches!(col.dtype(), DType::F64 | DType::I64),
            };
            if !fits {
                continue;
            }
            let s = self
                .g
                .aggregate(col, op)
                .map_err(|e| format!("ods.{}: '{}': {}", func, name, e))?;
            out.push((name.clone(), s));
        }
        self.with_keys(out)
    }
}

// ---------------------------------------------------------------------
// Aggregation expressions
// ---------------------------------------------------------------------

/// A value per group, or one value for every group.
pub(crate) enum GVal {
    Col(Series),
    Lit(Scalar),
}

fn gval_value(v: GVal) -> Value {
    match v {
        GVal::Col(s) => OdsSeries::into_value(s),
        GVal::Lit(s) => scalar_to_value(s),
    }
}

fn agg_needs(func: &str) -> String {
    format!(
        "{}: an aggregation reduces a column within each group — sum, mean, min, max, count, \
         median, quantile, std, var, first, last, n_unique, any, all, argmax or argmin — like \
         col(\"x\").sum(), or ods.count() for the rows",
        func
    )
}

/// The value of `e` within each group of `g` over `f` (grouped by
/// `keys`): an aggregation reduces its rows, arithmetic over
/// aggregations is per group, a key column is each group's key.
pub(crate) fn agg_value(
    f: &Frame,
    keys: &[String],
    g: &Grouping,
    e: &Arc<LExpr>,
) -> Result<GVal, String> {
    agg_value_with(f, keys, g, e, &Pre::new())
}

/// The row-level columns aggregations read, evaluated ahead (in one
/// plan), by their expression's identity.
type Pre = std::collections::HashMap<usize, Series>;

/// The row-level expressions under the aggregations of `e`.
fn agg_inputs(e: &Arc<LExpr>, out: &mut Vec<Arc<LExpr>>) {
    match e.as_ref() {
        LExpr::Alias(inner, _) => agg_inputs(inner, out),
        LExpr::Op(_, kids) => kids.iter().for_each(|k| agg_inputs(k, out)),
        LExpr::Call(func, args) if expr::agg_of(func).is_some() => {
            if let Some(LArg::E(x)) = args.first()
                && !matches!(x.as_ref(), LExpr::Col(_))
            {
                out.push(x.clone());
            }
        }
        LExpr::Call(_, args) => {
            for a in args {
                if let LArg::E(x) = a {
                    agg_inputs(x, out);
                }
            }
        }
        _ => {}
    }
}

fn agg_value_with(
    f: &Frame,
    keys: &[String],
    g: &Grouping,
    e: &Arc<LExpr>,
    pre: &Pre,
) -> Result<GVal, String> {
    let n = g.n_groups();
    Ok(match e.as_ref() {
        LExpr::Alias(inner, _) => agg_value_with(f, keys, g, inner, pre)?,
        LExpr::Lit(s) => GVal::Lit(s.clone()),
        LExpr::Col(name) if keys.contains(name) => {
            let c = f.column(name).map_err(|e| e.to_string())?;
            GVal::Col(g.keys_of(c).map_err(|e| e.to_string())?)
        }
        LExpr::Col(name) => {
            return Err(format!(
                "col({:?}) is not a key, so it has many values in a group; reduce it, like \
                 col({:?}).sum() or col({:?}).first()",
                name, name, name
            ));
        }
        LExpr::Series(s) if s.len() == n => GVal::Col(s.clone()),
        LExpr::Series(s) => {
            return Err(format!(
                "a Series of {} rows stands in an aggregation over {} groups",
                s.len(),
                n
            ));
        }
        LExpr::Over(..) => {
            return Err(
                "over gives a value to every row, so it cannot stand in an aggregation".to_string(),
            );
        }
        LExpr::Call(func, args) if func == "count" && args.is_empty() => GVal::Col(g.size()),
        LExpr::Call(func, args) if expr::agg_of(func).is_some() => {
            let (Some(LArg::E(x)), rest) = (args.first(), &args[args.len().min(1)..]) else {
                return Err(agg_needs(&format!("ods.{}", func)));
            };
            let op = if func == "quantile" {
                match rest.first() {
                    Some(LArg::V(Value::Float(q))) => AggOp::quantile(*q),
                    Some(LArg::V(Value::Integer(q))) => AggOp::quantile(*q as f64),
                    _ => return Err("quantile: q must be a number in [0, 1]".to_string()),
                }
            } else {
                if !rest.is_empty() {
                    return Err(format!("ods.{} takes no argument in an aggregation", func));
                }
                expr::agg_of(func).expect("checked")
            };
            let col = match pre.get(&(Arc::as_ptr(x) as usize)) {
                Some(c) => c.clone(),
                None => expr::eval_on(f, x)?,
            };
            GVal::Col(
                g.aggregate(&col, op)
                    .map_err(|e| format!("ods.{}: {}", func, e))?,
            )
        }
        LExpr::Op(op, kids) => {
            let vals = kids
                .iter()
                .map(|k| agg_value_with(f, keys, g, k, pre))
                .collect::<Result<Vec<_>, _>>()?;
            let any_col = vals.iter().any(|v| matches!(v, GVal::Col(_)));
            let kids: Vec<Arc<Expr>> = vals
                .into_iter()
                .map(|v| match v {
                    GVal::Col(s) => Expr::col(s),
                    GVal::Lit(s) => Expr::lit(s),
                })
                .collect();
            let x = Expr::op(*op, kids);
            if any_col {
                GVal::Col(expr::eval_one(&x, Some(n))?)
            } else {
                GVal::Lit(expr::eval_one(&x, Some(1))?.scalar_at(0))
            }
        }
        LExpr::Call(func, args) => {
            fn arg(
                f: &Frame,
                keys: &[String],
                g: &Grouping,
                a: &LArg,
                pre: &Pre,
            ) -> Result<Value, String> {
                Ok(match a {
                    LArg::E(e) => gval_value(agg_value_with(f, keys, g, e, pre)?),
                    LArg::V(v) => v.clone(),
                    LArg::L(items) => Value::List(Arc::new(
                        items
                            .iter()
                            .map(|x| arg(f, keys, g, x, pre))
                            .collect::<Result<_, _>>()?,
                    )),
                })
            }
            let vals = args
                .iter()
                .map(|a| arg(f, keys, g, a, pre))
                .collect::<Result<Vec<_>, _>>()?;
            let out = super::call(func, vals)?;
            if let Some(s) = series_of(&out) {
                if s.len() != n {
                    return Err(format!(
                        "ods.{} gives {} rows in an aggregation over {} groups",
                        func,
                        s.len(),
                        n
                    ));
                }
                GVal::Col(s.clone())
            } else if let Some(s) = value_to_scalar(&out) {
                GVal::Lit(s)
            } else {
                return Err(format!(
                    "ods.{} gives a {}, which is not a column or a value",
                    func,
                    out.type_name()
                ));
            }
        }
    })
}

/// Named aggregations over a grouping, one column each (n_groups rows).
fn aggregate_named(
    func: &str,
    f: &Frame,
    keys: &[String],
    g: &Grouping,
    items: &[(String, Value)],
) -> Result<Vec<(String, Series)>, String> {
    let n = g.n_groups();
    // Every row-level expression the aggregations read, in one plan.
    let mut inputs = Vec::new();
    for (_, v) in items {
        if let Some(e) = expr_of(v) {
            agg_inputs(e, &mut inputs);
        }
    }
    let mut pre = Pre::new();
    if !inputs.is_empty() {
        let cols = expr::eval_many(f, &inputs).map_err(|e| format!("ods.{}: {}", func, e))?;
        for (x, c) in inputs.iter().zip(cols) {
            pre.insert(Arc::as_ptr(x) as usize, c);
        }
    }
    items
        .iter()
        .map(|(name, v)| {
            let val = match expr_of(v) {
                Some(e) => agg_value_with(f, keys, g, e, &pre).map_err(|e| format!("ods.{}: '{}': {}", func, name, e))?,
                // text where an aggregation goes is an op's name more
                // often than a constant: refused, the expression named
                None if matches!(v, Value::String(_)) => {
                    let Value::String(t) = v else { unreachable!() };
                    let hint = if t.as_str() == "quantile" {
                        format!("col(\"{}\").quantile(0.5)", name)
                    } else if expr::agg_of(t.as_str()).is_some() {
                        format!("col(\"{}\").{}()", name, t)
                    } else {
                        "col(\"x\").sum()".to_string()
                    };
                    return Err(format!(
                        "ods.{}: '{}' is the text \"{}\"; an aggregation is an expression like {} \
                         (ods.lit(\"{}\") for a constant column; the triple [\"{}\", \"{}\", \"{}\"] names an op)",
                        func, name, t, hint, t, name, t, name
                    ));
                }
                None => match value_to_scalar(v) {
                    Some(s) => GVal::Lit(s),
                    None => {
                        return Err(format!(
                            "ods.{}: '{}' is a {}; an aggregation is an expression like \
                             col(\"x\").sum()",
                            func,
                            name,
                            v.type_name()
                        ));
                    }
                },
            };
            let col = match val {
                GVal::Col(s) => s,
                GVal::Lit(s) => olang_ods::column::full(n, &s),
            };
            Ok((name.clone(), col))
        })
        .collect()
}

// ---------------------------------------------------------------------
// apply: the tiers call the function, this file does the rest
// ---------------------------------------------------------------------

/// What `apply` needs after the calls: the grouping's keys per group.
pub struct ApplyState {
    keys: Vec<(String, Series)>,
}

/// `ods.apply(by, fn)`'s groups, one Frame each (in group order), and
/// what `apply_finish` combines the results with.
pub fn apply_groups(args: &[Value]) -> Result<(Vec<Value>, ApplyState), String> {
    if args.len() != 2 {
        return Err(format!(
            "ods.apply expects a GroupBy and a function, got {} arguments",
            args.len()
        ));
    }
    let by = want_group_by("apply", args)?;
    let parts = by.g.split(&by.frame).map_err(|e| e.to_string())?;
    Ok((
        parts.into_iter().map(OdsFrame::into_value).collect(),
        ApplyState {
            keys: by.key_columns()?,
        },
    ))
}

/// Whether a value is a function `apply` can call.
pub fn is_callable(v: &Value) -> bool {
    matches!(v, Value::Function(_) | Value::Builtin(_))
}

/// One result per group combined: Frames are stacked under their group's
/// keys; records (objects or maps) become a row each and plain values a
/// `value` column, beside the keys; Series are stacked as a `value`
/// column; anything else is the list of results.
pub fn apply_finish(state: ApplyState, results: Vec<Value>) -> Result<Value, String> {
    let n = results.len();
    let keys_at = |rows: &[usize]| -> Result<Vec<(String, Series)>, String> {
        let idx = Series::from_i64(rows.iter().map(|&r| r as i64).collect());
        state
            .keys
            .iter()
            .map(|(k, s)| Ok((k.clone(), s.take(&idx).map_err(|e| e.to_string())?)))
            .collect()
    };
    if n == 0 {
        return keys_at(&[]).and_then(|k| {
            Frame::new(k)
                .map(OdsFrame::into_value)
                .map_err(|e| e.to_string())
        });
    }
    if results.iter().all(|r| frame_of(r).is_some()) {
        let mut frames = Vec::with_capacity(n);
        for (g, r) in results.iter().enumerate() {
            let rf = frame_of(r).expect("checked");
            let mut pairs: Vec<(String, Series)> = keys_at(&vec![g; rf.n_rows()])?
                .into_iter()
                .filter(|(k, _)| rf.column(k).is_err())
                .collect();
            pairs.extend(rf.names().iter().cloned().zip(rf.columns().iter().cloned()));
            frames.push(
                Frame::new(pairs)
                    .map(OdsFrame::into_value)
                    .map_err(|e| e.to_string())?,
            );
        }
        return super::call("concat", vec![Value::List(Arc::new(frames))]);
    }
    if results.iter().all(|r| series_of(r).is_some()) {
        let mut rows = Vec::new();
        let mut values = Vec::new();
        for (g, r) in results.iter().enumerate() {
            let s = series_of(r).expect("checked");
            rows.extend(std::iter::repeat_n(g, s.len()));
            for i in 0..s.len() {
                values.push(scalar_to_value(s.scalar_at(i)));
            }
        }
        let mut pairs = keys_at(&rows)?;
        pairs.push(("value".to_string(), series_from_list(&values)?));
        return Frame::new(pairs)
            .map(OdsFrame::into_value)
            .map_err(|e| e.to_string());
    }
    let record =
        |r: &Value| expr::entries(r).filter(|_| matches!(r, Value::Map(_) | Value::Struct { .. }));
    if results.iter().all(|r| record(r).is_some()) {
        let mut records = Vec::with_capacity(n);
        for (g, r) in results.iter().enumerate() {
            let mut m = ValueMap::default();
            for (k, s) in &state.keys {
                m.insert(k.clone(), scalar_to_value(s.scalar_at(g)));
            }
            for (k, v) in record(r).expect("checked") {
                m.insert(k, v);
            }
            records.push(Value::Map(Arc::new(m)));
        }
        return super::call("frame_from_records", vec![Value::List(Arc::new(records))]);
    }
    if results.iter().all(|r| value_to_scalar(r).is_some()) {
        let mut pairs = keys_at(&(0..n).collect::<Vec<_>>())?;
        pairs.push(("value".to_string(), series_from_list(&results)?));
        return Frame::new(pairs)
            .map(OdsFrame::into_value)
            .map_err(|e| e.to_string());
    }
    Ok(Value::List(Arc::new(results)))
}

// ---------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------

fn arg<'a>(args: &'a [Value], i: usize) -> Option<&'a Value> {
    args.get(i).filter(|v| !matches!(v, Value::Unit))
}

fn want_str(func: &str, what: &str, v: Option<&Value>) -> Result<String, String> {
    match v {
        Some(Value::String(s)) => Ok(s.to_string()),
        other => Err(format!(
            "ods.{}: {} must be a String, got {}",
            func,
            what,
            other
                .map(|v| v.type_name())
                .unwrap_or_else(|| "nothing".to_string())
        )),
    }
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

fn want_int(func: &str, what: &str, v: Option<&Value>) -> Result<i64, String> {
    match v {
        Some(Value::Integer(n)) => Ok(*n),
        other => Err(format!(
            "ods.{}: {} must be an Int, got {}",
            func,
            what,
            other
                .map(|v| v.type_name())
                .unwrap_or_else(|| "nothing".to_string())
        )),
    }
}

fn want_bool(func: &str, what: &str, v: Option<&Value>, default: bool) -> Result<bool, String> {
    match v {
        None | Some(Value::Unit) => Ok(default),
        Some(Value::Boolean(b)) => Ok(*b),
        Some(other) => Err(format!(
            "ods.{}: {} must be a Bool, got {}",
            func,
            what,
            other.type_name()
        )),
    }
}

fn names_of(func: &str, what: &str, v: &Value) -> Result<Vec<String>, String> {
    string_list(func, std::slice::from_ref(v), 0).map_err(|_| {
        format!(
            "ods.{}: {} is a column name or a list of them, got {}",
            func,
            what,
            v.type_name()
        )
    })
}

fn frame_value(r: olang_ods::frame::Frame) -> Value {
    OdsFrame::into_value(r)
}

fn err(e: olang_ods::OdsError) -> String {
    e.to_string()
}

/// A Frame of its Arc'd Frame value: the receiver's own handle, for
/// the lookup cache.
fn ods_frame_of(v: &Value) -> Option<&OdsFrame> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsFrame>(),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

/// This file's functions, and the existing names it extends (`group_by`
/// without aggregations, the reductions of a GroupBy, `count()`,
/// `rolling` with options, the joins, `pivot` with `fill`, `rename` and
/// `frame` with an object). None when `func` with these arguments is
/// someone else's.
pub fn dispatch(func: &str, args: &[Value]) -> Option<Result<Value, String>> {
    // A GroupBy first: its verbs.
    if let Some(by) = args.first().and_then(group_by_of) {
        let r = match func {
            "agg" => match args.get(1) {
                Some(spec) if args.len() == 2 => by.agg(func, spec),
                _ => Err("ods.agg expects a GroupBy and its aggregations, like by.agg({ total: col(\"x\").sum() })".to_string()),
            },
            "size" => by.with_keys(vec![("size".to_string(), by.g.size())]),
            "groups" => groups(by),
            "transform" => transform(by, args.get(1)),
            "apply" => Err(format!(
                "ods.apply: the second argument must be a function of one group, got {}",
                args.get(1).map(|v| v.type_name()).unwrap_or_else(|| "nothing".to_string())
            )),
            "quantile" => match args.get(1) {
                Some(Value::Float(q)) => by.reduce_all(func, Some(*q)),
                Some(Value::Integer(q)) => by.reduce_all(func, Some(*q as f64)),
                _ => Err("ods.quantile: q must be a number in [0, 1]".to_string()),
            },
            f if REDUCTIONS.contains(&f) => {
                if args.len() > 1 {
                    Err(format!("ods.{} of a GroupBy takes no other argument", f))
                } else {
                    by.reduce_all(f, None)
                }
            }
            _ => return None,
        };
        return Some(r);
    }
    Some(match func {
        // `ods.count()`: the rows (of a group, in an aggregation).
        "count" if args.is_empty() => Ok(call_node("count", &[])),
        "group_by" => {
            let Some(f) = args.first().and_then(frame_of) else {
                return Some(Err(format!(
                    "ods.group_by: argument 1 must be a Frame, got {} — in a pipeline, \
                     `f |> ods.group_by(keys)` groups f",
                    args.first()
                        .map(|v| v.type_name())
                        .unwrap_or_else(|| "nothing".to_string())
                )));
            };
            match arg(args, 2) {
                // The triples: the long-standing path.
                Some(Value::List(items)) if items.iter().all(|i| matches!(i, Value::List(_))) => {
                    return None;
                }
                spec => {
                    let keys = match args.get(1) {
                        Some(v) => match names_of("group_by", "the keys", v) {
                            Ok(k) => k,
                            Err(e) => return Some(Err(e)),
                        },
                        None => {
                            return Some(Err(
                                "ods.group_by expects a Frame and its key columns".to_string()
                            ));
                        }
                    };
                    new_group_by(f, keys).and_then(|by| match spec {
                        None => Ok(Value::Native(NativeHandle::new(by))),
                        Some(spec) => by.agg("group_by", spec),
                    })
                }
            }
        }
        "slice" => slice(args),
        "row" => row(args),
        "lookup" => lookup(args),
        "crosstab" if holds_expr(args) => Ok(call_node(func, args)),
        "crosstab" => crosstab(args),
        "runs" | "run_id" if holds_expr(args) => Ok(call_node(func, args)),
        "runs" => want_series(func, args, 0)
            .and_then(|s| olang_ods::group::runs(s).map(frame_value).map_err(err)),
        "run_id" => run_id(args),
        "resample" => resample(args),
        // `floor(s, every)`: the start of each time's bucket. ISO text
        // here; a numeric column's floor is the math function's.
        "floor" if args.len() == 2 && matches!(args[1], Value::String(_)) => {
            if holds_expr(args) {
                Ok(call_node(func, args))
            } else {
                let Value::String(every) = &args[1] else {
                    unreachable!("matched")
                };
                match series_of(&args[0]) {
                    Some(s) if s.dtype() == DType::Str => Every::parse(&every)
                        .and_then(|e| olang_ods::time::floor(s, e))
                        .map(OdsSeries::into_value)
                        .map_err(|e| format!("ods.floor: {}", e)),
                    _ => return None,
                }
            }
        }
        "rolling" if holds_expr(args) => Ok(call_node(func, args)),
        "rolling" => rolling(args),
        "join" | "join_left" | "join_full" | "join_semi" | "join_anti" => join(func, args),
        "asof_join" => asof_join(args),
        "pivot" if args.len() > 5 => match args[5] {
            Value::Unit => super::call("pivot", args[..5].to_vec()),
            _ => pivot_fill(args),
        },
        "rename" if matches!(args.get(1), Some(Value::Struct { .. })) => rename_object(args),
        "frame" if matches!(args.first(), Some(Value::Map(_) | Value::Struct { .. })) => {
            frame_object(args)
        }
        "agg" | "size" | "transform" | "groups" => Err(format!(
            "ods.{}: argument 1 must be a GroupBy (from ods.group_by(f, keys)), got {}",
            func,
            args.first()
                .map(|v| v.type_name())
                .unwrap_or_else(|| "nothing".to_string())
        )),
        "apply" => Err(format!(
            "ods.apply: argument 1 must be a GroupBy (from ods.group_by(f, keys)), got {}",
            args.first()
                .map(|v| v.type_name())
                .unwrap_or_else(|| "nothing".to_string())
        )),
        _ => return None,
    })
}

fn groups(by: &OdsGroupBy) -> Result<Value, String> {
    let parts = by.g.split(&by.frame).map_err(err)?;
    let keys = by.key_columns()?;
    let out = parts
        .into_iter()
        .enumerate()
        .map(|(g, part)| {
            let key = if keys.len() == 1 {
                scalar_to_value(keys[0].1.scalar_at(g))
            } else {
                Value::Tuple(Arc::new(
                    keys.iter()
                        .map(|(_, s)| scalar_to_value(s.scalar_at(g)))
                        .collect(),
                ))
            };
            Value::Tuple(Arc::new(vec![key, OdsFrame::into_value(part)]))
        })
        .collect();
    Ok(Value::List(Arc::new(out)))
}

fn transform(by: &OdsGroupBy, spec: Option<&Value>) -> Result<Value, String> {
    let Some(spec) = spec else {
        return Err("ods.transform expects a GroupBy and an aggregation, like by.transform(col(\"x\").mean())".to_string());
    };
    let one = |e: &Arc<LExpr>| -> Result<Series, String> {
        match agg_value(&by.frame, &by.keys, &by.g, e)? {
            GVal::Col(s) => by.g.broadcast(&s).map_err(err),
            GVal::Lit(s) => Ok(olang_ods::column::full(by.frame.n_rows(), &s)),
        }
    };
    if let Some(e) = expr_of(spec) {
        return one(e).map(OdsSeries::into_value);
    }
    let items = named("transform", spec)?;
    let mut pairs = Vec::with_capacity(items.len());
    for (name, v) in items {
        let e = expr_of(&v).ok_or_else(|| {
            format!(
                "ods.transform: '{}' must be an aggregation expression, got {}",
                name,
                v.type_name()
            )
        })?;
        pairs.push((name, one(e)?));
    }
    Frame::new(pairs).map(frame_value).map_err(err)
}

/// `ods.slice(x, start, n)`: `n` rows from `start` (a negative start
/// counts from the end); a Frame or a Series.
fn slice(args: &[Value]) -> Result<Value, String> {
    let len = match args.first() {
        Some(v) => match (frame_of(v), series_of(v)) {
            (Some(f), _) => f.n_rows(),
            (_, Some(s)) => s.len(),
            _ => {
                return Err(format!(
                    "ods.slice: argument 1 must be a Frame or a Series, got {}",
                    v.type_name()
                ));
            }
        },
        None => return Err("ods.slice expects a Frame or Series, a start and a length".to_string()),
    };
    let start = want_int("slice", "start", args.get(1))?;
    let start = if start < 0 {
        (len as i64 + start).max(0) as usize
    } else {
        (start as usize).min(len)
    };
    let n = match arg(args, 2) {
        None => len - start,
        v => {
            let n = want_int("slice", "the length", v)?;
            if n < 0 {
                return Err(format!(
                    "ods.slice: the length cannot be negative, got {}",
                    n
                ));
            }
            (n as usize).min(len - start)
        }
    };
    let idx = Series::from_i64((start..start + n).map(|i| i as i64).collect());
    let v = &args[0];
    match frame_of(v) {
        Some(f) => f.take(&idx).map(frame_value).map_err(err),
        None => series_of(v)
            .expect("checked")
            .take(&idx)
            .map(OdsSeries::into_value)
            .map_err(err),
    }
}

/// `ods.row(f, i)`: row `i` (negative from the end) as a map of column
/// to value.
fn row(args: &[Value]) -> Result<Value, String> {
    let f = want_frame("row", args, 0)?;
    let i = want_int("row", "the row", args.get(1))?;
    let n = f.n_rows() as i64;
    let at = if i < 0 { n + i } else { i };
    if at < 0 || at >= n {
        return Err(format!(
            "ods.row: row {} is out of range for a Frame of {} rows",
            i, n
        ));
    }
    let mut m = ValueMap::default();
    for (name, col) in f.names().iter().zip(f.columns()) {
        m.insert(name.clone(), scalar_to_value(col.scalar_at(at as usize)));
    }
    Ok(Value::Map(Arc::new(m)))
}

/// `ods.lookup(f, key_col, key, value_col)`: the `value_col` of the first
/// row whose `key_col` is `key`, or `()`. A list or Series of keys gives
/// a Series (a null where a key is missing). The hash over `key_col` is
/// built once per Frame.
fn lookup(args: &[Value]) -> Result<Value, String> {
    let of = args.first().and_then(ods_frame_of).ok_or_else(|| {
        format!(
            "ods.lookup: argument 1 must be a Frame, got {}",
            args.first().map(|v| v.type_name()).unwrap_or_default()
        )
    })?;
    let key_col = want_str("lookup", "the key column", args.get(1))?;
    let value_col = want_str("lookup", "the value column", args.get(3))?;
    let values = of.0.column(&value_col).map_err(|_| {
        format!(
            "ods.lookup: no column '{}' in this Frame. It has: {}",
            value_col,
            of.0.names().join(", ")
        )
    })?;
    let ix = of.key_index(&key_col)?;
    let key = args
        .get(2)
        .ok_or("ods.lookup expects a Frame, a key column, a key and a value column")?;
    let many: Option<Vec<Scalar>> = match (series_of(key), key) {
        (Some(s), _) => Some((0..s.len()).map(|i| s.scalar_at(i)).collect()),
        (_, Value::List(items)) => Some(
            items
                .iter()
                .map(|v| {
                    value_to_scalar(v).ok_or_else(|| {
                        format!(
                            "ods.lookup: a key is a number, String or Bool, got {}",
                            v.type_name()
                        )
                    })
                })
                .collect::<Result<_, _>>()?,
        ),
        _ => None,
    };
    match many {
        Some(keys) => {
            let idx: Vec<Option<usize>> = keys.iter().map(|k| ix.get(k)).collect();
            Ok(OdsSeries::into_value(olang_ods::group::take_opt(
                values, &idx,
            )))
        }
        None => {
            let k = value_to_scalar(key).ok_or_else(|| {
                format!(
                    "ods.lookup: the key is a number, String or Bool, got {}",
                    key.type_name()
                )
            })?;
            Ok(ix
                .get(&k)
                .map(|r| scalar_to_value(values.scalar_at(r)))
                .unwrap_or(Value::Unit))
        }
    }
}

/// `ods.crosstab(a, b, values?, agg = "count", normalize?, name?)`.
fn crosstab(args: &[Value]) -> Result<Value, String> {
    let a = want_series("crosstab", args, 0)?;
    let b = want_series("crosstab", args, 1)?;
    let values = match arg(args, 2) {
        None => None,
        Some(v) => Some(series_of(v).ok_or_else(|| {
            format!(
                "ods.crosstab: values must be a Series, got {}",
                v.type_name()
            )
        })?),
    };
    let op = match arg(args, 3) {
        None => AggOp::Count,
        Some(Value::String(name)) => AggOp::parse(name).ok_or_else(|| {
            format!(
                "ods.crosstab: '{}' is not an aggregation. Expected one of {}",
                name,
                AggOp::NAMES.join(", ")
            )
        })?,
        Some(other) => {
            return Err(format!(
                "ods.crosstab: agg must be a String, got {}",
                other.type_name()
            ));
        }
    };
    if values.is_none() && op != AggOp::Count {
        return Err(format!(
            "ods.crosstab: agg \"{}\" needs values to aggregate",
            op.name()
        ));
    }
    let normalize = match arg(args, 4) {
        None | Some(Value::Boolean(false)) => Normalize::None,
        Some(Value::Boolean(true)) => Normalize::All,
        Some(Value::String(s)) => match s.as_str() {
            "all" => Normalize::All,
            "row" | "rows" | "index" => Normalize::Rows,
            "column" | "columns" => Normalize::Columns,
            other => {
                return Err(format!(
                    "ods.crosstab: normalize is \"all\", \"rows\" or \"columns\" (or true), got \"{}\"",
                    other
                ));
            }
        },
        Some(other) => {
            return Err(format!(
                "ods.crosstab: normalize must be a String or Bool, got {}",
                other.type_name()
            ));
        }
    };
    let name = match arg(args, 5) {
        None => "row".to_string(),
        v => want_str("crosstab", "name", v)?,
    };
    olang_ods::group::crosstab(
        a,
        b,
        values,
        op,
        normalize,
        &name,
        &crate::ast::format_float,
    )
    .map(frame_value)
    .map_err(|e| format!("ods.{}", e))
}

/// `ods.run_id(s)` or `ods.run_id([s1, s2, …])`.
fn run_id(args: &[Value]) -> Result<Value, String> {
    let cols: Vec<&Series> = match args.first() {
        Some(Value::List(items)) => items
            .iter()
            .map(|v| {
                series_of(v).ok_or_else(|| {
                    format!(
                        "ods.run_id: each column must be a Series, got {}",
                        v.type_name()
                    )
                })
            })
            .collect::<Result<_, _>>()?,
        _ => vec![want_series("run_id", args, 0)?],
    };
    olang_ods::group::run_id(&cols)
        .map(OdsSeries::into_value)
        .map_err(|e| format!("ods.{}", e))
}

/// `ods.rolling(s, window, agg, center = false, min_periods?, exact = false)`.
fn rolling(args: &[Value]) -> Result<Value, String> {
    let s = want_series("rolling", args, 0)?;
    let window = match args.get(1) {
        Some(Value::Integer(w)) if *w >= 1 => *w as usize,
        Some(Value::Integer(w)) => {
            return Err(format!(
                "ods.rolling: the window must be at least 1, got {}",
                w
            ));
        }
        other => {
            return Err(format!(
                "ods.rolling: the window must be an Int, got {}",
                other
                    .map(|v| v.type_name())
                    .unwrap_or_else(|| "nothing".to_string())
            ));
        }
    };
    let op_name = want_str("rolling", "the aggregation", args.get(2))?;
    let op = AggOp::parse(&op_name).ok_or_else(|| {
        format!(
            "ods.rolling: '{}' is not an aggregation. Expected one of \
             count, sum, mean, min, max, median, std, var",
            op_name
        )
    })?;
    let min_periods = match arg(args, 4) {
        None => None,
        v => {
            let m = want_int("rolling", "min_periods", v)?;
            if m < 0 {
                return Err(format!(
                    "ods.rolling: min_periods cannot be negative, got {}",
                    m
                ));
            }
            Some(m as usize)
        }
    };
    let opts = Rolling {
        center: want_bool("rolling", "center", args.get(3), false)?,
        min_periods,
        exact: want_bool("rolling", "exact", args.get(5), false)?,
    };
    s.rolling_with(window, op, opts)
        .map(OdsSeries::into_value)
        .map_err(err)
}

/// `ods.join(left, right, on, right_on?, how = "inner", suffix = "_right",
/// validate?)`, and the four named joins (their `how` fixed).
fn join(func: &str, args: &[Value]) -> Result<Value, String> {
    let l = want_frame(func, args, 0)?;
    let r = want_frame(func, args, 1)?;
    let left_on = match args.get(2) {
        Some(v) => names_of(func, "on", v)?,
        None => return Err(format!("ods.{}: needs the key column(s) to join on", func)),
    };
    let right_on = match arg(args, 3) {
        None => left_on.clone(),
        Some(v) => names_of(func, "right_on", v)?,
    };
    // The named joins take (left, right, on, right_on?, suffix, validate);
    // `join` has `how` before them.
    let (how, rest) = match func {
        "join" => {
            let how = match arg(args, 4) {
                None => JoinHow::Inner,
                Some(Value::String(h)) => match h.as_str() {
                    "inner" => JoinHow::Inner,
                    "left" => JoinHow::Left,
                    "full" | "outer" => JoinHow::Full,
                    "semi" => JoinHow::Semi,
                    "anti" => JoinHow::Anti,
                    other => {
                        return Err(format!(
                            "ods.join: how is \"inner\", \"left\", \"full\", \"semi\" or \"anti\", got \"{}\"",
                            other
                        ));
                    }
                },
                Some(other) => {
                    return Err(format!(
                        "ods.join: how must be a String, got {}",
                        other.type_name()
                    ));
                }
            };
            (how, 5)
        }
        "join_left" => (JoinHow::Left, 4),
        "join_full" => (JoinHow::Full, 4),
        "join_semi" => (JoinHow::Semi, 4),
        _ => (JoinHow::Anti, 4),
    };
    if args.len() > rest + 2 {
        return Err(format!("ods.{}: too many arguments ({})", func, args.len()));
    }
    let suffix = match arg(args, rest) {
        None => "_right".to_string(),
        v => want_str(func, "suffix", v)?,
    };
    let validate = match arg(args, rest + 1) {
        None => Validate::None,
        Some(Value::String(v)) => Validate::parse(v).ok_or_else(|| {
            format!(
                "ods.{}: validate is \"one_to_one\", \"many_to_one\", \"one_to_many\" or \"many_to_many\", got \"{}\"",
                func, v
            )
        })?,
        Some(other) => return Err(format!("ods.{}: validate must be a String, got {}", func, other.type_name())),
    };
    for (side, f, keys) in [("left", l, &left_on), ("right", r, &right_on)] {
        for k in keys.iter() {
            if f.column(k).is_err() {
                return Err(format!(
                    "ods.{}: no column '{}' in the {} Frame. It has: {}",
                    func,
                    k,
                    side,
                    f.names().join(", ")
                ));
            }
        }
    }
    l.join_by(
        r,
        &JoinSpec {
            left_on: &left_on,
            right_on: &right_on,
            how,
            suffix: &suffix,
            validate,
        },
    )
    .map(frame_value)
    .map_err(|e| format!("ods.{}", e))
}

fn pivot_fill(args: &[Value]) -> Result<Value, String> {
    let fill = value_to_scalar(&args[5]).ok_or_else(|| {
        format!(
            "ods.pivot: fill must be a number, String or Bool, got {}",
            args[5].type_name()
        )
    })?;
    let index = string_list("pivot", args, 1)?;
    let out = super::call("pivot", args[..5].to_vec())?;
    let f = frame_of(&out).expect("pivot gives a Frame");
    let mut pairs = Vec::with_capacity(f.n_cols());
    for (name, col) in f.names().iter().zip(f.columns()) {
        let col = if index.contains(name) {
            col.clone()
        } else {
            let fill = match (&fill, col.dtype()) {
                (Scalar::I64(x), DType::F64) => Scalar::F64(*x as f64),
                (s, _) => s.clone(),
            };
            col.clone()
                .fill_null(fill)
                .map_err(|e| format!("ods.pivot: fill: {}", e))?
        };
        pairs.push((name.clone(), col));
    }
    Frame::new(pairs).map(frame_value).map_err(err)
}

fn rename_object(args: &[Value]) -> Result<Value, String> {
    let entries = expr::entries(&args[1]).expect("an object");
    let mut m = ValueMap::default();
    for (k, v) in entries {
        m.insert(k, v);
    }
    super::call("rename", vec![args[0].clone(), Value::Map(Arc::new(m))])
}

/// `ods.frame({ a: s, b: [1, 2] })`: one column per entry, in order.
fn frame_object(args: &[Value]) -> Result<Value, String> {
    let pairs: Vec<Value> = expr::entries(&args[0])
        .expect("an object or map")
        .into_iter()
        .map(|(k, v)| Value::List(Arc::new(vec![Value::String(Arc::new(k)), v])))
        .collect();
    super::call("frame", vec![Value::List(Arc::new(pairs))])
}

/// `ods.resample(f, on, every, aggs, by?)`: the rows grouped by the start
/// of their time bucket (and `by`'s keys), aggregated, sorted by the keys
/// and then by time.
fn resample(args: &[Value]) -> Result<Value, String> {
    let f = want_frame("resample", args, 0)?;
    let on = want_str("resample", "on (the time column)", args.get(1))?;
    let every = want_str("resample", "every (the interval)", args.get(2))?;
    let every = Every::parse(&every).map_err(|e| format!("ods.resample: {}", e))?;
    let spec = args
        .get(3)
        .filter(|v| !matches!(v, Value::Unit))
        .ok_or("ods.resample expects the aggregations, like { total: col(\"x\").sum() }")?;
    let by = match arg(args, 4) {
        None => Vec::new(),
        Some(v) => names_of("resample", "by", v)?,
    };
    let _ = &by;
    let time = f.column(&on).map_err(|_| {
        format!(
            "ods.resample: no column '{}' in this Frame. It has: {}",
            on,
            f.names().join(", ")
        )
    })?;
    // Group on each row's bucket as an Int (seconds since the epoch):
    // no text per row, and the sort is an integer sort. The buckets are
    // written as text once each, at the end.
    let (codes, shape) = olang_ods::time::buckets(time, every)
        .map_err(|e| format!("ods.resample: '{}': {}", on, e))?;
    const BUCKET: &str = "\u{0}bucket";
    let f = f.with_column(BUCKET, codes).map_err(err)?;
    let mut keys = by.clone();
    keys.push(BUCKET.to_string());
    let grouped = new_group_by(&f, keys)?;
    let out = grouped.agg("resample", spec)?;
    let out_f = frame_of(&out).expect("agg gives a Frame");
    let key_cols: Vec<&Series> = grouped
        .keys
        .iter()
        .map(|k| out_f.column(k).map_err(err))
        .collect::<Result<_, _>>()?;
    let sort: Vec<(&Series, bool)> = key_cols.into_iter().map(|c| (c, false)).collect();
    let sorted = out_f.sort_by_keys(&sort).map_err(err)?;
    let mut pairs = Vec::with_capacity(sorted.n_cols());
    for (name, col) in sorted.names().iter().zip(sorted.columns()) {
        if name == BUCKET {
            pairs.push((
                on.clone(),
                olang_ods::time::format_buckets(col, every, shape),
            ));
        } else {
            pairs.push((name.clone(), col.clone()));
        }
    }
    Frame::new(pairs)
        .map(frame_value)
        .map_err(|e| format!("ods.resample: {}", e))
}

/// `ods.asof_join(left, right, on, by?, direction = "backward",
/// tolerance?, suffix = "_right")`.
fn asof_join(args: &[Value]) -> Result<Value, String> {
    let l = want_frame("asof_join", args, 0)?;
    let r = want_frame("asof_join", args, 1)?;
    let on = want_str("asof_join", "on", args.get(2))?;
    let by = match arg(args, 3) {
        None => Vec::new(),
        Some(v) => names_of("asof_join", "by", v)?,
    };
    let direction = match arg(args, 4) {
        None => AsOf::Backward,
        Some(Value::String(d)) => AsOf::parse(d).ok_or_else(|| {
            format!(
                "ods.asof_join: direction is \"backward\", \"forward\" or \"nearest\", got \"{}\"",
                d
            )
        })?,
        Some(other) => {
            return Err(format!(
                "ods.asof_join: direction must be a String, got {}",
                other.type_name()
            ));
        }
    };
    let tolerance = match arg(args, 5) {
        None => None,
        Some(Value::Float(t)) => Some(*t),
        Some(Value::Integer(t)) => Some(*t as f64),
        Some(other) => {
            return Err(format!(
                "ods.asof_join: tolerance must be a number, got {}",
                other.type_name()
            ));
        }
    };
    let suffix = match arg(args, 6) {
        None => "_right".to_string(),
        v => want_str("asof_join", "suffix", v)?,
    };
    for (side, f) in [("left", l), ("right", r)] {
        for k in std::iter::once(&on).chain(by.iter()) {
            if f.column(k).is_err() {
                return Err(format!(
                    "ods.asof_join: no column '{}' in the {} Frame. It has: {}",
                    k,
                    side,
                    f.names().join(", ")
                ));
            }
        }
    }
    l.asof_join(r, &on, &by, direction, tolerance, &suffix)
        .map(frame_value)
        .map_err(|e| format!("ods.{}", e))
}

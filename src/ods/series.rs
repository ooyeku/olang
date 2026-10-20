//! Series: the language-facing face of the olang-ods engine.
//!
//! Everything here is translation — olang `Value`s to engine `Scalar`s and
//! back, argument checking, and parallelism policy (the configured
//! threshold via `crate::parallel::should_parallelize`, and the engine's
//! per-class size via `olang_ods::par_worth`: see `par`). No numeric logic
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
use olang_ods::{AggOp, CmpOp, DType, RankMethod, Scalar, Series};
use std::any::Any;

#[derive(Debug)]
pub struct OdsSeries {
    /// The materialized series. Empty only while a fused expression is
    /// pending; every access goes through `series()`, which forces.
    cell: std::sync::OnceLock<Series>,
    /// The pending expression, when this value was built lazily from
    /// operators or column functions that cannot raise (see
    /// `expr::build`).
    pending: Option<Pending>,
}

#[derive(Debug)]
struct Pending {
    expr: std::sync::Arc<olang_ods::fuse::Expr>,
    len: usize,
    depth: usize,
    dtype: DType,
    /// The pending Series it was built from (`expr::force` keeps the
    /// costly ones' columns when it evaluates this).
    deps: Vec<Value>,
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

    /// A lazy Series: `expr` over `len` rows, of type `dtype`, `depth`
    /// operations deep. Only expressions that cannot raise are deferred.
    pub fn pending_value(
        expr: std::sync::Arc<olang_ods::fuse::Expr>,
        len: usize,
        depth: usize,
        dtype: DType,
        deps: Vec<Value>,
    ) -> Value {
        Value::Native(NativeHandle::new(OdsSeries {
            cell: std::sync::OnceLock::new(),
            pending: Some(Pending {
                expr,
                len,
                depth,
                dtype,
                deps,
            }),
        }))
    }

    /// Keep a column computed for this pending Series elsewhere (in the
    /// plan of an expression that read it).
    pub fn keep(&self, s: Series) {
        let _ = self.cell.set(s);
    }

    /// The pending expression, its length and depth — None once the
    /// value has been materialized (then the column is what to read).
    pub fn pending(&self) -> Option<(&std::sync::Arc<olang_ods::fuse::Expr>, usize, usize)> {
        if self.cell.get().is_some() {
            return None;
        }
        self.pending.as_ref().map(|p| (&p.expr, p.len, p.depth))
    }

    /// The dtype, without forcing a pending expression.
    pub fn dtype(&self) -> DType {
        match (self.cell.get(), &self.pending) {
            (Some(s), _) => s.dtype(),
            (None, Some(p)) => p.dtype,
            (None, None) => unreachable!("an OdsSeries is a column or an expression"),
        }
    }

    /// The materialized series, forcing a pending expression on first
    /// use. A pending expression cannot raise (only those are deferred)
    /// and evaluates to what the eager kernels give, so nothing
    /// observable depends on when this runs.
    pub fn series(&self) -> &Series {
        self.cell.get_or_init(|| {
            let p = self
                .pending
                .as_ref()
                .expect("an unmaterialized OdsSeries always carries its expression");
            super::expr::force(&p.expr, p.len, &p.deps)
                .expect("a deferred expression cannot raise")
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
                // A negative position is out of range, as a list's is.
                let pos = *i;
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
pub(super) fn mask_list<'a>(func: &str, args: &'a [Value]) -> Result<Vec<&'a Series>, String> {
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

/// The long-standing checks on a list of masks (each a Bool Series, of
/// one length), with their words.
pub(super) fn check_masks(func: &str, items: &[Value]) -> Result<(), String> {
    mask_list(func, &[Value::List(std::sync::Arc::new(items.to_vec()))]).map(|_| ())
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

/// `ods.is_in(s, values)`: true where the element equals one of `values`
/// (a list or a Series). A number matches a number of the other kind by
/// value (`2` is `2.0`); a value of another kind than the column's is an
/// error naming it. A null element is unknown (null), as a comparison
/// with it is — unless `values` holds `()`, which keeps nulls (true).
fn is_in(s: &Series, values: &Value) -> Result<Series, String> {
    use std::collections::HashSet;
    let wanted: Vec<Scalar> = if let Some(v) = series_of(values) {
        (0..v.len()).map(|i| v.scalar_at(i)).collect()
    } else {
        match values {
            Value::List(items) => items
                .iter()
                .enumerate()
                .map(|(i, x)| {
                    value_to_scalar(x).ok_or_else(|| {
                        format!(
                            "ods.is_in: value {} must be a number, a String, a Bool or (), got {}",
                            i + 1,
                            x.type_name()
                        )
                    })
                })
                .collect::<Result<_, _>>()?,
            other => {
                return Err(format!(
                    "ods.is_in: argument 2 must be a list or a Series of values, got {}",
                    other.type_name()
                ));
            }
        }
    };
    let keeps_null = wanted.iter().any(|w| matches!(w, Scalar::Null));
    let kind_of = |w: &Scalar| match w {
        Scalar::F64(_) | Scalar::I64(_) => "a number",
        Scalar::Str(_) => "a String",
        Scalar::Bool(_) => "a Bool",
        Scalar::Null => "()",
    };
    let fits = |w: &Scalar| {
        matches!(
            (s.dtype(), w),
            (_, Scalar::Null)
                | (DType::F64 | DType::I64, Scalar::F64(_) | Scalar::I64(_))
                | (DType::Str, Scalar::Str(_))
                | (DType::Bool, Scalar::Bool(_))
        )
    };
    if let Some(bad) = wanted.iter().find(|w| !fits(w)) {
        return Err(format!(
            "ods.is_in: {} cannot be in a Series of {}",
            kind_of(bad),
            s.dtype()
        ));
    }
    let n = s.len();
    let mut out: Vec<Option<bool>> = Vec::with_capacity(n);
    let null_answer = if keeps_null { Some(true) } else { None };
    match s {
        Series::Str { col, validity } => {
            let set: HashSet<&str> = wanted
                .iter()
                .filter_map(|w| match w {
                    Scalar::Str(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect();
            for (i, x) in col.iter().enumerate() {
                let valid = validity.as_ref().map(|b| b.get(i)).unwrap_or(true);
                out.push(if valid {
                    Some(set.contains(x))
                } else {
                    null_answer
                });
            }
        }
        Series::I64 { values, validity } => {
            let ints: HashSet<i64> = wanted
                .iter()
                .filter_map(|w| match w {
                    Scalar::I64(v) => Some(*v),
                    Scalar::F64(f) if f.fract() == 0.0 && f.abs() < 9.0e18 => Some(*f as i64),
                    _ => None,
                })
                .collect();
            for (i, x) in values.iter().enumerate() {
                let valid = validity.as_ref().map(|b| b.get(i)).unwrap_or(true);
                out.push(if valid {
                    Some(ints.contains(x))
                } else {
                    null_answer
                });
            }
        }
        _ => {
            let nums: Vec<f64> = wanted
                .iter()
                .filter_map(|w| match w {
                    Scalar::F64(v) => Some(*v),
                    Scalar::I64(v) => Some(*v as f64),
                    _ => None,
                })
                .collect();
            let bools: Vec<bool> = wanted
                .iter()
                .filter_map(|w| match w {
                    Scalar::Bool(b) => Some(*b),
                    _ => None,
                })
                .collect();
            for i in 0..n {
                out.push(match s.scalar_at(i) {
                    Scalar::F64(x) => Some(nums.contains(&x)),
                    Scalar::Bool(b) => Some(bools.contains(&b)),
                    _ => null_answer,
                });
            }
        }
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
pub(crate) fn series_from_list(items: &[Value]) -> Result<Series, String> {
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

/// Whether a kernel of `class` over `len` elements fans out across the
/// pool: the host's switch and threshold allow it (`set_parallel`,
/// `OLANG_PARALLELISM`), and the engine finds the work large enough for
/// its class to pay for the hand-off (`olang_ods::ParClass`).
pub fn par(class: olang_ods::ParClass, len: usize) -> bool {
    crate::parallel::should_parallelize(len) && olang_ods::par_worth(class, len)
}

fn par_for(class: olang_ods::ParClass, s: &Series) -> bool {
    par(class, s.len())
}

// ---------------------------------------------------------------------
// Namespace and dispatch
// ---------------------------------------------------------------------

/// (name, arity) of every series function in the `ods` namespace.
pub const FUNCTIONS: &[(&str, usize)] = &[
    ("series", 1),
    ("try_series", 1),
    ("date_part", 2),
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
    ("sort", 2),
    ("argsort", 2),
    ("take", 2),
    ("filter", 2),
    ("eq", 2),
    ("all_of", 1),
    ("any_of", 1),
    ("not", 1),
    ("ne", 2),
    ("is_in", 2),
    ("contains", 2),
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
    // A trailing `descending` is optional.
    if matches!(func, "sort" | "argsort") && args.len() + 1 == expected {
        args.push(Value::Boolean(false));
    }
    arity(func, &args, expected)?;
    let e = |err: olang_ods::OdsError| err.to_string();
    match func {
        // The Result form of `series`: a mixed-type list is an
        // `Err(message)` to match on, not a raise.
        // The derived group key for dates: the "year", "month", or "day"
        // prefix of an ISO date or timestamp column, as a String Series —
        // `ods.group_by(f, ods.date_part(f["date"], "month"), …)` without
        // a two-key group and two sorts.
        "date_part" => {
            let s = want_series(func, &args, 0)?;
            let unit = match &args[1] {
                Value::String(u) => u.to_string(),
                other => {
                    return Err(format!(
                        "ods.date_part: unit must be \"year\", \"month\", or \"day\", got {}",
                        other.type_name()
                    ));
                }
            };
            let width = match unit.as_str() {
                "year" => 4,
                "month" => 7,
                "day" => 10,
                other => {
                    return Err(format!(
                        "ods.date_part: unit must be \"year\", \"month\", or \"day\", got \"{}\"",
                        other
                    ));
                }
            };
            let mut out: Vec<Option<String>> = Vec::with_capacity(s.len());
            for i in 0..s.len() {
                match scalar_to_value(s.get(i as i64).map_err(e)?) {
                    Value::String(ref text) => {
                        if text.len() < width {
                            return Err(format!(
                                "ods.date_part: {:?} is not an ISO date (row {})",
                                text.as_str(),
                                i
                            ));
                        }
                        out.push(Some(text[..width].to_string()));
                    }
                    Value::Unit => out.push(None),
                    other => {
                        return Err(format!(
                            "ods.date_part: expects a String Series of ISO dates, got {}",
                            other.type_name()
                        ));
                    }
                }
            }
            Ok(OdsSeries::into_value(Series::from_str_options(out)))
        }
        "try_series" => Ok(match &args[0] {
            Value::List(items) => match series_from_list(items) {
                Ok(series) => Value::Ok(Box::new(OdsSeries::into_value(series))),
                Err(e) => Value::Err(Box::new(Value::String(std::sync::Arc::new(e.to_string())))),
            },
            other => Value::Err(Box::new(Value::String(std::sync::Arc::new(format!(
                "ods.try_series expects a list, got {}",
                other.type_name()
            ))))),
        }),
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
            s.sum(par_for(olang_ods::ParClass::Reduce, s)).map(scalar_to_value).map_err(e)
        }
        "mean" | "var" | "std" => {
            let s = want_series(func, &args, 0)?;
            let par = par_for(olang_ods::ParClass::Reduce, s);
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
                     count, sum, mean, min, max, median, std, var",
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
            a.dot(b, par_for(olang_ods::ParClass::Reduce, a)).map(Value::Float).map_err(e)
        }
        "sort" | "argsort" => {
            let s = want_series(func, &args, 0)?;
            let descending = match &args[1] {
                Value::Boolean(b) => *b,
                other => {
                    return Err(format!(
                        "ods.{}: descending must be a Bool, got {}",
                        func,
                        other.type_name()
                    ));
                }
            };
            if func == "sort" {
                s.sort_by_dir(descending, par_for(olang_ods::ParClass::Sort, s))
            } else {
                s.argsort_by(descending)
            }
            .map(OdsSeries::into_value)
            .map_err(e)
        }
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
        // Membership: the mask "is one of these", which a chain of `eq`s
        // under `any_of` spells badly once there are more than two.
        "is_in" => {
            let s = want_series(func, &args, 0)?;
            Ok(OdsSeries::into_value(is_in(s, &args[1])?))
        }
        "contains" => {
            let s = want_series(func, &args, 0)?;
            let text = match &args[1] {
                Value::String(t) => t.as_str(),
                other => {
                    return Err(format!(
                        "ods.contains: argument 2 must be a String, got {}",
                        other.type_name()
                    ));
                }
            };
            let Series::Str { col, validity } = s else {
                return Err(format!(
                    "ods.contains: argument 1 must be a String Series, got a {} Series",
                    s.dtype()
                ));
            };
            let bits: Vec<bool> = col.iter().map(|x| x.contains(text)).collect();
            Ok(OdsSeries::into_value(Series::Bool {
                values: std::sync::Arc::new(bits),
                validity: validity.clone(),
            }))
        }
        _ => unreachable!("dispatch() checked membership"),
    }
}

// ---------------------------------------------------------------------
// Operators
// ---------------------------------------------------------------------

/// Whether `v` is a String series, answered without forcing it.
fn is_str_series(v: &Value) -> bool {
    let Value::Native(h) = v else { return false };
    h.0.as_any()
        .downcast_ref::<OdsSeries>()
        .is_some_and(|s| s.dtype() == DType::Str)
}

fn is_series(v: &Value) -> bool {
    matches!(v, Value::Native(h) if h.0.as_any().downcast_ref::<OdsSeries>().is_some())
}

pub fn binary_op(
    op: &crate::ast::BinaryOp,
    lhs: &Value,
    rhs: &Value,
) -> Option<Result<Value, String>> {
    use crate::ast::BinaryOp as B;
    use olang_ods::fuse::{BinOp, Op};
    let bop = match op {
        B::Add => BinOp::Add,
        B::Subtract => BinOp::Sub,
        B::Multiply => BinOp::Mul,
        B::Divide => BinOp::Div,
        B::Modulo => BinOp::Mod,
        B::LessThan => BinOp::Lt,
        B::LessThanEqual => BinOp::Le,
        B::GreaterThan => BinOp::Gt,
        B::GreaterThanEqual => BinOp::Ge,
        B::Equal => BinOp::Eq,
        B::NotEqual => BinOp::Ne,
        _ => return None,
    };
    // An Expr on either side makes an Expr.
    if let Some(r) = super::expr::binary_op(bop, lhs, rhs) {
        return Some(r);
    }
    // `+` with a String series on a side joins text: `s["a"] + "|" +
    // s["b"]`, a String on every row or a String series alongside.
    if matches!(op, B::Add) && (is_str_series(lhs) || is_str_series(rhs)) {
        let joined = match (series_of(lhs), series_of(rhs), lhs, rhs) {
            (Some(a), Some(b), _, _) => a.concat_str(b),
            (Some(a), None, _, Value::String(t)) => a.concat_str_scalar(t, false),
            (None, Some(b), Value::String(t), _) => b.concat_str_scalar(t, true),
            (Some(_), None, _, other) | (None, Some(_), other, _) => {
                return Some(Err(format!(
                    "+ joins a String series to a String or a String series, not a {} — \
                     make it text first (`ods.cast(s, \"String\")`, `str(x)`)",
                    other.type_name()
                )));
            }
            (None, None, _, _) => return None,
        };
        return Some(joined.map(OdsSeries::into_value).map_err(|e| e.to_string()));
    }
    let (ls, rs) = (is_series(lhs), is_series(rhs));
    if !(ls || rs) {
        return None;
    }
    // Series == Series stays structural (the registry's native_eq
    // fallback); against a scalar there is no structural reading, so
    // ==/!= make masks like the ordered comparisons.
    if matches!(op, B::Equal | B::NotEqual) && ls && rs {
        return None;
    }
    // A null scalar has no elementwise reading in an operator (it would
    // null the whole column): the eager kernels refused it, and still do.
    if matches!(lhs, Value::Unit) || matches!(rhs, Value::Unit) {
        let side = if ls { series_of(lhs) } else { series_of(rhs) }.expect("a series side");
        return Some(Err(if matches!(bop, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod) {
            "cannot apply arithmetic with a null scalar".to_string()
        } else {
            format!("cannot compare Series[{}] with ()", side.dtype())
        }));
    }
    let a = super::expr::operand(lhs)?;
    let b = super::expr::operand(rhs)?;
    // Elementwise work is one fused expression: `a * b + 1.0` is
    // evaluated once, in one pass, when first observed; an operation
    // that can raise (a division, Int arithmetic) is evaluated at once,
    // so its error is raised here.
    Some(super::expr::build("", Op::Bin(bop), vec![a, b]))
}

/// `-s` and `!mask` on a Series or an Expr (the native unary operator
/// hook, L3): `-` negates (Int checked), `!` flips a Bool mask with nulls
/// staying null.
#[allow(dead_code)] // reached once the unary hook (L3) calls it
pub fn unary_op(op: &crate::ast::UnaryOp, v: &Value) -> Option<Result<Value, String>> {
    use olang_ods::fuse::Op;
    if !(is_series(v) || super::expr::expr_of(v).is_some()) {
        return None;
    }
    let o = match op {
        crate::ast::UnaryOp::Negate => Op::Neg,
        crate::ast::UnaryOp::Not => Op::Not,
    };
    let a = super::expr::operand(v)?;
    Some(super::expr::build("", o, vec![a]))
}

/// `a & b`, `a | b`, `a ^ b` between Bool masks (Series or Exprs, a
/// Bool on either side broadcasting): Kleene logic, so `false & null` is
/// false and `true | null` is true. Shifts have no column reading.
#[allow(dead_code)] // reached once the bitwise hook (L3) calls it
pub fn bitwise_op(
    op: &crate::ast::BitwiseOp,
    lhs: &Value,
    rhs: &Value,
) -> Option<Result<Value, String>> {
    use crate::ast::BitwiseOp as W;
    use olang_ods::fuse::{BinOp, Op};
    let o = match op {
        W::And => BinOp::And,
        W::Or => BinOp::Or,
        W::Xor => BinOp::Xor,
        W::Shl | W::Shr => return None,
    };
    let col = |v: &Value| is_series(v) || super::expr::expr_of(v).is_some();
    if !(col(lhs) || col(rhs)) {
        return None;
    }
    let a = super::expr::operand(lhs)?;
    let b = super::expr::operand(rhs)?;
    Some(super::expr::build("", Op::Bin(o), vec![a, b]))
}

/// Convenience used by tests: wrap a series value.
pub fn make_series_value(series: Series) -> Value {
    OdsSeries::into_value(series)
}

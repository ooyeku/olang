//! Expressions: `ods.col`, the elementwise builders, and the Frame verbs
//! that resolve them (`with`, `filter`, `select`, `sort_by`, `over`).
//!
//! Two layers meet here. The engine's `fuse::Expr` is concrete: columns
//! and scalars, compiled into one chunked plan. An `Expr` value (this
//! file's `LExpr`) is the language's: it names columns, and resolves
//! against a Frame into the engine's. Every elementwise column function
//! builds either one: given Series and scalars it builds the engine's
//! (a lazy Series, evaluated in one fused pass when first observed, or
//! at once when the operation can raise); given an Expr anywhere it
//! builds an Expr. A function that is not elementwise (`rank`, `shift`,
//! `mean`, …) given an Expr becomes a call node, evaluated against the
//! Frame when the Expr is.

use super::frame::{OdsFrame, frame_of};
use super::series::{OdsSeries, scalar_to_value, series_from_list, series_of, value_to_scalar};
use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use olang_ods::fuse::{self, BinOp, Expr, Op, Plan, Reduce};
use olang_ods::{AggOp, DType, Frame, Scalar, Series};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// A column expression, unresolved: it names columns.
#[derive(Debug)]
pub enum LExpr {
    /// `ods.col(name)`
    Col(String),
    /// A Series standing where an Expr goes: a literal column.
    Series(Series),
    /// A scalar, broadcast.
    Lit(Scalar),
    /// An elementwise operation, fused with its neighbours.
    Op(Op, Vec<Arc<LExpr>>),
    /// Any other `ods` function, called with its Expr arguments
    /// evaluated against the Frame.
    Call(String, Vec<LArg>),
    /// An aggregation within groups, given back to every row.
    Over(Arc<LExpr>, Vec<String>),
    /// The same expression, under a name.
    Alias(Arc<LExpr>, String),
}

/// An argument of a call node.
#[derive(Debug, Clone)]
pub enum LArg {
    E(Arc<LExpr>),
    V(Value),
    L(Vec<LArg>),
}

/// The `Expr` value.
#[derive(Debug)]
pub struct OdsExpr(pub Arc<LExpr>);

impl OdsExpr {
    pub fn into_value(e: LExpr) -> Value {
        Value::Native(NativeHandle::new(OdsExpr(Arc::new(e))))
    }
}

impl NativeObject for OdsExpr {
    fn module(&self) -> &'static str {
        "ods"
    }

    fn type_name(&self) -> &'static str {
        "Expr"
    }

    fn display(&self) -> String {
        render(&self.0)
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<OdsExpr>()
            .is_some_and(|o| render(&o.0) == render(&self.0))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn expr_of(v: &Value) -> Option<&Arc<LExpr>> {
    match v {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsExpr>().map(|e| &e.0),
        _ => None,
    }
}

/// Whether an argument list holds an Expr, at the top or in a list.
pub fn holds_expr(args: &[Value]) -> bool {
    args.iter().any(|a| match a {
        Value::List(items) => holds_expr(items),
        v => expr_of(v).is_some(),
    })
}

fn op_symbol(b: BinOp) -> &'static str {
    match b {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Pow => "pow",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "&",
        BinOp::Or => "|",
        BinOp::Xor => "^",
    }
}

fn render_scalar(s: &Scalar) -> String {
    match s {
        Scalar::F64(x) => Value::Float(*x).to_string(),
        Scalar::I64(x) => x.to_string(),
        Scalar::Bool(b) => b.to_string(),
        Scalar::Str(t) => format!("{:?}", t),
        Scalar::Null => "()".to_string(),
    }
}

fn render_arg(a: &LArg) -> String {
    match a {
        LArg::E(e) => render(e),
        LArg::V(v) => match v {
            Value::String(s) => format!("{:?}", s.as_str()),
            v => v.to_string(),
        },
        LArg::L(items) => format!(
            "[{}]",
            items.iter().map(render_arg).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// An Expr as it would be written.
pub fn render(e: &LExpr) -> String {
    match e {
        LExpr::Col(n) => format!("col({:?})", n),
        LExpr::Series(s) => format!("<Series[{}; {}]>", s.dtype(), s.len()),
        LExpr::Lit(s) => render_scalar(s),
        LExpr::Op(Op::Bin(BinOp::Pow), a) => format!("pow({}, {})", render(&a[0]), render(&a[1])),
        LExpr::Op(Op::Bin(b), a) => format!("({} {} {})", render(&a[0]), op_symbol(*b), render(&a[1])),
        LExpr::Op(op, a) => {
            let args: Vec<String> = a.iter().map(|x| render(x)).collect();
            let name = match op {
                Op::Neg => return format!("-{}", args[0]),
                Op::Not => return format!("!{}", args[0]),
                Op::Math(m) => m.name().to_string(),
                Op::Round(d) => return format!("round({}, {})", args[0], d),
                Op::IsNull => "is_null".to_string(),
                Op::NotNull => "not_null".to_string(),
                Op::IsNan => "is_nan".to_string(),
                Op::Where => "where".to_string(),
                Op::Coalesce => "coalesce".to_string(),
                Op::Clip => "clip".to_string(),
                Op::Between(c) => {
                    return format!(
                        "between({}, {:?})",
                        args.join(", "),
                        match c {
                            fuse::Closed::Both => "both",
                            fuse::Closed::Left => "left",
                            fuse::Closed::Right => "right",
                            fuse::Closed::Neither => "neither",
                        }
                    );
                }
                Op::Cast(t) => return format!("cast({}, {:?})", args[0], t.to_string()),
                Op::Bin(_) => unreachable!(),
            };
            format!("{}({})", name, args.join(", "))
        }
        LExpr::Call(f, args) => format!(
            "{}({})",
            f,
            args.iter().map(render_arg).collect::<Vec<_>>().join(", ")
        ),
        LExpr::Over(e, keys) => format!("over({}, {:?})", render(e), keys),
        LExpr::Alias(e, n) => format!("alias({}, {:?})", render(e), n),
    }
}

/// The name an Expr's column takes when nothing names it: its alias,
/// else the first column it reads.
pub fn output_name(e: &LExpr) -> Option<String> {
    match e {
        LExpr::Alias(_, n) | LExpr::Col(n) => Some(n.clone()),
        LExpr::Series(_) | LExpr::Lit(_) => None,
        LExpr::Op(_, a) => a.iter().find_map(|x| output_name(x)),
        LExpr::Call(_, a) => a.iter().find_map(|x| match x {
            LArg::E(e) => output_name(e),
            _ => None,
        }),
        LExpr::Over(e, _) => output_name(e),
    }
}

// ---------------------------------------------------------------------
// Operands and the elementwise builder
// ---------------------------------------------------------------------

/// Chains deeper than this are evaluated as they grow — a bound on the
/// plans a long loop of operators would otherwise build.
const MAX_FUSED_DEPTH: usize = 16;

/// One argument of an elementwise operation.
pub enum Operand {
    /// Concrete: an engine expression, its column length (None for a
    /// scalar), its depth, and the pending Series it came from (which a
    /// forced evaluation may keep, see `force`).
    Eng(Arc<Expr>, Option<usize>, usize, Vec<Value>),
    /// An Expr: resolved against a Frame later.
    Lazy(Arc<LExpr>),
}

/// `v` as an operand, or None when it is no column, Expr or scalar.
/// `()` is a null.
pub fn operand(v: &Value) -> Option<Operand> {
    if let Some(e) = expr_of(v) {
        return Some(Operand::Lazy(e.clone()));
    }
    if let Value::Native(h) = v
        && let Some(s) = h.0.as_any().downcast_ref::<OdsSeries>()
    {
        return Some(match s.pending() {
            Some((e, len, depth)) => Operand::Eng(e.clone(), Some(len), depth, vec![v.clone()]),
            None => Operand::Eng(Expr::col(s.series().clone()), Some(s.series().len()), 0, vec![]),
        });
    }
    let s = match v {
        Value::Float(_) | Value::Integer(_) | Value::Boolean(_) | Value::String(_) | Value::Unit => {
            value_to_scalar(v)?
        }
        _ => return None,
    };
    Some(Operand::Eng(Expr::lit(s), None, 0, vec![]))
}

fn to_lexpr(o: Operand) -> Result<Arc<LExpr>, String> {
    Ok(match o {
        Operand::Lazy(e) => e,
        Operand::Eng(e, _, _, _) => match e.as_ref() {
            Expr::Lit(s) => Arc::new(LExpr::Lit(s.clone())),
            Expr::Col(s) => Arc::new(LExpr::Series(s.clone())),
            _ => Arc::new(LExpr::Series(eval_one(&e, None)?)),
        },
    })
}

/// Whether a plan of `n` rows fans out.
fn par_for(n: usize, threshold: usize) -> bool {
    n >= threshold && crate::parallel::should_parallelize(n) && olang_ods::parallel_enabled()
}

/// Evaluate several engine expressions in one plan.
pub fn eval_roots(roots: &[Arc<Expr>], len: Option<usize>) -> Result<Vec<Series>, String> {
    let plan = Plan::new(roots, len).map_err(|e| e.to_string())?;
    let par = par_for(plan.len(), plan.par_rows());
    plan.eval(par).map_err(|e| e.to_string())
}

pub fn eval_one(e: &Arc<Expr>, len: Option<usize>) -> Result<Series, String> {
    Ok(eval_roots(std::slice::from_ref(e), len)?.pop().expect("one root"))
}

/// Fold a pending Series into a sum, mean, min, max or count without
/// materializing it. None when `v` is not a pending Series.
pub fn reduce_pending(v: &Value, kind: Reduce) -> Option<Result<Value, String>> {
    let Value::Native(h) = v else { return None };
    let s = h.0.as_any().downcast_ref::<OdsSeries>()?;
    let (e, _, _) = s.pending()?;
    Some(reduce_expr(e, kind))
}

fn reduce_expr(e: &Arc<Expr>, kind: Reduce) -> Result<Value, String> {
    let plan = Plan::new(std::slice::from_ref(e), None).map_err(|e| e.to_string())?;
    if plan.dtype(0) == DType::Bool && matches!(kind, Reduce::Min | Reduce::Max) {
        return Err("min/max are not defined for Bool series".to_string());
    }
    let par = par_for(plan.len(), plan.par_reduce_rows());
    let s = plan.reduce(kind, par).map_err(|e| e.to_string())?;
    Ok(scalar_to_value(s))
}

/// `ods.dot`: the sum of the products in one fused pass, nulls skipped
/// (a pending operand is never materialized).
pub fn dot_fused(a: &Value, b: &Value) -> Option<Result<Value, String>> {
    let (Some(Operand::Eng(x, Some(la), _, _)), Some(Operand::Eng(y, Some(lb), _, _))) = (operand(a), operand(b)) else {
        return None;
    };
    if la != lb {
        return Some(Err(format!("series length mismatch: {} vs {}", la, lb)));
    }
    for e in [&x, &y] {
        if !matches!(e.dtype(), Ok(Some(DType::F64 | DType::I64))) {
            return None;
        }
    }
    let f = |e: Arc<Expr>| Expr::op(Op::Cast(DType::F64), vec![e]);
    let prod = Expr::bin(BinOp::Mul, f(x), f(y));
    Some(reduce_expr(&prod, Reduce::Sum).map(|v| match v {
        Value::Integer(i) => Value::Float(i as f64),
        v => v,
    }))
}

/// Build `op` over `args`: an Expr when any is one; otherwise a Series,
/// lazy when the operation cannot raise and evaluated now when it can
/// (so an error is raised by the expression that causes it); a scalar
/// when every argument is one.
pub fn build(func: &str, op: Op, args: Vec<Operand>) -> Result<Value, String> {
    if args.iter().any(|a| matches!(a, Operand::Lazy(_))) {
        let kids = args.into_iter().map(to_lexpr).collect::<Result<_, _>>()?;
        return Ok(OdsExpr::into_value(LExpr::Op(op, kids)));
    }
    let mut len: Option<usize> = None;
    let mut depth = 0;
    let mut kids = Vec::with_capacity(args.len());
    let mut types = Vec::with_capacity(args.len());
    let mut deps = Vec::new();
    for a in args {
        let Operand::Eng(e, l, d, ds) = a else {
            unreachable!()
        };
        deps.extend(ds);
        if let Some(l) = l {
            match len {
                None => len = Some(l),
                Some(m) if m != l => {
                    return Err(format!(
                        "ods.{}: series length mismatch: {} vs {}",
                        func, m, l
                    ));
                }
                _ => {}
            }
        }
        depth = depth.max(d);
        types.push(e.dtype().map_err(|e| e.to_string())?);
        kids.push(e);
    }
    fuse::op_dtype(op, &types).map_err(|e| prefix(func, e.to_string()))?;
    let expr = Expr::op(op, kids);
    finish(func, expr, len, depth + 1, deps)
}

fn prefix(func: &str, msg: String) -> String {
    // Operator errors keep the eager kernels' words; a function's say
    // which function.
    if func.is_empty() {
        msg
    } else {
        format!("ods.{}: {}", func, msg)
    }
}

/// An engine expression as a value: lazy when it cannot raise and is
/// shallow, evaluated now otherwise; a scalar with no column under it.
/// `deps` are the pending Series the expression was built from.
pub fn finish(
    func: &str,
    expr: Arc<Expr>,
    len: Option<usize>,
    depth: usize,
    deps: Vec<Value>,
) -> Result<Value, String> {
    let Some(len) = len else {
        let s = eval_one(&expr, Some(1)).map_err(|e| prefix(func, e))?;
        return Ok(scalar_to_value(s.scalar_at(0)));
    };
    if expr.fallible() || depth > MAX_FUSED_DEPTH {
        let s = force(&expr, len, &deps).map_err(|e| prefix(func, e))?;
        return Ok(OdsSeries::into_value(s));
    }
    let dtype = expr.dtype().map_err(|e| e.to_string())?.unwrap_or(DType::F64);
    Ok(OdsSeries::pending_value(expr, len, depth, dtype, deps))
}

/// Below this cost a pending operand is recomputed by each expression
/// that reads it; at or above it (a transcendental function, or a
/// longer chain), the first evaluation that reads it keeps its column,
/// so `let p = ods.sigmoid(z)` used twice computes the sigmoid once.
const KEEP_COST: usize = 8;

/// Evaluate `expr`, keeping the column of each costly pending operand
/// (in the same plan: a shared subtree is computed once).
pub fn force(expr: &Arc<Expr>, len: usize, deps: &[Value]) -> Result<Series, String> {
    let keep: Vec<&OdsSeries> = deps
        .iter()
        .filter_map(|v| match v {
            Value::Native(h) => h.0.as_any().downcast_ref::<OdsSeries>(),
            _ => None,
        })
        .filter(|s| s.pending().is_some_and(|(e, _, _)| e.cost() >= KEEP_COST))
        .collect();
    let mut roots = vec![expr.clone()];
    roots.extend(keep.iter().map(|s| s.pending().expect("checked").0.clone()));
    let mut cols = eval_roots(&roots, Some(len))?.into_iter();
    let out = cols.next().expect("the root");
    for (s, col) in keep.into_iter().zip(cols) {
        s.keep(col);
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// Resolution against a Frame
// ---------------------------------------------------------------------

struct Resolver<'a> {
    frame: &'a Frame,
    memo: HashMap<usize, Arc<Expr>>,
}

/// The aggregation an `over` applies, from its call node's function.
pub(crate) fn agg_of(func: &str) -> Option<AggOp> {
    Some(match func {
        "sum" => AggOp::Sum,
        "mean" => AggOp::Mean,
        "min" => AggOp::Min,
        "max" => AggOp::Max,
        "count" => AggOp::Count,
        "median" => AggOp::Median,
        "std" => AggOp::Std,
        "var" => AggOp::Var,
        "first" => AggOp::First,
        "last" => AggOp::Last,
        "n_unique" => AggOp::NUnique,
        "any" => AggOp::Any,
        "all" => AggOp::All,
        "argmax" => AggOp::ArgMax,
        "argmin" => AggOp::ArgMin,
        // its q is the call's second argument
        "quantile" => AggOp::quantile(0.5),
        _ => return None,
    })
}

impl Resolver<'_> {
    fn column(&self, name: &str) -> Result<&Series, String> {
        self.frame.column(name).map_err(|_| {
            format!(
                "no column '{}' in this Frame. It has: {}",
                name,
                self.frame.names().join(", ")
            )
        })
    }

    fn series(&mut self, e: &Arc<LExpr>) -> Result<Series, String> {
        let r = self.resolve(e)?;
        eval_one(&r, Some(self.frame.n_rows()))
    }

    fn arg_value(&mut self, a: &LArg) -> Result<Value, String> {
        Ok(match a {
            LArg::V(v) => v.clone(),
            LArg::E(e) => OdsSeries::into_value(self.series(e)?),
            LArg::L(items) => Value::List(Arc::new(
                items
                    .iter()
                    .map(|x| self.arg_value(x))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
        })
    }

    fn resolve(&mut self, e: &Arc<LExpr>) -> Result<Arc<Expr>, String> {
        let key = Arc::as_ptr(e) as usize;
        if let Some(r) = self.memo.get(&key) {
            return Ok(r.clone());
        }
        let n = self.frame.n_rows();
        let r = match e.as_ref() {
            LExpr::Col(name) => Expr::col(self.column(name)?.clone()),
            LExpr::Series(s) => {
                if s.len() != n && self.frame.n_cols() > 0 {
                    return Err(format!(
                        "a Series of {} rows stands in an expression over a Frame of {} rows",
                        s.len(),
                        n
                    ));
                }
                Expr::col(s.clone())
            }
            LExpr::Lit(s) => Expr::lit(s.clone()),
            LExpr::Alias(inner, _) => self.resolve(inner)?,
            LExpr::Op(op, kids) => {
                let kids = kids
                    .iter()
                    .map(|k| self.resolve(k))
                    .collect::<Result<Vec<_>, _>>()?;
                Expr::op(*op, kids)
            }
            LExpr::Call(func, args) if func == "count" && args.is_empty() => {
                Expr::lit(Scalar::I64(n as i64))
            }
            LExpr::Call(func, args) => {
                let vals = args
                    .iter()
                    .map(|a| self.arg_value(a))
                    .collect::<Result<Vec<_>, _>>()?;
                let out = super::call(func, vals)?;
                if let Some(s) = series_of(&out) {
                    if s.len() != n {
                        return Err(format!(
                            "ods.{} gives {} rows in an expression over a Frame of {} rows; \
                             an aggregation keeps its rows with .over(keys)",
                            func,
                            s.len(),
                            n
                        ));
                    }
                    Expr::col(s.clone())
                } else if let Some(s) = value_to_scalar(&out) {
                    Expr::lit(s)
                } else {
                    return Err(format!(
                        "ods.{} gives a {}, which is not a column or a value",
                        func,
                        out.type_name()
                    ));
                }
            }
            LExpr::Over(inner, keys) => {
                let key_cols: Vec<&Series> = keys
                    .iter()
                    .map(|k| self.column(k))
                    .collect::<Result<_, _>>()?;
                let g = olang_ods::group::Grouping::new(&key_cols).map_err(|e| e.to_string())?;
                match super::group::agg_value(self.frame, keys, &g, inner)? {
                    super::group::GVal::Col(s) => {
                        Expr::col(g.broadcast(&s).map_err(|e| e.to_string())?)
                    }
                    super::group::GVal::Lit(s) => Expr::lit(s),
                }
            }
        };
        self.memo.insert(key, r.clone());
        Ok(r)
    }
}

/// An Expr evaluated against a Frame: one column of its rows.
pub(crate) fn eval_on(f: &Frame, e: &Arc<LExpr>) -> Result<Series, String> {
    if let LExpr::Col(name) = e.as_ref()
        && let Ok(c) = f.column(name)
    {
        return Ok(c.clone());
    }
    let mut r = Resolver {
        frame: f,
        memo: HashMap::new(),
    };
    r.series(e)
}

/// Several Exprs evaluated against a Frame in one plan (a column they
/// share is read once).
pub(crate) fn eval_many(f: &Frame, es: &[Arc<LExpr>]) -> Result<Vec<Series>, String> {
    let mut r = Resolver {
        frame: f,
        memo: HashMap::new(),
    };
    let roots = es.iter().map(|e| r.resolve(e)).collect::<Result<Vec<_>, _>>()?;
    eval_roots(&roots, Some(f.n_rows()))
}

/// A value in a Frame verb's spec as an engine root: an Expr resolves,
/// a Series is itself (a pending one fuses with the rest), a scalar
/// broadcasts, a list becomes a column.
fn root_of(r: &mut Resolver<'_>, v: &Value, what: &str) -> Result<Arc<Expr>, String> {
    if let Some(e) = expr_of(v) {
        return r.resolve(e);
    }
    if let Value::List(items) = v {
        return Ok(Expr::col(series_from_list(items)?));
    }
    match operand(v) {
        Some(Operand::Eng(e, len, _, _)) => {
            let n = r.frame.n_rows();
            if let Some(l) = len
                && l != n
                && r.frame.n_cols() > 0
            {
                return Err(format!(
                    "{}: a column of {} rows for a Frame of {} rows",
                    what, l, n
                ));
            }
            Ok(e)
        }
        _ => Err(format!(
            "{}: expected an Expr, a Series, a list or a value, got {}",
            what,
            v.type_name()
        )),
    }
}

/// The entries of an object or map literal, in order.
pub(crate) fn entries(v: &Value) -> Option<Vec<(String, Value)>> {
    match v {
        Value::Map(m) => Some(m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        Value::Struct { fields, .. } => {
            Some(fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        }
        _ => None,
    }
}

/// Named outputs from an object, a map, or a list of named Exprs.
pub(crate) fn named(func: &str, v: &Value) -> Result<Vec<(String, Value)>, String> {
    if let Some(e) = entries(v) {
        return Ok(e);
    }
    let items: Vec<Value> = match v {
        Value::List(items) => items.as_ref().clone(),
        v if expr_of(v).is_some() => vec![v.clone()],
        other => {
            return Err(format!(
                "ods.{}: expected an object of name: expression, got {}",
                func,
                other.type_name()
            ));
        }
    };
    items
        .into_iter()
        .map(|item| match expr_of(&item).and_then(|e| output_name(e)) {
            Some(n) => Ok((n, item)),
            None => Err(format!(
                "ods.{}: {} has no column name; give it one with ods.alias(expr, \"name\") \
                 or an object {{ name: expr }}",
                func,
                item
            )),
        })
        .collect()
}

/// Evaluate named values against `f` in one plan.
fn evaluate(func: &str, f: &Frame, items: &[(String, Value)]) -> Result<Vec<Series>, String> {
    let mut r = Resolver {
        frame: f,
        memo: HashMap::new(),
    };
    let mut roots = Vec::with_capacity(items.len());
    for (name, v) in items {
        roots.push(root_of(&mut r, v, &format!("ods.{}: '{}'", func, name))?);
    }
    eval_roots(&roots, Some(f.n_rows())).map_err(|e| format!("ods.{}: {}", func, e))
}

/// `ods.with(f, { name: expr, … })`: every expression sees the Frame as
/// it was, in one fused plan; then each replaces its column or is added.
fn with(args: &[Value]) -> Result<Value, String> {
    let f = frame_arg("with", args)?;
    let spec = args.get(1).ok_or("ods.with expects a Frame and an object of columns")?;
    let items = named("with", spec)?;
    let cols = evaluate("with", f, &items)?;
    let mut out = f.clone();
    for ((name, _), col) in items.iter().zip(cols) {
        out = out.with_column(name, col).map_err(|e| format!("ods.with: {}", e))?;
    }
    Ok(OdsFrame::into_value(out))
}

fn frame_arg<'a>(func: &str, args: &'a [Value]) -> Result<&'a Frame, String> {
    args.first().and_then(frame_of).ok_or_else(|| {
        format!(
            "ods.{}: argument 1 must be a Frame, got {}",
            func,
            args.first().map(|v| v.type_name()).unwrap_or_default()
        )
    })
}

fn filter(f: &Frame, mask: &Value) -> Result<Value, String> {
    let items = vec![("mask".to_string(), mask.clone())];
    let m = evaluate("filter", f, &items)?.pop().expect("one");
    if m.dtype() != DType::Bool {
        return Err(format!(
            "ods.filter: the condition must be a Bool mask, got a {} column",
            m.dtype()
        ));
    }
    f.filter(&m)
        .map(OdsFrame::into_value)
        .map_err(|e| e.to_string())
}

fn select(f: &Frame, spec: &Value) -> Result<Value, String> {
    let mut items: Vec<(String, Value)> = Vec::new();
    let list: Vec<Value> = match spec {
        Value::List(l) => l.as_ref().clone(),
        v => vec![v.clone()],
    };
    for v in list {
        match &v {
            Value::String(name) => {
                items.push((name.to_string(), OdsExpr::into_value(LExpr::Col(name.to_string()))))
            }
            v => items.extend(named("select", v)?),
        }
    }
    let cols = evaluate("select", f, &items)?;
    let pairs: Vec<(String, Series)> = items.into_iter().map(|(n, _)| n).zip(cols).collect();
    Frame::new(pairs)
        .map(OdsFrame::into_value)
        .map_err(|e| format!("ods.select: {}", e))
}

fn sort_by(args: &[Value]) -> Result<Value, String> {
    let f = frame_arg("sort_by", args)?;
    let keys: Vec<Value> = match args.get(1) {
        Some(Value::List(l)) => l.as_ref().clone(),
        Some(v) => vec![v.clone()],
        None => return Err("ods.sort_by expects a Frame and the keys to sort by".to_string()),
    };
    if keys.is_empty() {
        return Err("ods.sort_by: needs at least one key".to_string());
    }
    let desc: Vec<bool> = match args.get(2) {
        None => vec![false; keys.len()],
        Some(Value::Boolean(b)) => vec![*b; keys.len()],
        Some(Value::List(l)) => {
            let d: Vec<bool> = l
                .iter()
                .map(|v| match v {
                    Value::Boolean(b) => Ok(*b),
                    other => Err(format!(
                        "ods.sort_by: each descending flag must be a Bool, got {}",
                        other.type_name()
                    )),
                })
                .collect::<Result<_, _>>()?;
            if d.len() != keys.len() {
                return Err(format!(
                    "ods.sort_by: {} keys but {} descending flags",
                    keys.len(),
                    d.len()
                ));
            }
            d
        }
        Some(other) => {
            return Err(format!(
                "ods.sort_by: descending must be a Bool or a list of Bools, got {}",
                other.type_name()
            ));
        }
    };
    // One column named: the long-standing single-key sort.
    if let ([Value::String(name)], [d]) = (keys.as_slice(), desc.as_slice()) {
        return f
            .sort_by(name, *d)
            .map(OdsFrame::into_value)
            .map_err(|e| e.to_string());
    }
    let items: Vec<(String, Value)> = keys
        .iter()
        .enumerate()
        .map(|(i, k)| match k {
            Value::String(name) => Ok((name.to_string(), OdsExpr::into_value(LExpr::Col(name.to_string())))),
            v if expr_of(v).is_some() || series_of(v).is_some() => Ok((format!("key {}", i + 1), v.clone())),
            other => Err(format!(
                "ods.sort_by: a key is a column name or an Expr, got {}",
                other.type_name()
            )),
        })
        .collect::<Result<_, _>>()?;
    let cols = evaluate("sort_by", f, &items)?;
    let keys: Vec<(&Series, bool)> = cols.iter().zip(desc).collect();
    f.sort_by_keys(&keys)
        .map(OdsFrame::into_value)
        .map_err(|e| format!("ods.sort_by: {}", e))
}

fn string_keys(func: &str, v: Option<&Value>) -> Result<Vec<String>, String> {
    match v {
        Some(Value::String(s)) => Ok(vec![s.to_string()]),
        Some(Value::List(items)) if !items.is_empty() => items
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.to_string()),
                other => Err(format!(
                    "ods.{}: each key is a column name, got {}",
                    func,
                    other.type_name()
                )),
            })
            .collect(),
        other => Err(format!(
            "ods.{}: the keys are a column name or a list of them, got {}",
            func,
            other.map(|v| v.type_name()).unwrap_or_default()
        )),
    }
}

/// The Expr functions and the Frame verbs that take Exprs. None when
/// `func` with these arguments is someone else's.
pub fn dispatch(func: &str, args: &[Value]) -> Option<Result<Value, String>> {
    Some(match func {
        "col" => match args {
            [Value::String(name)] => Ok(OdsExpr::into_value(LExpr::Col(name.to_string()))),
            _ => Err(format!(
                "ods.col expects one column name, got {}",
                args.first().map(|v| v.type_name()).unwrap_or_default()
            )),
        },
        "lit" => match args {
            [v] => match value_to_scalar(v) {
                Some(s) => Ok(OdsExpr::into_value(LExpr::Lit(s))),
                None => match series_of(v) {
                    Some(s) => Ok(OdsExpr::into_value(LExpr::Series(s.clone()))),
                    None => Err(format!(
                        "ods.lit expects a number, String, Bool, () or Series, got {}",
                        v.type_name()
                    )),
                },
            },
            _ => Err("ods.lit expects one value".to_string()),
        },
        "alias" => match (args.first().and_then(expr_of), args.get(1)) {
            (Some(e), Some(Value::String(n))) if args.len() == 2 => {
                Ok(OdsExpr::into_value(LExpr::Alias(e.clone(), n.to_string())))
            }
            _ => Err("ods.alias expects an Expr and a name".to_string()),
        },
        "over" => {
            let Some(e) = args.first().and_then(expr_of) else {
                return Some(Err(format!(
                    "ods.over expects an Expr, like col(\"x\").mean(), got {}",
                    args.first().map(|v| v.type_name()).unwrap_or_default()
                )));
            };
            match string_keys("over", args.get(1)) {
                Ok(keys) => Ok(OdsExpr::into_value(LExpr::Over(e.clone(), keys))),
                Err(err) => Err(err),
            }
        }
        "with" => with(args),
        "filter" if args.len() == 2 && frame_of(&args[0]).is_some() && !is_plain_mask(&args[1]) => {
            filter(frame_of(&args[0]).expect("checked"), &args[1])
        }
        "select" if args.len() == 2 && frame_of(&args[0]).is_some() => {
            select(frame_of(&args[0]).expect("checked"), &args[1])
        }
        "sort_by" => sort_by(args),
        _ => return None,
    })
}

/// A materialized Bool Series: the frame verb's long-standing path.
fn is_plain_mask(v: &Value) -> bool {
    matches!(v, Value::Native(h) if h.0.as_any().downcast_ref::<OdsSeries>().is_some_and(|s| s.pending().is_none()))
}

/// A non-elementwise function given an Expr: a call node.
pub fn call_node(func: &str, args: &[Value]) -> Value {
    fn arg(v: &Value) -> LArg {
        match v {
            Value::List(items) if holds_expr(items) => LArg::L(items.iter().map(arg).collect()),
            v => match expr_of(v) {
                Some(e) => LArg::E(e.clone()),
                None => LArg::V(v.clone()),
            },
        }
    }
    OdsExpr::into_value(LExpr::Call(func.to_string(), args.iter().map(arg).collect()))
}

/// The binary operator hook for an Expr on either side.
pub fn binary_op(op: BinOp, lhs: &Value, rhs: &Value) -> Option<Result<Value, String>> {
    if expr_of(lhs).is_none() && expr_of(rhs).is_none() {
        return None;
    }
    let a = operand(lhs)?;
    let b = operand(rhs)?;
    Some(build("", Op::Bin(op), vec![a, b]))
}

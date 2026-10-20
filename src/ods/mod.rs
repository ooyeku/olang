//! ods — the Olang Data Stack.
//!
//! Phase 0 landed the seam: the module registry, Native values, and the
//! probe type that pins the plumbing. Phase 1 added Series — a typed,
//! null-aware 1-D array backed by the `olang-ods` engine crate (pure
//! kernels, no olang dependency). Phase 2 adds the `stats` namespace:
//! distributions, t-tests, chi-squared, correlation, and `stats.lm`,
//! all thin tables over the engine (L2 of the layer cake). See
//! `docs/design/ods.md`.

mod colfns;
mod columns;
pub mod dtypes;
pub mod expr;
mod frame;
mod group;
mod learn;
pub mod gg;
mod plot;
mod series;
mod stats;
mod table;

pub use frame::{OdsFrame, frame_of};
pub use group::{ApplyState, apply_finish, apply_groups, is_callable as apply_callable};
pub use series::{OdsSeries, make_series_value, series_of};
pub use table::scalar_text;

/// A Frame's bytes in the columnar file format (`ods.write_frame`'s), or
/// None for anything else: what a recorded run keeps of a Frame it read.
pub fn frame_bytes(v: &Value) -> Option<Vec<u8>> {
    frame_of(v).map(columns::encode)
}

/// A Frame made again from `frame_bytes`.
pub fn frame_from_bytes(b: &[u8]) -> Result<Value, String> {
    columns::decode(b, None).map(OdsFrame::into_value)
}

use crate::ast::{BinaryOp, BitwiseOp, BuiltinFunction, UnaryOp, Value};
use crate::native::{NativeHandle, NativeObject, OvmModule};
use std::any::Any;
use std::sync::Arc;

/// A minimal native value: an integer tag. `probe + Int` produces a new
/// probe with the summed tag; two probes are equal when their tags are.
#[derive(Debug)]
pub struct OdsProbe {
    pub tag: i64,
}

impl NativeObject for OdsProbe {
    fn module(&self) -> &'static str {
        "ods"
    }

    fn type_name(&self) -> &'static str {
        "OdsProbe"
    }

    fn display(&self) -> String {
        format!("<ods.probe {}>", self.tag)
    }

    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<OdsProbe>()
            .is_some_and(|o| o.tag == self.tag)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn probe_of(value: &Value) -> Option<&OdsProbe> {
    match value {
        Value::Native(h) => h.0.as_any().downcast_ref::<OdsProbe>(),
        _ => None,
    }
}

pub struct OdsModule;

impl OvmModule for OdsModule {
    fn name(&self) -> &'static str {
        "ods"
    }

    fn namespaces(&self) -> Vec<(String, Value)> {
        let mut module = crate::ast::ValueMap::default();
        for (name, arity) in [("version", 0)]
            .iter()
            .copied()
            .chain(series::FUNCTIONS.iter().copied())
            .chain(frame::FUNCTIONS.iter().copied())
            .chain(colfns::FUNCTIONS.iter().copied())
            .chain(dtypes::FUNCTIONS.iter().copied())
            .chain(group::FUNCTIONS.iter().copied())
        {
            module.insert(
                name.to_string(),
                Value::Builtin(BuiltinFunction {
                    name: format!("ods.{}", name),
                    arity,
                }),
            );
        }
        // The seam's probe is a test hook, not library surface: it lives
        // in a namespace of its own whose name no program would write,
        // and the reverse documentation test skips `__` names.
        let mut hooks = crate::ast::ValueMap::default();
        for (name, arity) in [("probe", 1), ("probe_tag", 1)] {
            hooks.insert(
                name.to_string(),
                Value::Builtin(BuiltinFunction {
                    name: format!("ods.{}", name),
                    arity,
                }),
            );
        }
        vec![
            (
                "ods".to_string(),
                Value::Struct {
                    type_name: "Module".to_string(),
                    fields: std::sync::Arc::new(module),
                },
            ),
            (
                "__ods_test".to_string(),
                Value::Struct {
                    type_name: "Module".to_string(),
                    fields: std::sync::Arc::new(hooks),
                },
            ),
        ]
    }

    fn dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, String> {
        call(func, args)
    }

    fn binary_op(&self, op: &BinaryOp, lhs: &Value, rhs: &Value) -> Option<Result<Value, String>> {
        // `a @ b` on two Series: their dot product (a Matrix's product
        // joins this when the Matrix does).
        if matches!(op, BinaryOp::MatMul) {
            if series_of(lhs).is_some() && series_of(rhs).is_some() {
                return Some(call("dot", vec![lhs.clone(), rhs.clone()]));
            }
            if let (Some(a), Some(b)) = (probe_of(lhs), probe_of(rhs)) {
                return Some(Ok(Value::Integer(a.tag * b.tag)));
            }
            return None;
        }
        if let Some(result) = series::binary_op(op, lhs, rhs) {
            return Some(result);
        }
        probe_binary_op(op, lhs, rhs)
    }

    fn unary_op(&self, op: &UnaryOp, value: &Value) -> Option<Result<Value, String>> {
        if let Some(result) = series::unary_op(op, value) {
            return Some(result);
        }
        // The probe pins the hook's plumbing in every tier: `-p` negates
        // its tag, `!p` flips its lowest bit.
        let probe = probe_of(value)?;
        let tag = match op {
            UnaryOp::Negate => probe.tag.checked_neg()?,
            UnaryOp::Not => probe.tag ^ 1,
        };
        Some(Ok(Value::Native(NativeHandle::new(OdsProbe { tag }))))
    }

    fn bitwise_op(
        &self,
        op: &BitwiseOp,
        lhs: &Value,
        rhs: &Value,
    ) -> Option<Result<Value, String>> {
        if let Some(result) = series::bitwise_op(op, lhs, rhs) {
            return Some(result);
        }
        let (a, b) = (probe_of(lhs)?, probe_of(rhs)?);
        let tag = match op {
            BitwiseOp::And => a.tag & b.tag,
            BitwiseOp::Or => a.tag | b.tag,
            BitwiseOp::Xor => a.tag ^ b.tag,
            _ => return None,
        };
        Some(Ok(Value::Native(NativeHandle::new(OdsProbe { tag }))))
    }
}

/// Call `ods.<func>` with `args`: what the module dispatches, and what
/// an Expr's call nodes run against a Frame.
pub(crate) fn call(func: &str, args: Vec<Value>) -> Result<Value, String> {
    // Exprs and the Frame verbs that take them.
    if let Some(r) = expr::dispatch(func, &args) {
        return r;
    }
    // GroupBy and the shape, group, join and time verbs (`floor` of ISO
    // text among them).
    if let Some(r) = group::dispatch(func, &args) {
        return r;
    }
    // Dates' parts and arithmetic: fused expressions too; text and
    // categories: eager kernels (call nodes for an Expr).
    if dtypes::handles(func, &args) {
        if let Some(r) = dtypes::elementwise(func, &args) {
            return r;
        }
        if let Some(r) = dtypes::eager(func, &args) {
            return r;
        }
    }
    // Elementwise: one fused expression (an Expr given an Expr).
    if !(func == "fill_null" && colfns::fill_method(&args))
        && let Some(r) = colfns::elementwise(func, &args)
    {
        return r;
    }
    // A reduction of a pending expression folds it in one pass.
    if let Some(r) = colfns::reduction(func, &args) {
        return r;
    }
    if let Some(r) = colfns::eager(func, &args) {
        return r;
    }
    // Any other column function given an Expr is a call node.
    if expr::holds_expr(&args) && series::FUNCTIONS.iter().any(|(n, _)| *n == func) {
        return Ok(expr::call_node(func, &args));
    }
    if frame::FUNCTIONS.iter().any(|(n, _)| *n == func) {
        return frame::dispatch(func, args);
    }
    // filter/take/sample are shared names: a Frame first argument
    // routes to the frame verbs, everything else to the series
    // kernels.
    if matches!(func, "filter" | "take" | "sample")
        && let Some(result) = frame::dispatch_shared(func, &args)
    {
        return result;
    }
    if series::FUNCTIONS.iter().any(|(n, _)| *n == func) {
        return series::dispatch(func, args).expect("membership checked above");
    }
    match func {
        "version" => Ok(Value::String(Arc::new(
            env!("CARGO_PKG_VERSION").to_string(),
        ))),
        "probe" => match args.as_slice() {
            [Value::Integer(tag)] => Ok(Value::Native(NativeHandle::new(OdsProbe { tag: *tag }))),
            _ => Err("ods.probe expects one Int argument".to_string()),
        },
        "probe_tag" => match args.first().and_then(probe_of) {
            Some(probe) if args.len() == 1 => Ok(Value::Integer(probe.tag)),
            _ => Err("ods.probe_tag expects one OdsProbe argument".to_string()),
        },
        _ => Err(format!("unknown ods function: ods.{}", func)),
    }
}

fn probe_binary_op(op: &BinaryOp, lhs: &Value, rhs: &Value) -> Option<Result<Value, String>> {
    match op {
        BinaryOp::Add => {
            let (probe, n) = match (probe_of(lhs), rhs) {
                (Some(p), Value::Integer(n)) => (p, *n),
                _ => match (lhs, probe_of(rhs)) {
                    (Value::Integer(n), Some(p)) => (p, *n),
                    _ => return None,
                },
            };
            Some(match probe.tag.checked_add(n) {
                Some(tag) => Ok(Value::Native(NativeHandle::new(OdsProbe { tag }))),
                None => Err(
                    "Integer overflow in addition (bigint.of gives arbitrary precision)"
                        .to_string(),
                ),
            })
        }
        // Equality falls through to the registry's structural
        // native_eq fallback; everything else is a type error.
        _ => None,
    }
}

/// The `plot` namespace: charts as SVG text from Series data. Like
/// stats, a pure-function module over ods values — no native types or
/// operators of its own.
pub struct PlotModule;

impl OvmModule for PlotModule {
    fn name(&self) -> &'static str {
        "plot"
    }

    fn namespaces(&self) -> Vec<(String, Value)> {
        let mut module = crate::ast::ValueMap::default();
        for (name, arity) in plot::FUNCTIONS.iter().chain(gg::FUNCTIONS.iter()) {
            module.insert(
                name.to_string(),
                Value::Builtin(BuiltinFunction {
                    name: format!("plot.{}", name),
                    arity: *arity,
                }),
            );
        }
        vec![(
            "plot".to_string(),
            Value::Struct {
                type_name: "Module".to_string(),
                fields: std::sync::Arc::new(module),
            },
        )]
    }

    fn dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, String> {
        if gg::FUNCTIONS.iter().any(|(n, _)| *n == func) {
            return gg::dispatch(func, args);
        }
        if !plot::handles(func) {
            return Err(format!("unknown plot function: plot.{}", func));
        }
        plot::dispatch(func, args)
    }

    fn binary_op(
        &self,
        _op: &BinaryOp,
        _lhs: &Value,
        _rhs: &Value,
    ) -> Option<Result<Value, String>> {
        None
    }
}

/// The `stats` namespace as its own registry module, so
/// `stats.norm.pdf(...)` routes through the same seam as `ods.*`.
/// Its values are ods Series; it defines no native types or operators
/// of its own.
pub struct StatsModule;

impl OvmModule for StatsModule {
    fn name(&self) -> &'static str {
        "stats"
    }

    fn namespaces(&self) -> Vec<(String, Value)> {
        vec![("stats".to_string(), stats::namespace())]
    }

    fn dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, String> {
        if !stats::handles(func) {
            return Err(format!("unknown stats function: stats.{}", func));
        }
        stats::dispatch(func, args)
    }

    fn binary_op(
        &self,
        _op: &BinaryOp,
        _lhs: &Value,
        _rhs: &Value,
    ) -> Option<Result<Value, String>> {
        None
    }
}

/// The `learn` namespace: machine learning over Frames (models,
/// preprocessing, validation, metrics). Its one native type is the
/// fitted model; it defines no operators.
pub struct LearnModule;

impl OvmModule for LearnModule {
    fn name(&self) -> &'static str {
        "learn"
    }

    fn namespaces(&self) -> Vec<(String, Value)> {
        vec![("learn".to_string(), learn::namespace())]
    }

    fn dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, String> {
        if !learn::handles(func) {
            return Err(format!("unknown learn function: learn.{}", func));
        }
        learn::dispatch(func, args)
    }

    fn binary_op(
        &self,
        _op: &BinaryOp,
        _lhs: &Value,
        _rhs: &Value,
    ) -> Option<Result<Value, String>> {
        None
    }
}

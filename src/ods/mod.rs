//! ods — the Olang Data Stack.
//!
//! Phase 0 landed the seam: the module registry, Native values, and the
//! probe type that pins the plumbing. Phase 1 added Series — a typed,
//! null-aware 1-D array backed by the `olang-ods` engine crate (pure
//! kernels, no olang dependency). Phase 2 adds the `stats` namespace:
//! distributions, t-tests, chi-squared, correlation, and `stats.lm`,
//! all thin tables over the engine (L2 of the layer cake). See
//! `docs/design/ods.md`.

mod frame;
mod plot;
mod series;
mod stats;

pub use frame::OdsFrame;
pub use series::{OdsSeries, make_series_value, series_of};

use crate::ast::{BinaryOp, BuiltinFunction, Value};
use crate::native::{NativeHandle, NativeObject, OvmModule};
use std::any::Any;
use std::collections::HashMap;
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
        let mut module = HashMap::new();
        let seam_probes = [("version", 0), ("probe", 1), ("probe_tag", 1)];
        for (name, arity) in seam_probes
            .iter()
            .copied()
            .chain(series::FUNCTIONS.iter().copied())
            .chain(frame::FUNCTIONS.iter().copied())
        {
            module.insert(
                name.to_string(),
                Value::Builtin(BuiltinFunction {
                    name: format!("ods.{}", name),
                    arity,
                }),
            );
        }
        vec![(
            "ods".to_string(),
            Value::Struct {
                type_name: "Module".to_string(),
                fields: module,
            },
        )]
    }

    fn dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, String> {
        if frame::FUNCTIONS.iter().any(|(n, _)| *n == func) {
            return frame::dispatch(func, args);
        }
        // filter/take are shared names: a Frame first argument routes to
        // the frame verbs, everything else to the series kernels.
        if matches!(func, "filter" | "take")
            && let Some(result) = frame::dispatch_shared(func, &args)
        {
            return result;
        }
        if series::FUNCTIONS.iter().any(|(n, _)| *n == func) {
            return series::dispatch(func, args).expect("membership checked above");
        }
        match func {
            "version" => Ok(Value::String(Arc::new(format!(
                "{} (phase 4)",
                env!("CARGO_PKG_VERSION")
            )))),
            "probe" => match args.as_slice() {
                [Value::Integer(tag)] => {
                    Ok(Value::Native(NativeHandle::new(OdsProbe { tag: *tag })))
                }
                _ => Err("ods.probe expects one Int argument".to_string()),
            },
            "probe_tag" => match args.first().and_then(probe_of) {
                Some(probe) if args.len() == 1 => Ok(Value::Integer(probe.tag)),
                _ => Err("ods.probe_tag expects one OdsProbe argument".to_string()),
            },
            _ => Err(format!("unknown ods function: ods.{}", func)),
        }
    }

    fn binary_op(&self, op: &BinaryOp, lhs: &Value, rhs: &Value) -> Option<Result<Value, String>> {
        if let Some(result) = series::binary_op(op, lhs, rhs) {
            return Some(result);
        }
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
                    None => Err("Integer overflow in addition".to_string()),
                })
            }
            // Equality falls through to the registry's structural
            // native_eq fallback; everything else is a type error.
            _ => None,
        }
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
        let mut module = HashMap::new();
        for (name, arity) in plot::FUNCTIONS {
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
                fields: module,
            },
        )]
    }

    fn dispatch(&self, func: &str, args: Vec<Value>) -> Result<Value, String> {
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

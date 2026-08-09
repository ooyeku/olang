//! ods — the Olang Data Stack. Phase 0: the seam, not the engine.
//!
//! This registers the module and its seam probes only. The numerical
//! engine (typed arrays, kernels) arrives in Phase 1 — see
//! `docs/design/ods.md`. The probe type exists so every piece of
//! Native-value plumbing — tier boundary, `typeof`, display, equality,
//! operator interception, module dispatch — is pinned by tests before
//! any real kernel depends on it.

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
        for (name, arity) in [("version", 0), ("probe", 1), ("probe_tag", 1)] {
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
        match func {
            "version" => Ok(Value::String(Arc::new(format!(
                "{} (phase 0)",
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

//! Controls a host binds: `runtime.bind(node, value)`, what `ui.slider`,
//! `ui.select`, `ui.date`, `ui.text`, `ui.toggle` and `ui.brush` answer.
//!
//! With no host (`olang run`, `test`, `check`, a plain REPL) a control is
//! its value. A notebook's kernel (`olang repl --serve`, evaluating a
//! cell) is a host: each control a cell binds is recorded for the cell,
//! and a value the editor set for the cell's control (`set`) is the
//! control's value while it still fits — the same kind, the same default
//! as when it was set, within the control's range or options now.
//!
//! The host's state is the evaluating thread's: the kernel evaluates a
//! cell, and the cell binds, on one thread.

use crate::ast::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// A control as a cell bound it.
#[derive(Clone, Debug)]
pub struct Bound {
    /// `slider`, `select`, `date`, `text`, `toggle`, `brush`.
    pub kind: String,
    /// What the cell gave as its value.
    pub default: Value,
    /// Its spec (`min`, `max`, `step`, `options`, `chart`), a Map.
    pub spec: Value,
    /// What `bind` answered: the host's value, or the default.
    pub value: Value,
}

/// A value a host set for a cell's control.
#[derive(Clone, Debug)]
pub struct HostValue {
    pub kind: String,
    pub default: Value,
    pub value: Value,
}

#[derive(Default)]
struct Host {
    /// The cell being evaluated, while a host evaluates one.
    cell: Option<String>,
    bound: Vec<Bound>,
    values: HashMap<String, HostValue>,
}

thread_local! {
    static HOST: RefCell<Host> = RefCell::new(Host::default());
}

/// A host evaluates `cell`: what it binds is recorded from now.
pub fn begin_cell(cell: &str) {
    HOST.with(|h| {
        let mut h = h.borrow_mut();
        h.cell = Some(cell.to_string());
        h.bound.clear();
    });
}

/// The cell's evaluation ended: the controls it bound.
pub fn end_cell() -> Vec<Bound> {
    HOST.with(|h| {
        let mut h = h.borrow_mut();
        h.cell = None;
        std::mem::take(&mut h.bound)
    })
}

/// The host's value for `cell`'s control, kept until it no longer fits.
pub fn set_value(cell: &str, v: HostValue) {
    HOST.with(|h| {
        h.borrow_mut().values.insert(cell.to_string(), v);
    });
}

/// `cell`'s host value forgotten (a cell dropped).
pub fn forget(cell: &str) {
    HOST.with(|h| {
        h.borrow_mut().values.remove(cell);
    });
}

/// Every host value forgotten (a reset).
pub fn forget_all() {
    HOST.with(|h| h.borrow_mut().values.clear());
}

fn get<'a>(spec: &'a Value, key: &str) -> Option<&'a Value> {
    match spec {
        Value::Map(m) => m.get(key),
        _ => None,
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Integer(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// Whether a slider counts in Ints: its default, its bounds and its step
/// (when it has one) all Ints.
fn int_slider(default: &Value, spec: &Value) -> bool {
    let int = |v: Option<&Value>| matches!(v, Some(Value::Integer(_)));
    matches!(default, Value::Integer(_))
        && int(get(spec, "min"))
        && int(get(spec, "max"))
        && matches!(
            get(spec, "step"),
            None | Some(Value::Unit) | Some(Value::Integer(_))
        )
}

/// Whether `s` is a date as `ui.date` takes one: `YYYY-MM-DD`.
pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(|c| c.is_ascii_digit());
    if !(digits(0..4) && digits(5..7) && digits(8..10)) {
        return false;
    }
    let month: u32 = s[5..7].parse().unwrap_or(0);
    let day: u32 = s[8..10].parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// `v` as the value of a control of `kind` with this spec and default,
/// or why it is not one: a slider's value a number within [min, max] (an
/// Int slider's an Int; an Int on a Float slider read as a Float), a
/// select's one of its options, a date a `YYYY-MM-DD` String, a text a
/// String, a toggle a Bool, a brush `()` or `[x0, x1]` (Floats, x0 ≤ x1).
pub fn validate(kind: &str, default: &Value, spec: &Value, v: &Value) -> Result<Value, String> {
    match kind {
        "slider" => {
            let lo = get(spec, "min").and_then(num);
            let hi = get(spec, "max").and_then(num);
            let ints = int_slider(default, spec);
            let x = match (v, ints) {
                (Value::Integer(_), _) | (Value::Float(_), false) => num(v).unwrap_or(0.0),
                (Value::Float(_), true) => {
                    return Err(format!("the slider counts in Ints; {} is a Float", v));
                }
                _ => {
                    return Err(format!(
                        "a slider's value is a number, got {}",
                        v.type_name()
                    ));
                }
            };
            if let (Some(lo), Some(hi)) = (lo, hi)
                && (x < lo || x > hi)
            {
                return Err(format!(
                    "{} is outside the slider's range [{}, {}]",
                    v, lo, hi
                ));
            }
            Ok(if ints { v.clone() } else { Value::Float(x) })
        }
        "select" => {
            let options = match get(spec, "options") {
                Some(Value::List(o)) => o.clone(),
                _ => Arc::new(Vec::new()),
            };
            if options.contains(v) {
                Ok(v.clone())
            } else {
                Err(format!(
                    "{} is not one of the select's options ({})",
                    v,
                    options
                        .iter()
                        .map(|o| o.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        }
        "date" => match v {
            Value::String(s) if is_date(s) => Ok(v.clone()),
            Value::String(s) => Err(format!("a date is \"YYYY-MM-DD\", got \"{}\"", s)),
            other => Err(format!("a date is a String, got {}", other.type_name())),
        },
        "text" => match v {
            Value::String(_) => Ok(v.clone()),
            other => Err(format!(
                "a text's value is a String, got {}",
                other.type_name()
            )),
        },
        "toggle" => match v {
            Value::Boolean(_) => Ok(v.clone()),
            other => Err(format!(
                "a toggle's value is a Bool, got {}",
                other.type_name()
            )),
        },
        "brush" => match v {
            Value::Unit => Ok(Value::Unit),
            Value::List(items) if items.len() == 2 => match (num(&items[0]), num(&items[1])) {
                (Some(a), Some(b)) if a <= b => Ok(Value::List(Arc::new(vec![
                    Value::Float(a),
                    Value::Float(b),
                ]))),
                (Some(_), Some(_)) => Err("a brush's range is [x0, x1] with x0 ≤ x1".to_string()),
                _ => Err("a brush's range is two numbers, [x0, x1]".to_string()),
            },
            _ => Err(format!(
                "a brush's value is () or [x0, x1], got {}",
                v.type_name()
            )),
        },
        other => Err(format!("no control is a {}", other)),
    }
}

/// `runtime.bind(node, value)`: `value`, or the host's for the cell being
/// evaluated while it still fits. The node is `ui.control`'s (its
/// `control` the kind, its `spec` the spec).
pub fn bind(args: Vec<Value>) -> Result<Value, String> {
    let [node, value] = args.as_slice() else {
        return Err(format!(
            "runtime.bind expects 2 arguments (a control's node and its value), got {}",
            args.len()
        ));
    };
    let kind = match get(node, "control") {
        Some(Value::String(k)) => k.to_string(),
        _ => {
            return Err(
                "runtime.bind: the node is no control (ui.control makes one, with its `control` kind)"
                    .to_string(),
            );
        }
    };
    let spec = get(node, "spec").cloned().unwrap_or(Value::Unit);
    Ok(HOST.with(|h| {
        let mut h = h.borrow_mut();
        let Some(cell) = h.cell.clone() else {
            return value.clone();
        };
        let mut answer = value.clone();
        if let Some(set) = h.values.get(&cell).cloned() {
            let fits = set.kind == kind
                && set.default == *value
                && validate(&kind, value, &spec, &set.value).is_ok();
            if fits {
                answer = set.value;
            } else {
                h.values.remove(&cell);
            }
        }
        h.bound.push(Bound {
            kind,
            default: value.clone(),
            spec,
            value: answer.clone(),
        });
        answer
    }))
}

/// The `runtime` module where the binary has no other (the browser):
/// `bind` alone, so `ui`'s controls work there too.
pub fn bind_only_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    module.insert(
        "bind".to_string(),
        Value::Builtin(crate::ast::BuiltinFunction {
            name: "runtime.bind".to_string(),
            arity: 2,
        }),
    );
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

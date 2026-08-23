//! Collections module (`col`) — higher-order list operations that were,
//! until now, reimplemented by hand via `fold` in every program.
//!
//! Unlike the other stdlib modules, several of these take a function
//! argument (a key function or predicate) and call back into the
//! interpreter, so the dispatcher receives `&mut Interpreter`. This is the
//! first higher-order stdlib module; the core operations (`map`, `filter`,
//! `fold`, `group_by`, ...) remain top-level builtins.
//!
//! Convention, as elsewhere in the stdlib: these are total operations over
//! well-typed input and return bare values. A wrong-typed argument is a
//! runtime error (a bug), not a recoverable `Err`.

use crate::ast::Value;
use crate::interpreter::{Interpreter, InterpreterError};
use std::collections::HashMap;
use std::sync::Arc;

/// Registers the `col` module. The function bodies live in
/// `call_collections_function`, which — unlike other modules — needs the
/// interpreter to invoke the function arguments.
pub fn create_collections_module() -> Value {
    let mut module = HashMap::new();

    let two = [
        "min_by",
        "max_by",
        "sort_by",
        "count_by",
        "partition",
        "flat_map",
        "take_while",
        "drop_while",
        "all",
        "any",
        "sum_by",
        "window",
    ];
    for name in two {
        module.insert(name.to_string(), create_builtin_function(name, 2));
    }

    let one = ["unique", "frequencies", "last"];
    for name in one {
        module.insert(name.to_string(), create_builtin_function(name, 1));
    }

    // The mutation primitives behind the olang-source collections
    // (Campaign 6): indexed write, swap, and preallocation. Their copy
    // semantics live here; the O(1) story is the assignment fusion —
    // `xs = col.set(xs, i, v)` mutates in place when `xs` holds the
    // only reference, on both tiers, exactly like `xs = xs + [..]`.
    module.insert("set".to_string(), create_builtin_function("set", 3));
    module.insert("swap".to_string(), create_builtin_function("swap", 3));
    module.insert("filled".to_string(), create_builtin_function("filled", 2));

    // zip_with(a, b, combiner)
    module.insert(
        "zip_with".to_string(),
        create_builtin_function("zip_with", 3),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("col.{}", name),
        arity,
    })
}

/// Dispatcher for `col` functions. Takes the interpreter so higher-order
/// operations can call their function arguments.
pub fn call_collections_function(
    name: &str,
    args: Vec<Value>,
    interpreter: &mut Interpreter,
) -> Result<Value, InterpreterError> {
    match name {
        "min_by" => min_max_by(args, interpreter, false),
        "max_by" => min_max_by(args, interpreter, true),
        "sort_by" => sort_by(args, interpreter),
        "count_by" => count_by(args, interpreter),
        "frequencies" => frequencies(args),
        "partition" => partition(args, interpreter),
        "flat_map" => flat_map(args, interpreter),
        "take_while" => take_drop_while(args, interpreter, true),
        "drop_while" => take_drop_while(args, interpreter, false),
        "all" => all_any(args, interpreter, true),
        "any" => all_any(args, interpreter, false),
        "sum_by" => sum_by(args, interpreter),
        "unique" => unique(args),
        "window" => window(args),
        "zip_with" => zip_with(args, interpreter),
        "last" => last(args),
        "set" => col_set(args),
        "swap" => col_swap(args),
        "filled" => col_filled(args),
        _ => Err(InterpreterError::RuntimeError {
            message: format!("Unknown col function: {}", name),
        }),
    }
}

// ── helpers ─────────────────────────────────────────────────────────

fn list_arg<'a>(
    args: &'a [Value],
    i: usize,
    func: &str,
) -> Result<&'a Arc<Vec<Value>>, InterpreterError> {
    match args.get(i) {
        Some(Value::List(items)) => Ok(items),
        Some(other) => Err(InterpreterError::TypeError {
            message: format!(
                "col.{}: argument {} must be a list, got {}",
                func,
                i + 1,
                other.type_name()
            ),
        }),
        None => Err(InterpreterError::RuntimeError {
            message: format!("col.{}: missing argument {}", func, i + 1),
        }),
    }
}

/// Resolve an index against a length with the language's indexing rule:
/// negatives count from the end. Out of bounds raises — a bad index is a
/// caller's bug, not data (same as `xs[i]`).
pub(crate) fn resolve_index(func: &str, i: i64, len: usize) -> Result<usize, String> {
    let resolved = if i < 0 { i + len as i64 } else { i };
    if resolved < 0 || resolved as usize >= len {
        return Err(format!(
            "col.{}: index {} out of bounds for list of length {}",
            func, i, len
        ));
    }
    Ok(resolved as usize)
}

fn int_arg(args: &[Value], i: usize, func: &str) -> Result<i64, InterpreterError> {
    match args.get(i) {
        Some(Value::Integer(n)) => Ok(*n),
        Some(other) => Err(InterpreterError::TypeError {
            message: format!(
                "col.{}: argument {} must be an Int, got {}",
                func,
                i + 1,
                other.type_name()
            ),
        }),
        None => Err(InterpreterError::RuntimeError {
            message: format!("col.{}: missing argument {}", func, i + 1),
        }),
    }
}

/// `col.set(xs, i, v)` — the list with element `i` replaced by `v`.
/// This is the copy path; `xs = col.set(xs, i, v)` fuses to an O(1)
/// in-place write when `xs` is sole-owned.
fn col_set(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let items = list_arg(&args, 0, "set")?;
    let i = int_arg(&args, 1, "set")?;
    let at = resolve_index("set", i, items.len())
        .map_err(|message| InterpreterError::RuntimeError { message })?;
    let value = args
        .get(2)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: "col.set: missing argument 3".to_string(),
        })?;
    let mut out = (**items).clone();
    out[at] = value;
    Ok(Value::List(Arc::new(out)))
}

/// `col.swap(xs, i, j)` — the list with elements `i` and `j` exchanged.
/// Copy path; the assignment fusion makes it O(1) in place.
fn col_swap(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let items = list_arg(&args, 0, "swap")?;
    let i = int_arg(&args, 1, "swap")?;
    let j = int_arg(&args, 2, "swap")?;
    let a = resolve_index("swap", i, items.len())
        .map_err(|message| InterpreterError::RuntimeError { message })?;
    let b = resolve_index("swap", j, items.len())
        .map_err(|message| InterpreterError::RuntimeError { message })?;
    let mut out = (**items).clone();
    out.swap(a, b);
    Ok(Value::List(Arc::new(out)))
}

/// `col.filled(n, v)` — a list of `n` copies of `v`: the preallocation
/// primitive the flat-array structures build their backing stores with.
fn col_filled(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let n = int_arg(&args, 0, "filled")?;
    if n < 0 {
        return Err(InterpreterError::RuntimeError {
            message: format!("col.filled: length must be non-negative, got {}", n),
        });
    }
    let value = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: "col.filled: missing argument 2".to_string(),
        })?;
    Ok(Value::List(Arc::new(vec![value; n as usize])))
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Boolean(b) => *b,
        Value::Unit => false,
        _ => true,
    }
}

/// Derive a map key from a value. Strings use their raw content, not the
/// quoted display form — otherwise `count_by`/`frequencies` keys come out
/// double-quoted (`"red"` instead of `red`), inconsistent with map literals.
fn key_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.to_string(),
        other => other.to_string(),
    }
}

/// Compare two key values for ordering. Integers and floats compare
/// numerically (and against each other); strings and booleans by their
/// natural order. Mixed/other types fall back to string comparison so a key
/// function never crashes the operation.
fn compare_keys(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let num = |v: &Value| -> Option<f64> {
        match v {
            Value::Integer(n) => Some(*n as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    };
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => match (a, b) {
            (Value::String(x), Value::String(y)) => x.cmp(y),
            (Value::Boolean(x), Value::Boolean(y)) => x.cmp(y),
            _ => a.to_string().cmp(&b.to_string()),
        },
    }
}

// ── operations ──────────────────────────────────────────────────────

fn min_max_by(
    args: Vec<Value>,
    interpreter: &mut Interpreter,
    want_max: bool,
) -> Result<Value, InterpreterError> {
    let func = if want_max { "max_by" } else { "min_by" };
    let list = list_arg(&args, 0, func)?;
    let key_fn = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: format!("col.{}: missing key function", func),
        })?;
    if list.is_empty() {
        return Err(InterpreterError::RuntimeError {
            message: format!("col.{}: empty list", func),
        });
    }
    let mut best_item = list[0].clone();
    let mut best_key = interpreter.call_function(key_fn.clone(), vec![list[0].clone()])?;
    for item in list.iter().skip(1) {
        let key = interpreter.call_function(key_fn.clone(), vec![item.clone()])?;
        let ordering = compare_keys(&key, &best_key);
        let replace = if want_max {
            ordering == std::cmp::Ordering::Greater
        } else {
            ordering == std::cmp::Ordering::Less
        };
        if replace {
            best_key = key;
            best_item = item.clone();
        }
    }
    Ok(best_item)
}

fn sort_by(args: Vec<Value>, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "sort_by")?;
    let key_fn = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: "col.sort_by: missing key function".to_string(),
        })?;
    // Precompute keys once (Schwartzian transform) so a costly key function
    // runs O(n) times, not O(n log n)
    let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(list.len());
    for item in list.iter() {
        let key = interpreter.call_function(key_fn.clone(), vec![item.clone()])?;
        keyed.push((key, item.clone()));
    }
    keyed.sort_by(|a, b| compare_keys(&a.0, &b.0));
    let sorted: Vec<Value> = keyed.into_iter().map(|(_, v)| v).collect();
    Ok(Value::List(sorted.into()))
}

fn count_by(args: Vec<Value>, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "count_by")?;
    let key_fn = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: "col.count_by: missing key function".to_string(),
        })?;
    let mut counts: HashMap<String, i64> = HashMap::new();
    for item in list.iter() {
        let key = interpreter.call_function(key_fn.clone(), vec![item.clone()])?;
        *counts.entry(key_string(&key)).or_insert(0) += 1;
    }
    Ok(map_of_counts(counts))
}

fn frequencies(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "frequencies")?;
    let mut counts: HashMap<String, i64> = HashMap::new();
    for item in list.iter() {
        *counts.entry(key_string(item)).or_insert(0) += 1;
    }
    Ok(map_of_counts(counts))
}

/// Build an olang map (Struct-backed) from string keys to integer counts.
fn map_of_counts(counts: HashMap<String, i64>) -> Value {
    let fields: HashMap<String, Value> = counts
        .into_iter()
        .map(|(k, v)| (k, Value::Integer(v)))
        .collect();
    Value::Map(Arc::new(fields))
}

fn partition(args: Vec<Value>, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "partition")?;
    let pred = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: "col.partition: missing predicate".to_string(),
        })?;
    let mut yes = Vec::new();
    let mut no = Vec::new();
    for item in list.iter() {
        let keep = interpreter.call_function(pred.clone(), vec![item.clone()])?;
        if truthy(&keep) {
            yes.push(item.clone());
        } else {
            no.push(item.clone());
        }
    }
    Ok(Value::Tuple(
        vec![Value::List(yes.into()), Value::List(no.into())].into(),
    ))
}

fn flat_map(args: Vec<Value>, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "flat_map")?;
    let func = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: "col.flat_map: missing function".to_string(),
        })?;
    let mut out = Vec::new();
    for item in list.iter() {
        let mapped = interpreter.call_function(func.clone(), vec![item.clone()])?;
        match mapped {
            Value::List(items) => out.extend(items.iter().cloned()),
            other => out.push(other),
        }
    }
    Ok(Value::List(out.into()))
}

fn take_drop_while(
    args: Vec<Value>,
    interpreter: &mut Interpreter,
    take: bool,
) -> Result<Value, InterpreterError> {
    let func = if take { "take_while" } else { "drop_while" };
    let list = list_arg(&args, 0, func)?;
    let pred = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: format!("col.{}: missing predicate", func),
        })?;
    let mut split = list.len();
    for (i, item) in list.iter().enumerate() {
        let keep = interpreter.call_function(pred.clone(), vec![item.clone()])?;
        if !truthy(&keep) {
            split = i;
            break;
        }
    }
    let slice: Vec<Value> = if take {
        list[..split].to_vec()
    } else {
        list[split..].to_vec()
    };
    Ok(Value::List(slice.into()))
}

fn all_any(
    args: Vec<Value>,
    interpreter: &mut Interpreter,
    want_all: bool,
) -> Result<Value, InterpreterError> {
    let func = if want_all { "all" } else { "any" };
    let list = list_arg(&args, 0, func)?;
    let pred = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: format!("col.{}: missing predicate", func),
        })?;
    for item in list.iter() {
        let result = interpreter.call_function(pred.clone(), vec![item.clone()])?;
        let t = truthy(&result);
        // Short-circuit: all stops on the first false, any on the first true
        if want_all && !t {
            return Ok(Value::Boolean(false));
        }
        if !want_all && t {
            return Ok(Value::Boolean(true));
        }
    }
    Ok(Value::Boolean(want_all))
}

fn sum_by(args: Vec<Value>, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "sum_by")?;
    let func = args
        .get(1)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: "col.sum_by: missing function".to_string(),
        })?;
    let mut int_sum: i64 = 0;
    let mut float_sum: f64 = 0.0;
    let mut any_float = false;
    for item in list.iter() {
        let v = interpreter.call_function(func.clone(), vec![item.clone()])?;
        match v {
            Value::Integer(n) => {
                int_sum = int_sum
                    .checked_add(n)
                    .ok_or_else(|| InterpreterError::RuntimeError {
                        message: "col.sum_by: integer overflow".to_string(),
                    })?;
            }
            Value::Float(f) => {
                any_float = true;
                float_sum += f;
            }
            other => {
                return Err(InterpreterError::TypeError {
                    message: format!(
                        "col.sum_by: function must return a number, got {}",
                        other.type_name()
                    ),
                });
            }
        }
    }
    if any_float {
        Ok(Value::Float(float_sum + int_sum as f64))
    } else {
        Ok(Value::Integer(int_sum))
    }
}

/// Deduplicate while preserving first-seen order.
fn unique(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "unique")?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in list.iter() {
        if seen.insert(item.to_string()) {
            out.push(item.clone());
        }
    }
    Ok(Value::List(out.into()))
}

/// Sliding windows of a fixed size: window([1,2,3,4], 2) -> [[1,2],[2,3],[3,4]].
fn window(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "window")?;
    let size = match args.get(1) {
        Some(Value::Integer(n)) if *n > 0 => *n as usize,
        Some(Value::Integer(_)) => {
            return Err(InterpreterError::RuntimeError {
                message: "col.window: size must be positive".to_string(),
            });
        }
        _ => {
            return Err(InterpreterError::TypeError {
                message: "col.window: second argument must be a positive integer".to_string(),
            });
        }
    };
    let items: Vec<Value> = list.iter().cloned().collect();
    let windows: Vec<Value> = if items.len() < size {
        Vec::new()
    } else {
        items
            .windows(size)
            .map(|w| Value::List(w.to_vec().into()))
            .collect()
    };
    Ok(Value::List(windows.into()))
}

fn zip_with(args: Vec<Value>, interpreter: &mut Interpreter) -> Result<Value, InterpreterError> {
    let a = list_arg(&args, 0, "zip_with")?.clone();
    let b = list_arg(&args, 1, "zip_with")?.clone();
    let combiner = args
        .get(2)
        .cloned()
        .ok_or_else(|| InterpreterError::RuntimeError {
            message: "col.zip_with: missing combiner function".to_string(),
        })?;
    let n = a.len().min(b.len());
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let combined =
            interpreter.call_function(combiner.clone(), vec![a[i].clone(), b[i].clone()])?;
        out.push(combined);
    }
    Ok(Value::List(out.into()))
}

fn last(args: Vec<Value>) -> Result<Value, InterpreterError> {
    let list = list_arg(&args, 0, "last")?;
    match list.last() {
        Some(v) => Ok(v.clone()),
        None => Err(InterpreterError::RuntimeError {
            message: "col.last: empty list".to_string(),
        }),
    }
}

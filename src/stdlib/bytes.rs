//! `bytes` — immutable binary data.
//!
//! olang's strings are UTF-8 text, which left binary data — an image, a
//! gzip, a non-UTF-8 file, a hash's raw output — with no value to be.
//! `Bytes` is that value: an immutable byte sequence carried as a native
//! handle, so it crosses the tier boundary as a refcount bump like every
//! other native value, compares structurally, subscripts (`b[i]` is the
//! byte at `i` as an Int, negative indices from the end), and answers
//! `len`.
//!
//! ```text
//! let b = bytes.from_list([104, 105])
//! len(b)                     // 2
//! b[0]                       // 104
//! unwrap(bytes.to_string(b)) // "hi"
//! ```
//!
//! Conventions (docs/stdlib.md): construction from a list validates its
//! elements and raises on misuse (rule 3); `bytes.to_string` returns a
//! `Result` because "these bytes are not UTF-8" is the caller's decision
//! (rule 2); everything else is infallible and returns its value.
//! `fs.read_bytes` / `fs.write_bytes` live in `fs` and carry its
//! capability gates.

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

pub struct BytesObject(pub Vec<u8>);

impl std::fmt::Debug for BytesObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl NativeObject for BytesObject {
    fn module(&self) -> &'static str {
        "bytes"
    }
    fn type_name(&self) -> &'static str {
        "Bytes"
    }
    fn display(&self) -> String {
        // A hex preview, capped: value displays are for humans and logs,
        // and a megabyte of hex is neither.
        const PREVIEW: usize = 16;
        let head: String = self
            .0
            .iter()
            .take(PREVIEW)
            .map(|b| format!("{b:02x}"))
            .collect();
        if self.0.len() > PREVIEW {
            format!("b\"{head}…\" ({} bytes)", self.0.len())
        } else {
            format!("b\"{head}\"")
        }
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        other
            .as_any()
            .downcast_ref::<BytesObject>()
            .map(|o| self.0 == o.0)
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn length(&self) -> Option<usize> {
        Some(self.0.len())
    }
    fn index(&self, key: &Value) -> Option<Result<Value, String>> {
        let i = match key {
            Value::Integer(i) => *i,
            other => {
                return Some(Err(format!(
                    "Bytes takes an integer position, got {}",
                    other.type_name()
                )));
            }
        };
        let n = self.0.len() as i64;
        let idx = if i < 0 { n + i } else { i };
        if idx < 0 || idx >= n {
            return Some(Err(format!(
                "Index {i} out of bounds for Bytes of length {n}"
            )));
        }
        Some(Ok(Value::Integer(self.0[idx as usize] as i64)))
    }
}

/// Wrap raw bytes as an olang value.
pub fn to_value(bytes: Vec<u8>) -> Value {
    Value::Native(NativeHandle::new(BytesObject(bytes)))
}

/// Pull the byte slice back out of a value, or say what it was.
pub fn bytes_of(value: &Value) -> Result<&[u8], String> {
    match value {
        Value::Native(h) => {
            h.0.as_any()
                .downcast_ref::<BytesObject>()
                .map(|b| b.0.as_slice())
                .ok_or_else(|| format!("expected Bytes, got a {} handle", h.0.type_name()))
        }
        other => Err(format!("expected Bytes, got {}", other.type_name())),
    }
}

pub fn create_bytes_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("from_list", 1),
        ("to_list", 1),
        ("from_string", 1),
        ("to_string", 1),
        ("len", 1),
        ("slice", 3),
        ("concat", 2),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("bytes.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

pub fn call_bytes_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "from_list" => from_list(args),
        "to_list" => to_list(args),
        "from_string" => from_string(args),
        "to_string" => to_string(args),
        "len" => len(args),
        "slice" => slice(args),
        "concat" => concat(args),
        _ => Err(format!("Unknown bytes function: {}", name).into()),
    }
}

fn one_arg<'a>(args: &'a [Value], name: &str) -> Result<&'a Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "bytes.{name} expects 1 argument, got {}",
            args.len()
        ));
    }
    Ok(&args[0])
}

fn from_list(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let arg = one_arg(&args, "from_list")?;
    let items = match arg {
        Value::List(items) => items,
        other => {
            return Err(format!(
                "bytes.from_list: argument must be a list of integers 0..=255, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        match item {
            Value::Integer(n) if (0..=255).contains(n) => out.push(*n as u8),
            Value::Integer(n) => {
                return Err(format!(
                    "bytes.from_list: element {i} is {n}, outside a byte's range 0..=255"
                )
                .into());
            }
            other => {
                return Err(format!(
                    "bytes.from_list: element {i} must be an integer, got {}",
                    other.type_name()
                )
                .into());
            }
        }
    }
    Ok(to_value(out))
}

fn to_list(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let b = bytes_of(one_arg(&args, "to_list")?).map_err(|e| format!("bytes.to_list: {e}"))?;
    Ok(Value::List(Arc::new(
        b.iter().map(|&x| Value::Integer(x as i64)).collect(),
    )))
}

fn from_string(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    match one_arg(&args, "from_string")? {
        Value::String(s) => Ok(to_value(s.as_bytes().to_vec())),
        other => Err(format!(
            "bytes.from_string: argument must be a string, got {}",
            other.type_name()
        )
        .into()),
    }
}

/// UTF-8 decode. A `Result`, not a raise: whether these bytes are text is
/// a property of the data, and the caller decides what a non-text answer
/// means.
fn to_string(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let b = bytes_of(one_arg(&args, "to_string")?).map_err(|e| format!("bytes.to_string: {e}"))?;
    Ok(match std::str::from_utf8(b) {
        Ok(s) => Value::Ok(Box::new(Value::String(Arc::new(s.to_string())))),
        Err(e) => Value::Err(Box::new(Value::String(Arc::new(format!(
            "bytes are not valid UTF-8: {e}"
        ))))),
    })
}

fn len(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let b = bytes_of(one_arg(&args, "len")?).map_err(|e| format!("bytes.len: {e}"))?;
    Ok(Value::Integer(b.len() as i64))
}

/// Half-open, clamped — the same shape as `str.substring`.
fn slice(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 3 {
        return Err(format!("bytes.slice expects 3 arguments, got {}", args.len()).into());
    }
    let b = bytes_of(&args[0]).map_err(|e| format!("bytes.slice: {e}"))?;
    let (from, to) = match (&args[1], &args[2]) {
        (Value::Integer(f), Value::Integer(t)) => (*f, *t),
        _ => return Err("bytes.slice: from and to must be integers".into()),
    };
    let n = b.len() as i64;
    let from = from.clamp(0, n) as usize;
    let to = to.clamp(0, n) as usize;
    Ok(to_value(if from < to {
        b[from..to].to_vec()
    } else {
        Vec::new()
    }))
}

fn concat(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(format!("bytes.concat expects 2 arguments, got {}", args.len()).into());
    }
    let a = bytes_of(&args[0]).map_err(|e| format!("bytes.concat: {e}"))?;
    let b = bytes_of(&args[1]).map_err(|e| format!("bytes.concat: {e}"))?;
    let mut out = Vec::with_capacity(a.len() + b.len());
    out.extend_from_slice(a);
    out.extend_from_slice(b);
    Ok(to_value(out))
}

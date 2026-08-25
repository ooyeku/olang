//! `caps` — what this code was granted.
//!
//! A denial still stops the program. That is deliberate and unchanged: a
//! call that needed a capability it did not have is a mistake in how the
//! program was deployed, and continuing past it would mean running a
//! program that is not the one the manifest describes.
//!
//! What was missing is the *other* branch — the ability to look before
//! leaping. A program that can degrade gracefully (skip the cache when it
//! cannot write, log to stdout when it cannot open a file, run the
//! in-memory path when it has no `db`) could not ask; it could only
//! attempt the call and be killed by it. So degradation had to be written
//! as recovery from an error, which the language deliberately does not
//! offer.
//!
//! These two functions close that gap by making the grant readable:
//!
//! ```olang
//! let plan = if caps.allowed("fs") => "cache to disk" else => "in memory"
//! ```
//!
//! The answer is the *caller's* grant, not the application's. Code living
//! in an attenuated dependency sees what that dependency was given, which
//! is the same set the gate would enforce a moment later — one
//! classification, asked two ways, exactly as `check` and `required`
//! already relate.
//!
//! Neither function is itself gated. Asking what you hold reaches nothing
//! and reveals nothing a caller could not learn by trying the call and
//! reading the error.

use crate::ast::{BuiltinFunction, Value};
use crate::caps::{Caps, FsCap};
use std::collections::HashMap;
use std::sync::Arc;

/// The capability names a program can ask about, in report order.
pub const NAMES: &[&str] = &["fs", "net", "proc", "db", "env"];

pub fn create_caps_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [("allowed", 1usize), ("granted", 0), ("level", 1)] {
        module.insert(
            name.to_string(),
            Value::Builtin(BuiltinFunction {
                name: format!("caps.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

/// `fs` is the one capability with a level rather than a yes/no, so it
/// gets a string everywhere it is reported: "none", "read", or "full".
fn fs_level(caps: &Caps) -> &'static str {
    match caps.fs {
        FsCap::None => "none",
        FsCap::Read => "read",
        FsCap::Full => "full",
    }
}

/// Is `name` held at all? For `fs` this is true at read level or above —
/// a program asking "may I touch the filesystem" is asking the coarse
/// question, and `caps.level("fs")` answers the fine one.
/// Whether `caps` holds the named capability; None for an unknown name.
/// pub(crate): the bytecode compiler asks the same question at compile
/// time to constant-fold `caps.allowed("...")` under a static manifest.
pub(crate) fn holds(caps: &Caps, name: &str) -> Option<bool> {
    Some(match name {
        "fs" => caps.fs != FsCap::None,
        "net" => caps.net,
        "proc" => caps.proc,
        "db" => caps.db,
        "env" => caps.env,
        _ => return None,
    })
}

fn unknown(name: &str) -> String {
    format!(
        "caps.allowed: '{}' is not a capability. Known: {}",
        name,
        NAMES.join(", ")
    )
}

/// Dispatch, given the caller's already-resolved grant. The resolution
/// itself lives in the interpreter, which is what knows whose code is
/// executing.
pub fn call(function: &str, args: Vec<Value>, caps: &Caps) -> Result<Value, String> {
    match function {
        "allowed" => match args.first() {
            Some(Value::String(name)) => holds(caps, name.as_str())
                .map(Value::Boolean)
                .ok_or_else(|| unknown(name.as_str())),
            other => Err(format!(
                "caps.allowed expects a capability name as a String, got {}",
                other.map(|v| v.type_name()).unwrap_or_default()
            )),
        },
        // The fine question, for the one capability that has levels.
        // Every other capability answers "none" or "full", so a caller
        // can treat the result uniformly.
        "level" => match args.first() {
            Some(Value::String(name)) => match name.as_str() {
                "fs" => Ok(Value::String(Arc::new(fs_level(caps).to_string()))),
                other => holds(caps, other)
                    .map(|held| {
                        Value::String(Arc::new(if held { "full" } else { "none" }.to_string()))
                    })
                    .ok_or_else(|| unknown(other)),
            },
            other => Err(format!(
                "caps.level expects a capability name as a String, got {}",
                other.map(|v| v.type_name()).unwrap_or_default()
            )),
        },
        // The whole grant at once, for reporting it rather than branching
        // on it. `fs` carries its level; the rest are Bool.
        "granted" => {
            let mut map = HashMap::new();
            map.insert(
                "fs".to_string(),
                Value::String(Arc::new(fs_level(caps).to_string())),
            );
            for name in &NAMES[1..] {
                map.insert(
                    (*name).to_string(),
                    Value::Boolean(holds(caps, name).expect("NAMES are known")),
                );
            }
            Ok(Value::Map(Arc::new(map)))
        }
        other => Err(format!("unknown caps function: caps.{}", other)),
    }
}

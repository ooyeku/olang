//! Regular expression module (`re`), backed by the `regex` crate.
//!
//! Every operation but `is_valid` can fail on a malformed pattern, so it
//! returns a `Result` — the olang programmer decides how to handle a bad
//! pattern rather than the program aborting. `is_valid` is total (it answers
//! the question directly) so it returns a bare boolean.

use crate::ast::Value;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;

/// Creates the regex module.
pub fn create_regex_module() -> Value {
    let mut module = HashMap::new();

    module.insert(
        "is_valid".to_string(),
        create_builtin_function("is_valid", 1),
    );
    module.insert(
        "is_match".to_string(),
        create_builtin_function("is_match", 2),
    );
    module.insert("find".to_string(), create_builtin_function("find", 2));
    module.insert(
        "find_all".to_string(),
        create_builtin_function("find_all", 2),
    );
    module.insert(
        "captures".to_string(),
        create_builtin_function("captures", 2),
    );
    module.insert("split".to_string(), create_builtin_function("split", 2));
    module.insert("replace".to_string(), create_builtin_function("replace", 3));
    module.insert(
        "replace_all".to_string(),
        create_builtin_function("replace_all", 3),
    );

    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("re.{}", name),
        arity,
    })
}

/// Dispatcher for `re` functions.
pub fn call_regex_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "is_valid" => re_is_valid(args),
        "is_match" => re_is_match(args),
        "find" => re_find(args),
        "find_all" => re_find_all(args),
        "captures" => re_captures(args),
        "split" => re_split(args),
        "replace" => re_replace(args, false),
        "replace_all" => re_replace(args, true),
        _ => Err(format!("Unknown re function: {}", name).into()),
    }
}

fn arg_str<'a>(
    args: &'a [Value],
    i: usize,
    func: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.as_ref()),
        Some(other) => Err(format!(
            "re.{}: argument {} must be a string, got {}",
            func,
            i + 1,
            other.type_name()
        )
        .into()),
        None => Err(format!("re.{}: missing argument {}", func, i + 1).into()),
    }
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

fn err(msg: String) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg))))
}

/// Compile a pattern, mapping a failure to an olang `Err` rather than a
/// runtime abort.
fn compile(pattern: &str) -> Result<Regex, Value> {
    Regex::new(pattern).map_err(|e| err(format!("invalid pattern: {}", e)))
}

fn string(s: &str) -> Value {
    Value::String(Arc::new(s.to_string()))
}

// ── operations ──────────────────────────────────────────────────────

/// Total: does the pattern compile at all?
fn re_is_valid(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "is_valid")?;
    Ok(Value::Boolean(Regex::new(pattern).is_ok()))
}

fn re_is_match(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "is_match")?;
    let text = arg_str(&args, 1, "is_match")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    Ok(ok(Value::Boolean(re.is_match(text))))
}

/// First match as `Ok(string)`, or `Ok("")` when there is no match — the
/// pattern was valid, so this is a success with an empty result.
fn re_find(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "find")?;
    let text = arg_str(&args, 1, "find")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    let found = re.find(text).map(|m| m.as_str()).unwrap_or("");
    Ok(ok(string(found)))
}

fn re_find_all(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "find_all")?;
    let text = arg_str(&args, 1, "find_all")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    let matches: Vec<Value> = re.find_iter(text).map(|m| string(m.as_str())).collect();
    Ok(ok(Value::List(matches.into())))
}

/// Capture groups of the first match: index 0 is the whole match, then each
/// group. `Ok([])` when there is no match.
fn re_captures(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "captures")?;
    let text = arg_str(&args, 1, "captures")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    let groups: Vec<Value> = match re.captures(text) {
        Some(caps) => caps
            .iter()
            .map(|g| string(g.map(|m| m.as_str()).unwrap_or("")))
            .collect(),
        None => Vec::new(),
    };
    Ok(ok(Value::List(groups.into())))
}

fn re_split(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let pattern = arg_str(&args, 0, "split")?;
    let text = arg_str(&args, 1, "split")?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    let parts: Vec<Value> = re.split(text).map(string).collect();
    Ok(ok(Value::List(parts.into())))
}

fn re_replace(args: Vec<Value>, all: bool) -> Result<Value, Box<dyn std::error::Error>> {
    let func = if all { "replace_all" } else { "replace" };
    let pattern = arg_str(&args, 0, func)?;
    let text = arg_str(&args, 1, func)?;
    let replacement = arg_str(&args, 2, func)?;
    let re = match compile(pattern) {
        Ok(re) => re,
        Err(e) => return Ok(e),
    };
    let result = if all {
        re.replace_all(text, replacement).into_owned()
    } else {
        re.replace(text, replacement).into_owned()
    };
    Ok(ok(string(&result)))
}

//! `toml` — parse and emit TOML, the config format olang's own
//! `olang.toml` manifests use. The module mirrors `json`'s core surface
//! (`parse` / `stringify` / `validate`) and produces the same value
//! shapes: tables become Maps, arrays become Lists, datetimes become
//! their string rendering. Bridging runs through serde, so the two
//! modules cannot drift on how values convert.

use crate::ast::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub fn create_toml_module() -> Value {
    let mut module = HashMap::new();
    module.insert("parse".to_string(), create_builtin_function("parse", 1));
    module.insert(
        "stringify".to_string(),
        create_builtin_function("stringify", 1),
    );
    module.insert(
        "validate".to_string(),
        create_builtin_function("validate", 1),
    );
    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

fn create_builtin_function(name: &str, arity: usize) -> Value {
    Value::Builtin(crate::ast::BuiltinFunction {
        name: format!("toml.{}", name),
        arity,
    })
}

pub fn call_toml_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "parse" => toml_parse(args),
        "stringify" => toml_stringify(args),
        "validate" => toml_validate(args),
        _ => Err(format!("Unknown toml function: {}", name).into()),
    }
}

fn one_string_arg<'a>(name: &str, args: &'a [Value]) -> Result<&'a str, Value> {
    if args.len() != 1 {
        return Err(Value::Err(Box::new(Value::String(Arc::new(format!(
            "{} expects 1 argument, got {}",
            name,
            args.len()
        ))))));
    }
    match &args[0] {
        Value::String(s) => Ok(s.as_ref()),
        _ => Err(Value::Err(Box::new(Value::String(Arc::new(format!(
            "{}: argument must be a string",
            name
        )))))),
    }
}

/// `toml.parse(text)` — a TOML document as a Map (tables → Maps,
/// arrays → Lists, datetimes → strings).
fn toml_parse(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let text = match one_string_arg("parse", &args) {
        Ok(t) => t,
        Err(e) => return Ok(e),
    };
    match text.parse::<toml::Value>() {
        Ok(tv) => match super::json::json_to_olang_value(toml_to_json(tv)) {
            Ok(v) => Ok(Value::Ok(Box::new(v))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(e))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "TOML parse error: {}",
            e
        )))))),
    }
}

/// toml::Value → serde_json::Value directly. `serde_json::to_value`
/// used to do this, but with arbitrary_precision on it leaks serde's
/// private Number wrapper as a visible one-key object; the explicit walk
/// keeps every number a plain Number and renders datetimes as their
/// string form (the documented shape) without the wrapper dance.
fn toml_to_json(v: toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s),
        toml::Value::Integer(i) => serde_json::Value::Number(serde_json::Number::from(i)),
        toml::Value::Float(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        toml::Value::Boolean(b) => serde_json::Value::Bool(b),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        toml::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(toml_to_json).collect())
        }
        toml::Value::Table(table) => serde_json::Value::Object(
            table
                .into_iter()
                .map(|(k, v)| (k, toml_to_json(v)))
                .collect(),
        ),
    }
}

/// serde_json::Value → toml::Value, the stringify direction of the
/// explicit walk above. TOML has no null, so Unit-valued fields are
/// refused with a message naming the shape rule.
fn json_to_toml(v: serde_json::Value) -> Result<toml::Value, String> {
    Ok(match v {
        serde_json::Value::Null => return Err("TOML has no null: remove () fields".to_string()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                return Err(format!("number {} does not fit TOML's integer or float", n));
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s),
        serde_json::Value::Array(items) => toml::Value::Array(
            items
                .into_iter()
                .map(json_to_toml)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        serde_json::Value::Object(map) => toml::Value::Table(
            map.into_iter()
                .map(|(k, v)| json_to_toml(v).map(|tv| (k, tv)))
                .collect::<Result<toml::map::Map<_, _>, _>>()?,
        ),
    })
}

/// `toml.stringify(value)` — a Map (or struct-like) as pretty TOML.
/// TOML documents are tables at the top level; anything else is an Err.
fn toml_stringify(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(format!("stringify expects 1 argument, got {}", args.len()).into());
    }
    let jv = match super::json::olang_value_to_json(&args[0]) {
        Ok(v) => v,
        Err(e) => {
            return Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "stringify: {}",
                e
            ))))));
        }
    };
    if !jv.is_object() {
        return Err(
            "stringify: a TOML document is a table — pass a Map or struct-like value"
                .to_string()
                .into(),
        );
    }
    match json_to_toml(jv) {
        Ok(tv) => match toml::to_string_pretty(&tv) {
            Ok(text) => Ok(Value::Ok(Box::new(Value::String(Arc::new(text))))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "TOML serialize error: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "TOML serialize error: {}",
            e
        )))))),
    }
}

/// `toml.validate(text)` — does the text parse as TOML?
fn toml_validate(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let text = match one_string_arg("validate", &args) {
        Ok(t) => t,
        Err(e) => return Ok(e),
    };
    Ok(Value::Boolean(text.parse::<toml::Value>().is_ok()))
}

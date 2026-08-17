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
        Ok(tv) => match serde_json::to_value(tv) {
            Ok(jv) => Ok(Value::Ok(Box::new(super::json::json_to_olang_value(
                unwrap_datetimes(jv),
            )))),
            Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
                "TOML convert error: {}",
                e
            )))))),
        },
        Err(e) => Ok(Value::Err(Box::new(Value::String(Arc::new(format!(
            "TOML parse error: {}",
            e
        )))))),
    }
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
    match toml::Value::try_from(jv) {
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

/// The toml crate serializes datetimes as a private one-key wrapper
/// object (`{"$__toml_private_datetime": "..."}`); unwrap those to the
/// plain string the documentation promises, recursively.
fn unwrap_datetimes(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            if map.len() == 1
                && let Some(serde_json::Value::String(s)) = map.get("$__toml_private_datetime")
            {
                return serde_json::Value::String(s.clone());
            }
            serde_json::Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, unwrap_datetimes(v)))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(unwrap_datetimes).collect())
        }
        other => other,
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

//! `toml` — parse and emit TOML, the config format olang's own
//! `olang.toml` manifests use. The module mirrors `json`'s core surface
//! (`parse` / `stringify` / `is_valid`) and produces the same value
//! shapes: tables become Maps, arrays become Lists, datetimes become
//! their string rendering. Bridging runs through serde, so the two
//! modules cannot drift on how values convert.

use crate::ast::Value;
use std::sync::Arc;

pub fn create_toml_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    module.insert("parse".to_string(), create_builtin_function("parse", 1));
    module.insert(
        "stringify".to_string(),
        create_builtin_function("stringify", 1),
    );
    module.insert(
        "is_valid".to_string(),
        create_builtin_function("is_valid", 1),
    );
    Value::Struct {
        type_name: "Module".into(),
        fields: std::sync::Arc::new(module).into(),
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
        "is_valid" => toml_is_valid(args),
        _ => Err(format!("Unknown toml function: {}", name).into()),
    }
}

/// The one string a function takes; a call without one is a misuse, and
/// raises.
fn one_string_arg<'a>(
    name: &str,
    args: &'a [Value],
) -> Result<&'a str, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(crate::stdlib::misuse::arity(
            &format!("toml.{name}"),
            "1",
            args.len(),
        ));
    }
    match &args[0] {
        Value::String(s) => Ok(s.as_ref()),
        other => Err(crate::stdlib::misuse::arg_type(
            &format!("toml.{name}"),
            "the text",
            "a string",
            other,
        )),
    }
}

/// `toml.parse(text)` — a TOML document as a Map (tables → Maps,
/// arrays → Lists, datetimes → strings).
fn toml_parse(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let text = one_string_arg("parse", &args)?;
    match text.parse::<toml::Value>() {
        Ok(tv) => match toml_to_json(tv).and_then(super::json::json_to_olang_value) {
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
///
/// TOML's `inf` and `nan` are floats olang has no value for (a Float is
/// always finite): they are an error naming the key, never a silent `()`
/// that `cfg.timeout ?? 30` would read as absent.
pub(crate) fn toml_to_json(v: toml::Value) -> Result<serde_json::Value, String> {
    to_json_at(v, &mut String::new())
}

fn to_json_at(v: toml::Value, path: &mut String) -> Result<serde_json::Value, String> {
    Ok(match v {
        toml::Value::String(s) => serde_json::Value::String(s),
        toml::Value::Integer(i) => serde_json::Value::Number(serde_json::Number::from(i)),
        toml::Value::Float(f) => match serde_json::Number::from_f64(f) {
            Some(n) => serde_json::Value::Number(n),
            None => {
                return Err(format!(
                    "TOML float {} at `{}` does not fit Float: olang Floats are finite",
                    f,
                    if path.is_empty() {
                        "(top)"
                    } else {
                        path.as_str()
                    }
                ));
            }
        },
        toml::Value::Boolean(b) => serde_json::Value::Bool(b),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        toml::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.into_iter().enumerate() {
                let len = path.len();
                path.push_str(&format!("[{}]", i));
                out.push(to_json_at(item, path)?);
                path.truncate(len);
            }
            serde_json::Value::Array(out)
        }
        toml::Value::Table(table) => {
            let mut out = serde_json::Map::new();
            for (k, v) in table {
                let len = path.len();
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(&k);
                let jv = to_json_at(v, path)?;
                path.truncate(len);
                out.insert(k, jv);
            }
            serde_json::Value::Object(out)
        }
    })
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
/// TOML documents are tables at the top level: anything else is misuse
/// and raises. A table with a value TOML cannot hold (a `()` field) is an
/// Err.
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

/// `toml.is_valid(text)` — does the text parse as TOML?
fn toml_is_valid(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let text = one_string_arg("is_valid", &args)?;
    Ok(Value::Boolean(text.parse::<toml::Value>().is_ok()))
}

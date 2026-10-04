//! Reading and making the olang values `gui` speaks in: option maps,
//! colours, boxes, and the event maps it sends.

use super::text::Color;
use crate::ast::Value;
use std::sync::Arc;

pub type Res<T> = Result<T, String>;

pub fn s(text: &str) -> Value {
    Value::String(Arc::new(text.to_string()))
}

pub fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

pub fn err(msg: impl Into<String>) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg.into()))))
}

pub fn float(f: f32) -> Value {
    Value::Float(f as f64)
}

pub fn map(fields: Vec<(&str, Value)>) -> Value {
    let mut m = crate::ast::ValueMap::default();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(m))
}

/// An event: a map with `kind` and its fields.
pub fn event(kind: &str, mut fields: Vec<(&str, Value)>) -> Value {
    fields.push(("kind", s(kind)));
    map(fields)
}

pub fn opt_str(v: Option<&str>) -> Value {
    match v {
        Some(t) => s(t),
        None => Value::Unit,
    }
}

/// The fields of a map or an object, or `None` for anything else.
pub fn fields(v: &Value) -> Option<&crate::ast::ValueMap> {
    match v {
        Value::Map(m) => Some(m),
        Value::Struct { fields, .. } => Some(fields),
        _ => None,
    }
}

pub fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match fields(v)?.get(key) {
        Some(Value::Unit) | None => None,
        Some(x) => Some(x),
    }
}

pub fn num(v: &Value) -> Option<f32> {
    match v {
        Value::Integer(i) => Some(*i as f32),
        Value::Float(f) => Some(*f as f32),
        _ => None,
    }
}

pub fn get_num(v: &Value, key: &str, what: &str) -> Res<Option<f32>> {
    match get(v, key) {
        None => Ok(None),
        Some(x) => num(x)
            .map(Some)
            .ok_or_else(|| format!("{what}: \"{key}\" must be a number, got {}", x.type_name())),
    }
}

pub fn get_str<'a>(v: &'a Value, key: &str, what: &str) -> Res<Option<&'a str>> {
    match get(v, key) {
        None => Ok(None),
        Some(Value::String(t)) => Ok(Some(t.as_str())),
        Some(x) => Err(format!(
            "{what}: \"{key}\" must be a String, got {}",
            x.type_name()
        )),
    }
}

pub fn get_bool(v: &Value, key: &str, what: &str) -> Res<Option<bool>> {
    match get(v, key) {
        None => Ok(None),
        Some(Value::Boolean(b)) => Ok(Some(*b)),
        Some(x) => Err(format!(
            "{what}: \"{key}\" must be a Bool, got {}",
            x.type_name()
        )),
    }
}

/// The numbers of a tuple or list.
pub fn nums(v: &Value) -> Option<Vec<f32>> {
    let items = match v {
        Value::Tuple(t) => t.as_slice(),
        Value::List(l) => l.as_slice(),
        _ => return None,
    };
    items.iter().map(num).collect()
}

/// `(w, h)`.
pub fn get_pair(v: &Value, key: &str, what: &str) -> Res<Option<(f32, f32)>> {
    match get(v, key) {
        None => Ok(None),
        Some(x) => match nums(x).as_deref() {
            Some([a, b]) => Ok(Some((*a, *b))),
            _ => Err(format!(
                "{what}: \"{key}\" must be a pair of numbers (a, b)"
            )),
        },
    }
}

/// `(x, y, w, h)`.
pub fn get_rect(v: &Value, key: &str, what: &str) -> Res<Option<[f32; 4]>> {
    match get(v, key) {
        None => Ok(None),
        Some(x) => match nums(x).as_deref() {
            Some([a, b, c, d]) => Ok(Some([*a, *b, *c, *d])),
            _ => Err(format!(
                "{what}: \"{key}\" must be four numbers (x, y, width, height)"
            )),
        },
    }
}

/// One number for all four sides or corners, or four (top, right,
/// bottom, left; or top-left, top-right, bottom-right, bottom-left).
pub fn get_sides(v: &Value, key: &str, what: &str) -> Res<Option<[f32; 4]>> {
    match get(v, key) {
        None => Ok(None),
        Some(x) => {
            if let Some(n) = num(x) {
                return Ok(Some([n; 4]));
            }
            match nums(x).as_deref() {
                Some([a, b]) => Ok(Some([*a, *b, *a, *b])),
                Some([a, b, c, d]) => Ok(Some([*a, *b, *c, *d])),
                _ => Err(format!(
                    "{what}: \"{key}\" must be a number or 2 or 4 numbers"
                )),
            }
        }
    }
}

pub fn get_color(v: &Value, key: &str, what: &str) -> Res<Option<Color>> {
    match get(v, key) {
        None => Ok(None),
        Some(x) => parse_color(x)
            .map(Some)
            .ok_or_else(|| {
                format!(
                    "{what}: \"{key}\" must be a colour: \"#rgb\", \"#rrggbb\", \"#rrggbbaa\", or (r, g, b[, a]) of 0–255"
                )
            }),
    }
}

pub fn parse_color(v: &Value) -> Option<Color> {
    match v {
        Value::String(t) => parse_hex(t),
        _ => match nums(v)?.as_slice() {
            [r, g, b] => Some([*r as u8, *g as u8, *b as u8, 255]),
            [r, g, b, a] => Some([*r as u8, *g as u8, *b as u8, *a as u8]),
            _ => None,
        },
    }
}

fn parse_hex(t: &str) -> Option<Color> {
    if t == "transparent" {
        return Some([0, 0, 0, 0]);
    }
    let h = t.strip_prefix('#')?;
    let digit = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok();
    let pair = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    match h.len() {
        3 => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17, 255]),
        6 => Some([pair(0)?, pair(2)?, pair(4)?, 255]),
        8 => Some([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
        _ => None,
    }
}

/// The window id inside a `Window` handle.
pub fn window_id(function: &str, v: Option<&Value>) -> Res<u64> {
    match v {
        Some(Value::Struct { type_name, fields }) if type_name == "Window" => {
            match fields.get("id") {
                Some(Value::Integer(id)) => Ok(*id as u64),
                _ => Err(format!("{function}: malformed Window handle")),
            }
        }
        Some(other) => Err(format!(
            "{function}: expected a Window (from gui.open or gui.headless), got {}",
            other.type_name()
        )),
        None => Err(format!("{function}: expected a Window")),
    }
}

pub fn window_value(id: u64, headless: bool) -> Value {
    let mut f = crate::ast::ValueMap::default();
    f.insert("id".to_string(), Value::Integer(id as i64));
    f.insert("headless".to_string(), Value::Boolean(headless));
    Value::Struct {
        type_name: "Window".to_string(),
        fields: Arc::new(f),
    }
}

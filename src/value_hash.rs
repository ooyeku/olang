//! `hash(v)`: a 64-bit structural hash that agrees with `==` (OL-425).
//!
//! Two values that are `==` hash the same: a map whatever the order of
//! its keys, a parsed JSON object as the map with its keys and values, a
//! whole Float as the Int it equals (`hash(1) == hash(1.0)`), a list in
//! the VM's typed layout as the list of its elements. The hash is XXH3
//! over a fixed encoding of the value, so it is the same in every run,
//! on every machine and in every release of olang that keeps this
//! encoding: a key a program writes to disk today finds its entry
//! tomorrow.
//!
//! The encoding: a tag byte for each kind, then its payload. Ordered
//! collections stream their elements into the same hasher; a map's (or a
//! struct's) entries are hashed one by one and summed, so their order
//! does not matter. A function has no hash (functions are not compared by
//! value), and neither has a native value whose module gives it none (a
//! cell, a channel).
//!
//! Both tiers' values are hashed here, by one walk each over the same
//! encoding: the VM's values are never converted to hash them.

use crate::ast::{EnumVariantData, Value};
use crate::ovm::value::{EnumData, OvmValue, ValueData};
use xxhash_rust::xxh3::Xxh3;

const INT: u8 = 1;
const FLOAT: u8 = 2;
const STRING: u8 = 3;
const BOOL: u8 = 4;
const UNIT: u8 = 5;
const LIST: u8 = 6;
const TUPLE: u8 = 7;
const MAP: u8 = 8;
const STRUCT: u8 = 9;
const ENUM: u8 = 10;
const OK: u8 = 11;
const ERR: u8 = 12;
const RANGE: u8 = 13;
const NATIVE: u8 = 14;

/// The hash of a value, as `hash(v)` answers it.
pub fn hash_value(v: &Value) -> Result<i64, String> {
    let mut h = Xxh3::new();
    feed(&mut h, v)?;
    Ok(h.digest() as i64)
}

/// The hash of one of the VM's values: `hash_value` of its interpreter
/// form, without converting it.
pub fn hash_ovm(v: &OvmValue) -> Result<i64, String> {
    let mut h = Xxh3::new();
    feed_ovm(&mut h, v)?;
    Ok(h.digest() as i64)
}

/// The 64-bit hash of a byte string, as `str.hash` and `crypto.xxh3`
/// answer it.
pub fn hash_bytes(b: &[u8]) -> i64 {
    xxhash_rust::xxh3::xxh3_64(b) as i64
}

fn no_hash(kind: &str) -> String {
    format!("hash: a {kind} has no hash (it is not compared by value)")
}

#[inline]
fn int(h: &mut Xxh3, n: i64) {
    h.update(&[INT]);
    h.update(&n.to_le_bytes());
}

/// A Float: a whole one as the Int it equals, as `==` says.
#[inline]
fn float(h: &mut Xxh3, f: f64) {
    if f.fract() == 0.0 && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&f)
    {
        int(h, f as i64);
    } else {
        h.update(&[FLOAT]);
        h.update(&f.to_bits().to_le_bytes());
    }
}

#[inline]
fn string(h: &mut Xxh3, s: &str) {
    h.update(&[STRING]);
    h.update(&(s.len() as u64).to_le_bytes());
    h.update(s.as_bytes());
}

fn header(h: &mut Xxh3, tag: u8, len: usize) {
    h.update(&[tag]);
    h.update(&(len as u64).to_le_bytes());
}

/// An unordered collection of entries: each hashed alone, then summed.
fn entries<'a, V: 'a, R: std::ops::Deref<Target = V>>(
    h: &mut Xxh3,
    tag: u8,
    name: Option<&str>,
    items: impl ExactSizeIterator<Item = (&'a str, R)>,
    each: impl Fn(&mut Xxh3, &V) -> Result<(), String>,
) -> Result<(), String> {
    header(h, tag, items.len());
    if let Some(n) = name {
        string(h, n);
    }
    let mut sum: u64 = 0;
    for (k, v) in items {
        let mut e = Xxh3::new();
        string(&mut e, k);
        each(&mut e, &v)?;
        sum = sum.wrapping_add(e.digest());
    }
    h.update(&sum.to_le_bytes());
    Ok(())
}

fn native(h: &mut Xxh3, n: &crate::native::NativeHandle) -> Result<(), String> {
    // pmap, pvec, psorted hash as the Map or List they equal
    if let Some(p) =
        n.0.as_any()
            .downcast_ref::<crate::stdlib::persistent::PersistentObject>()
    {
        return feed(h, &p.0.to_plain());
    }
    let x = n.0.hash64().ok_or_else(|| no_hash(n.0.type_name()))?;
    h.update(&[NATIVE]);
    string(h, n.0.type_name());
    h.update(&x.to_le_bytes());
    Ok(())
}

fn feed(h: &mut Xxh3, v: &Value) -> Result<(), String> {
    match v {
        Value::Integer(n) => int(h, *n),
        Value::Float(f) => float(h, *f),
        Value::String(s) => string(h, s),
        Value::Boolean(b) => h.update(&[BOOL, *b as u8]),
        Value::Unit => h.update(&[UNIT]),
        Value::List(items) => {
            header(h, LIST, items.len());
            for x in items.iter() {
                feed(h, x)?;
            }
        }
        Value::Tuple(items) => {
            header(h, TUPLE, items.len());
            for x in items.iter() {
                feed(h, x)?;
            }
        }
        Value::Map(m) => entries(h, MAP, None, m.iter().map(|(k, v)| (k.as_str(), v)), feed)?,
        // A struct the VM built hashes where it is, as the VM hashes it
        // (the two agree), without building its map.
        Value::Struct {
            fields: crate::ast::StructFields::Vm(obj),
            type_name,
        } if obj.shape.type_name == *type_name => {
            feed_ovm(h, &crate::ovm::value::OvmValue::new_struct(obj.clone()))?
        }
        Value::Struct { type_name, fields } => {
            // The map kinds hash as maps, since they are `==` to them.
            let name = (type_name != "Object" && type_name != "JsonObject").then_some(type_name);
            let tag = if name.is_some() { STRUCT } else { MAP };
            entries(
                h,
                tag,
                name.map(|s| s.as_str()),
                fields.iter().map(|(k, v)| (k.as_str(), v)),
                feed,
            )?
        }
        Value::Enum(e) => {
            h.update(&[ENUM]);
            string(h, &e.type_name);
            string(h, &e.variant_name);
            match &e.variant_data {
                EnumVariantData::Unit => h.update(&[0]),
                EnumVariantData::Tuple(items) => {
                    header(h, TUPLE, items.len());
                    for x in items {
                        feed(h, x)?;
                    }
                }
                EnumVariantData::Struct(fields) => entries(
                    h,
                    STRUCT,
                    None,
                    fields.iter().map(|(k, v)| (k.as_str(), v)),
                    feed,
                )?,
            }
        }
        Value::Ok(x) => {
            h.update(&[OK]);
            feed(h, x)?;
        }
        Value::Err(x) => {
            h.update(&[ERR]);
            feed(h, x)?;
        }
        Value::Range {
            start,
            end,
            inclusive,
        } => {
            h.update(&[RANGE, *inclusive as u8]);
            h.update(&start.to_le_bytes());
            h.update(&end.to_le_bytes());
        }
        Value::Native(n) => native(h, n)?,
        Value::Function(_) | Value::Builtin(_) | Value::EnumConstructor(_) => {
            return Err(no_hash("Function"));
        }
        Value::TypeInfo(_) => return Err(no_hash("type")),
    }
    Ok(())
}

fn feed_ovm(h: &mut Xxh3, v: &OvmValue) -> Result<(), String> {
    use ValueData as D;
    match &v.data {
        D::Integer(n) => int(h, *n),
        D::Float(f) => float(h, *f),
        D::String(s) => string(h, s),
        D::Boolean(b) => h.update(&[BOOL, *b as u8]),
        D::Unit => h.update(&[UNIT]),
        D::List(items) => {
            header(h, LIST, items.len());
            for x in items.iter() {
                feed_ovm(h, x)?;
            }
        }
        D::IntList(items) => {
            header(h, LIST, items.len());
            for n in items.iter() {
                int(h, *n);
            }
        }
        D::FloatList(items) => {
            header(h, LIST, items.len());
            for f in items.iter() {
                float(h, *f);
            }
        }
        D::AstList(items) => {
            header(h, LIST, items.len());
            for x in items.iter() {
                feed(h, x)?;
            }
        }
        D::Tuple(items) => {
            header(h, TUPLE, items.len());
            for x in items.iter() {
                feed_ovm(h, x)?;
            }
        }
        D::Map(m) => entries(
            h,
            MAP,
            None,
            m.iter().map(|(k, v)| (k.as_str(), v)),
            feed_ovm,
        )?,
        D::AstMap(m) | D::AstStruct(m, _) => {
            entries(h, MAP, None, m.iter().map(|(k, v)| (k.as_str(), v)), feed)?
        }
        D::Struct(st) => {
            let type_name = &st.shape.type_name;
            let name = (type_name != "Object" && type_name != "JsonObject").then_some(type_name);
            let tag = if name.is_some() { STRUCT } else { MAP };
            entries(
                h,
                tag,
                name.map(|s| s.as_str()),
                st.shape
                    .field_names
                    .iter()
                    .map(|k| k.as_str())
                    .zip(st.values()),
                feed_ovm,
            )?
        }
        D::Enum(e) => {
            h.update(&[ENUM]);
            string(h, &e.type_name);
            string(h, &e.variant_name);
            match &e.data {
                EnumData::Unit => h.update(&[0]),
                EnumData::Tuple(items) => {
                    header(h, TUPLE, items.len());
                    for x in items {
                        feed_ovm(h, x)?;
                    }
                }
                EnumData::Struct(fields) => entries(
                    h,
                    STRUCT,
                    None,
                    fields.iter().map(|(k, v)| (k.as_str(), v)),
                    feed_ovm,
                )?,
            }
        }
        D::Result(r) => match (&r.ok, &r.err) {
            (Some(x), _) => {
                h.update(&[OK]);
                feed_ovm(h, x)?;
            }
            (None, Some(x)) => {
                h.update(&[ERR]);
                feed_ovm(h, x)?;
            }
            (None, None) => {
                h.update(&[OK]);
                h.update(&[UNIT]);
            }
        },
        D::Range(r) => {
            h.update(&[RANGE, r.inclusive as u8]);
            h.update(&r.start.to_le_bytes());
            h.update(&r.end.to_le_bytes());
        }
        D::Native(n) => native(h, n)?,
        D::Function(_) | D::AstFunction(_) | D::Closure(_) | D::Builtin(_) => {
            return Err(no_hash("Function"));
        }
        D::Error(_) => return Err(no_hash("error")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn map(pairs: &[(&str, Value)]) -> Value {
        Value::Map(Arc::new(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        ))
    }

    #[test]
    fn equal_values_hash_alike() {
        let a = map(&[("x", Value::Integer(1)), ("y", Value::Integer(2))]);
        let b = map(&[("y", Value::Integer(2)), ("x", Value::Integer(1))]);
        assert_eq!(hash_value(&a), hash_value(&b));
        assert_eq!(
            hash_value(&Value::Integer(3)),
            hash_value(&Value::Float(3.0))
        );
        assert_eq!(
            hash_value(&Value::Integer(0)),
            hash_value(&Value::Float(-0.0))
        );
        let obj = Value::Struct {
            type_name: "Object".into(),
            fields: Arc::new(
                [
                    ("x".to_string(), Value::Integer(1)),
                    ("y".to_string(), Value::Integer(2)),
                ]
                .into_iter()
                .collect::<crate::ast::ValueMap>(),
            )
            .into(),
        };
        assert_eq!(hash_value(&a), hash_value(&obj));
    }

    #[test]
    fn different_values_hash_apart() {
        let pairs = [
            (Value::Integer(1), Value::Integer(2)),
            (
                Value::List(Arc::new(vec![Value::Integer(1)])),
                Value::Tuple(Arc::new(vec![Value::Integer(1)])),
            ),
            (Value::String(Arc::new("1".into())), Value::Integer(1)),
            (Value::Unit, Value::Boolean(false)),
            (
                Value::List(Arc::new(vec![
                    Value::String(Arc::new("ab".into())),
                    Value::String(Arc::new("c".into())),
                ])),
                Value::List(Arc::new(vec![
                    Value::String(Arc::new("a".into())),
                    Value::String(Arc::new("bc".into())),
                ])),
            ),
        ];
        for (a, b) in pairs {
            assert_ne!(hash_value(&a), hash_value(&b), "{a} vs {b}");
        }
    }

    #[test]
    fn the_hash_is_the_same_in_every_run() {
        // Pinned: a hash written to disk must find its entry in a later run
        // and a later release. Changing the encoding changes these.
        assert_eq!(
            hash_value(&Value::Integer(1)),
            Ok(hash_value(&Value::Integer(1)).unwrap())
        );
        let pinned = hash_value(&map(&[("a", Value::Integer(1))])).unwrap();
        assert_eq!(pinned, PINNED_MAP_A1);
        assert_eq!(hash_bytes(b""), 0x2D06800538D394C2u64 as i64);
    }

    /// `hash(#{ "a": 1 })`, recorded when the encoding was written.
    const PINNED_MAP_A1: i64 = 5042304312280865944;

    #[test]
    fn both_tiers_hash_alike() {
        let values = [
            map(&[
                (
                    "rows",
                    Value::List(Arc::new(vec![Value::Integer(1), Value::Float(2.5)])),
                ),
                ("ok", Value::Ok(Box::new(Value::Unit))),
                ("t", Value::Tuple(Arc::new(vec![Value::Boolean(true)]))),
            ]),
            Value::List(Arc::new((0..100).map(Value::Integer).collect())),
            Value::List(Arc::new(
                (0..100).map(|i| Value::Float(i as f64 * 0.5)).collect(),
            )),
        ];
        for v in values {
            let ovm = OvmValue::from_ast(v.clone());
            assert_eq!(hash_ovm(&ovm), hash_value(&v), "{v}");
        }
    }
}

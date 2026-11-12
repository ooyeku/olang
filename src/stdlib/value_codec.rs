//! `value` — a lossless binary encoding of olang values (OL-427).
//!
//! `value.encode(v)` answers `Bytes`; `value.decode(b)` answers
//! `Ok(v)` with a value `==` to the one encoded, for every data value:
//! Int, Float, String, Bool, `()`, List, Tuple, Map, a struct, an enum,
//! `Ok`/`Err`, Bytes, a Date, a BigInt and a range. JSON keeps none of
//! tuples, Bytes, results or enums, and reorders nothing it is not told
//! to; this keeps them all.
//!
//! `value.encode(v, #{ canonical: true })` writes every map's keys (and
//! every struct's fields) in sorted order, so two `==` values encode to
//! the same bytes: an encoding that can be hashed, compared or used as a
//! key. Without it a map is written in its own order, which decoding
//! keeps.
//!
//! A function, a cell, a channel or any other handle has no data to
//! write, and encoding one raises naming it.
//!
//! The format: the magic `OLV`, a version byte (1), then the value. Each
//! value is a tag byte and its payload; lengths and Ints are LEB128
//! varints (an Int zigzagged), a Float its eight little-endian bytes.

use crate::ast::{EnumValue, EnumVariantData, Value, ValueMap};
use std::sync::Arc;

const MAGIC: &[u8; 3] = b"OLV";
const VERSION: u8 = 1;

const T_UNIT: u8 = 0;
const T_FALSE: u8 = 1;
const T_TRUE: u8 = 2;
const T_INT: u8 = 3;
const T_FLOAT: u8 = 4;
const T_STRING: u8 = 5;
const T_LIST: u8 = 6;
const T_TUPLE: u8 = 7;
const T_MAP: u8 = 8;
const T_STRUCT: u8 = 9;
const T_ENUM: u8 = 10;
const T_OK: u8 = 11;
const T_ERR: u8 = 12;
const T_BYTES: u8 = 13;
const T_RANGE: u8 = 14;
const T_DATE: u8 = 15;
const T_BIGINT: u8 = 16;
const T_PMAP: u8 = 17;
const T_PVEC: u8 = 18;
const T_PSORTED: u8 = 19;

const E_UNIT: u8 = 0;
const E_TUPLE: u8 = 1;
const E_STRUCT: u8 = 2;

/// How deep a value may nest before decoding refuses it (a crafted input
/// must not exhaust the stack).
const MAX_DEPTH: usize = 10_000;

pub fn create_value_module() -> Value {
    let mut module = ValueMap::default();
    for (name, arity) in [("encode", 1), ("decode", 1)] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("value.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".into(),
        fields: Arc::new(module).into(),
    }
}

pub fn call_value_function(name: &str, args: Vec<Value>) -> Result<Value, String> {
    match name {
        "encode" => {
            let canonical = match args.as_slice() {
                [_] => false,
                [_, opts] => canonical_opt(opts)?,
                _ => {
                    return Err(format!(
                        "value.encode expects a value and, optionally, an options map; got {} arguments",
                        args.len()
                    ));
                }
            };
            Ok(crate::stdlib::bytes::to_value(encode(&args[0], canonical)?))
        }
        "decode" => match args.as_slice() {
            [b] => {
                let bytes =
                    crate::stdlib::bytes::bytes_of(b).map_err(|e| format!("value.decode: {e}"))?;
                Ok(match decode(bytes) {
                    Ok(v) => Value::Ok(Box::new(v)),
                    Err(e) => Value::Err(Box::new(Value::String(Arc::new(e)))),
                })
            }
            _ => Err(format!(
                "value.decode expects one argument, Bytes; got {}",
                args.len()
            )),
        },
        _ => Err(format!("Unknown value function: {name}")),
    }
}

fn canonical_opt(opts: &Value) -> Result<bool, String> {
    let fields = match opts {
        Value::Map(m) => m.clone(),
        Value::Struct { type_name, fields }
            if type_name == "Object" || type_name == "JsonObject" =>
        {
            fields.to_arc()
        }
        other => {
            return Err(format!(
                "value.encode: options must be a map, got {}",
                other.type_name()
            ));
        }
    };
    let mut canonical = false;
    for (k, v) in fields.iter() {
        match (k.as_str(), v) {
            ("canonical", Value::Boolean(b)) => canonical = *b,
            ("canonical", other) => {
                return Err(format!(
                    "value.encode: canonical must be true or false, got {}",
                    other.type_name()
                ));
            }
            (other, _) => {
                return Err(format!(
                    "value.encode: unknown option {other:?} (the option is canonical)"
                ));
            }
        }
    }
    Ok(canonical)
}

/// The encoding of `v`, header included.
pub fn encode(v: &Value, canonical: bool) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    write(&mut out, v, canonical)?;
    Ok(out)
}

fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn bytes(out: &mut Vec<u8>, b: &[u8]) {
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

fn write_fields(out: &mut Vec<u8>, m: &ValueMap, canonical: bool) -> Result<(), String> {
    varint(out, m.len() as u64);
    if canonical {
        let mut keys: Vec<&String> = m.keys().collect();
        keys.sort_unstable();
        for k in keys {
            bytes(out, k.as_bytes());
            write(out, &m[k.as_str()], canonical)?;
        }
    } else {
        for (k, v) in m.iter() {
            bytes(out, k.as_bytes());
            write(out, v, canonical)?;
        }
    }
    Ok(())
}

fn write(out: &mut Vec<u8>, v: &Value, canonical: bool) -> Result<(), String> {
    match v {
        Value::Unit => out.push(T_UNIT),
        Value::Boolean(false) => out.push(T_FALSE),
        Value::Boolean(true) => out.push(T_TRUE),
        Value::Integer(n) => {
            out.push(T_INT);
            varint(out, ((n << 1) ^ (n >> 63)) as u64);
        }
        Value::Float(f) => {
            out.push(T_FLOAT);
            out.extend_from_slice(&f.to_le_bytes());
        }
        Value::String(s) => {
            out.push(T_STRING);
            bytes(out, s.as_bytes());
        }
        Value::List(items) | Value::Tuple(items) => {
            out.push(if matches!(v, Value::List(_)) {
                T_LIST
            } else {
                T_TUPLE
            });
            varint(out, items.len() as u64);
            for x in items.iter() {
                write(out, x, canonical)?;
            }
        }
        Value::Map(m) => {
            out.push(T_MAP);
            write_fields(out, m, canonical)?;
        }
        Value::Struct { type_name, fields } => {
            if type_name == "Module" {
                return Err("value.encode: a module cannot be encoded".to_string());
            }
            out.push(T_STRUCT);
            bytes(out, type_name.as_bytes());
            write_fields(out, fields, canonical)?;
        }
        Value::Enum(e) => {
            out.push(T_ENUM);
            bytes(out, e.type_name.as_bytes());
            bytes(out, e.variant_name.as_bytes());
            match &e.variant_data {
                EnumVariantData::Unit => out.push(E_UNIT),
                EnumVariantData::Tuple(items) => {
                    out.push(E_TUPLE);
                    varint(out, items.len() as u64);
                    for x in items {
                        write(out, x, canonical)?;
                    }
                }
                EnumVariantData::Struct(fields) => {
                    out.push(E_STRUCT);
                    write_fields(out, fields, canonical)?;
                }
            }
        }
        Value::Ok(x) => {
            out.push(T_OK);
            write(out, x, canonical)?;
        }
        Value::Err(x) => {
            out.push(T_ERR);
            write(out, x, canonical)?;
        }
        Value::Range {
            start,
            end,
            inclusive,
        } => {
            out.push(T_RANGE);
            out.push(*inclusive as u8);
            varint(out, ((start << 1) ^ (start >> 63)) as u64);
            varint(out, ((end << 1) ^ (end >> 63)) as u64);
        }
        Value::Native(h) => {
            let any = h.0.as_any();
            if let Some(p) = crate::stdlib::persistent::of(v) {
                use crate::stdlib::persistent::Persistent as P;
                match p {
                    P::Vec(items) => {
                        out.push(T_PVEC);
                        varint(out, items.len() as u64);
                        for x in items.iter() {
                            write(out, x, canonical)?;
                        }
                    }
                    P::Map(_) | P::Sorted(_) => {
                        out.push(if matches!(p, P::Map(_)) {
                            T_PMAP
                        } else {
                            T_PSORTED
                        });
                        // keys in order: canonical without asking
                        let listed = crate::stdlib::persistent::call_keyed(
                            if matches!(p, P::Map(_)) {
                                "pmap"
                            } else {
                                "psorted"
                            },
                            "entries",
                            vec![v.clone()],
                        )?;
                        let Value::List(entries) = &listed else {
                            unreachable!("entries is a list")
                        };
                        varint(out, entries.len() as u64);
                        for e in entries.iter() {
                            if let Value::Tuple(kv) = e {
                                write(out, &kv[0], canonical)?;
                                write(out, &kv[1], canonical)?;
                            }
                        }
                    }
                }
            } else if let Some(b) = any.downcast_ref::<crate::stdlib::bytes::BytesObject>() {
                out.push(T_BYTES);
                bytes(out, b.as_slice());
            } else if let Some(d) = any.downcast_ref::<crate::stdlib::dates::DateObject>() {
                use chrono::Datelike;
                out.push(T_DATE);
                let n = d.0.num_days_from_ce() as i64;
                varint(out, ((n << 1) ^ (n >> 63)) as u64);
            } else if let Some(b) = any.downcast_ref::<crate::stdlib::bigint::BigIntObject>() {
                out.push(T_BIGINT);
                bytes(out, &b.0.to_signed_bytes_le());
            } else {
                return Err(format!(
                    "value.encode: a {} cannot be encoded: it is a handle, not data",
                    h.0.type_name()
                ));
            }
        }
        Value::Function(_) | Value::Builtin(_) | Value::EnumConstructor(_) => {
            return Err(
                "value.encode: a Function cannot be encoded: it is code, not data".to_string(),
            );
        }
        Value::TypeInfo(_) => {
            return Err("value.encode: a type cannot be encoded".to_string());
        }
    }
    Ok(())
}

/// The value `b` encodes, or what is wrong with it.
pub fn decode(b: &[u8]) -> Result<Value, String> {
    let rest = b
        .strip_prefix(MAGIC.as_slice())
        .ok_or("value.decode: not an encoded value (no OLV header)")?;
    let (&version, body) = rest
        .split_first()
        .ok_or("value.decode: the input ends after its header")?;
    if version != VERSION {
        return Err(format!(
            "value.decode: version {version} of the encoding, and this olang reads version {VERSION}"
        ));
    }
    let mut r = Reader { b: body, at: 0 };
    let v = r.value(0)?;
    if r.at != body.len() {
        return Err(format!(
            "value.decode: {} bytes left over after the value",
            body.len() - r.at
        ));
    }
    Ok(v)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("value.decode: {what} at byte {}", self.at + 4))
    }

    fn byte(&mut self) -> Result<u8, String> {
        match self.b.get(self.at) {
            Some(&x) => {
                self.at += 1;
                Ok(x)
            }
            None => self.fail("the input ends early"),
        }
    }

    fn varint(&mut self) -> Result<u64, String> {
        let mut n: u64 = 0;
        for shift in (0..64).step_by(7) {
            let x = self.byte()?;
            n |= ((x & 0x7f) as u64) << shift;
            if x & 0x80 == 0 {
                return Ok(n);
            }
        }
        self.fail("a number too long")
    }

    fn zigzag(&mut self) -> Result<i64, String> {
        let n = self.varint()?;
        Ok(((n >> 1) as i64) ^ -((n & 1) as i64))
    }

    fn len(&mut self) -> Result<usize, String> {
        let n = self.varint()?;
        // Every element takes at least one byte: a length past what is left
        // is a corrupt input, not an allocation to attempt.
        if n > (self.b.len() - self.at) as u64 {
            return self.fail("a length longer than the input");
        }
        Ok(n as usize)
    }

    fn slice(&mut self) -> Result<&'a [u8], String> {
        let n = self.len()?;
        let s = &self.b[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }

    fn text(&mut self) -> Result<String, String> {
        let s = self.slice()?;
        match std::str::from_utf8(s) {
            Ok(t) => Ok(t.to_string()),
            Err(_) => self.fail("a string that is not UTF-8"),
        }
    }

    fn fields(&mut self, depth: usize) -> Result<ValueMap, String> {
        let n = self.len()?;
        let mut m = ValueMap::with_capacity_and_hasher(n, Default::default());
        for _ in 0..n {
            let k = self.text()?;
            let v = self.value(depth + 1)?;
            m.insert(k, v);
        }
        Ok(m)
    }

    fn items(&mut self, depth: usize) -> Result<Vec<Value>, String> {
        let n = self.len()?;
        let mut items = Vec::with_capacity(n);
        for _ in 0..n {
            items.push(self.value(depth + 1)?);
        }
        Ok(items)
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return self.fail("a value nested too deep");
        }
        Ok(match self.byte()? {
            T_UNIT => Value::Unit,
            T_FALSE => Value::Boolean(false),
            T_TRUE => Value::Boolean(true),
            T_INT => Value::Integer(self.zigzag()?),
            T_FLOAT => {
                if self.b.len() - self.at < 8 {
                    return self.fail("the input ends early");
                }
                let mut a = [0u8; 8];
                a.copy_from_slice(&self.b[self.at..self.at + 8]);
                self.at += 8;
                let f = f64::from_le_bytes(a);
                if !f.is_finite() {
                    return self.fail("a Float that is not finite");
                }
                Value::Float(f)
            }
            T_STRING => Value::String(Arc::new(self.text()?)),
            T_LIST => Value::List(Arc::new(self.items(depth)?)),
            T_TUPLE => Value::Tuple(Arc::new(self.items(depth)?)),
            T_MAP => Value::Map(Arc::new(self.fields(depth)?)),
            T_STRUCT => {
                let type_name = self.text()?;
                Value::Struct {
                    type_name: type_name.into(),
                    fields: Arc::new(self.fields(depth)?).into(),
                }
            }
            T_ENUM => {
                let type_name = self.text()?;
                let variant_name = self.text()?;
                let variant_data = match self.byte()? {
                    E_UNIT => EnumVariantData::Unit,
                    E_TUPLE => EnumVariantData::Tuple(self.items(depth)?),
                    E_STRUCT => EnumVariantData::Struct(self.fields(depth)?),
                    _ => return self.fail("an enum of an unknown kind"),
                };
                Value::Enum(Arc::new(EnumValue {
                    type_name,
                    variant_name,
                    variant_data,
                }))
            }
            T_OK => Value::Ok(Box::new(self.value(depth + 1)?)),
            T_ERR => Value::Err(Box::new(self.value(depth + 1)?)),
            T_BYTES => crate::stdlib::bytes::to_value(self.slice()?.to_vec()),
            T_RANGE => {
                let inclusive = self.byte()? != 0;
                let start = self.zigzag()?;
                let end = self.zigzag()?;
                Value::Range {
                    start,
                    end,
                    inclusive,
                }
            }
            T_DATE => {
                let n = self.zigzag()?;
                match i32::try_from(n)
                    .ok()
                    .and_then(chrono::NaiveDate::from_num_days_from_ce_opt)
                {
                    Some(d) => crate::stdlib::dates::date_value(d),
                    None => return self.fail("a date out of range"),
                }
            }
            T_BIGINT => {
                let b = self.slice()?;
                crate::stdlib::bigint::bigint_value(num_bigint::BigInt::from_signed_bytes_le(b))
            }
            T_PVEC => {
                let items = self.items(depth)?;
                crate::stdlib::persistent::call_vec("of", vec![Value::List(Arc::new(items))])
                    .map_err(|e| format!("value.decode: {e}"))?
            }
            tag @ (T_PMAP | T_PSORTED) => {
                let n = self.len()?;
                let prefix = if tag == T_PMAP { "pmap" } else { "psorted" };
                let mut p = crate::stdlib::persistent::call_keyed(prefix, "new", vec![])?;
                for _ in 0..n {
                    let k = self.value(depth + 1)?;
                    let v = self.value(depth + 1)?;
                    p = crate::stdlib::persistent::call_keyed(prefix, "set", vec![p, k, v])
                        .map_err(|e| format!("value.decode: {e}"))?;
                }
                p
            }
            _ => return self.fail("an unknown tag"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Value {
        Value::String(Arc::new(x.to_string()))
    }

    #[test]
    fn data_round_trips() {
        let m: ValueMap = [
            ("b".to_string(), Value::Integer(-7)),
            ("a".to_string(), Value::Float(2.5)),
        ]
        .into_iter()
        .collect();
        let v = Value::List(Arc::new(vec![
            Value::Unit,
            Value::Boolean(true),
            Value::Integer(i64::MIN),
            Value::Integer(i64::MAX),
            s("héllo"),
            Value::Tuple(Arc::new(vec![Value::Integer(1), s("x")])),
            Value::Map(Arc::new(m.clone())),
            Value::Ok(Box::new(Value::Err(Box::new(s("no"))))),
            crate::stdlib::bytes::to_value(vec![0, 255, 7]),
            Value::Struct {
                type_name: "P".into(),
                fields: Arc::new(m).into(),
            },
        ]));
        let back = decode(&encode(&v, false).unwrap()).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn canonical_encodings_of_equal_maps_are_equal() {
        let a: ValueMap = [("x", 1), ("y", 2)]
            .iter()
            .map(|(k, v)| (k.to_string(), Value::Integer(*v)))
            .collect();
        let b: ValueMap = [("y", 2), ("x", 1)]
            .iter()
            .map(|(k, v)| (k.to_string(), Value::Integer(*v)))
            .collect();
        let (a, b) = (Value::Map(Arc::new(a)), Value::Map(Arc::new(b)));
        assert_eq!(encode(&a, true), encode(&b, true));
        assert_ne!(encode(&a, false), encode(&b, false));
    }

    #[test]
    fn corrupt_input_is_an_error_not_a_crash() {
        let good = encode(&Value::List(Arc::new(vec![s("abc"); 3])), false).unwrap();
        for cut in 0..good.len() {
            assert!(decode(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut huge = b"OLV\x01\x06".to_vec();
        huge.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0x0f]);
        assert!(decode(&huge).is_err());
        assert!(decode(b"OLV\x02\x00").unwrap_err().contains("version 2"));
    }
}

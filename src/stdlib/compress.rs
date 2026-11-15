//! `compress` — gzip and raw deflate, in memory.
//!
//! The wire is where a response spends its time: a 90 KB JSON sync is
//! 10 KB gzipped, and until now nothing in the language could produce
//! it — `http.serve` negotiated encodings only for pre-compressed files
//! on disk. Every function takes a string or `Bytes` and answers
//! `Bytes` (or `Result` where the input may be malformed), on every
//! target: the codec is pure Rust, so the browser runtime has it too.

use crate::ast::Value;
use crate::stdlib::bytes;
use std::io::{Read, Write};
use std::sync::Arc;

pub fn create_compress_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [
        ("gzip", 1),
        ("gunzip", 1),
        #[cfg(feature = "native")]
        ("brotli", 1),
        #[cfg(feature = "native")]
        ("brotli_level", 2),
        #[cfg(feature = "native")]
        ("unbrotli", 1),
        ("deflate", 1),
        ("inflate", 1),
        ("gzip_level", 2),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("compress.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".into(),
        fields: Arc::new(module).into(),
    }
}

pub fn call_compress_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "gzip" => {
            let input = input_bytes(&args, "gzip", 1)?;
            Ok(bytes::to_value(gzip(input, 6)?))
        }
        "gzip_level" => {
            let input = input_bytes(&args, "gzip_level", 2)?;
            let level = match args.get(1) {
                Some(Value::Integer(n)) if (0..=9).contains(n) => *n as u32,
                _ => return Err("compress.gzip_level: level must be an integer 0..=9".into()),
            };
            Ok(bytes::to_value(gzip(input, level)?))
        }
        "deflate" => {
            let input = input_bytes(&args, "deflate", 1)?;
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
            encoder.write_all(input)?;
            Ok(bytes::to_value(encoder.finish()?))
        }
        #[cfg(feature = "native")]
        "brotli" => {
            let input = input_bytes(&args, "brotli", 1)?;
            Ok(bytes::to_value(brotli(input, 5)?))
        }
        #[cfg(feature = "native")]
        "brotli_level" => {
            let input = input_bytes(&args, "brotli_level", 2)?;
            let quality = match args.get(1) {
                Some(Value::Integer(n)) if (0..=11).contains(n) => *n as u32,
                _ => return Err("compress.brotli_level: quality must be an integer 0..=11".into()),
            };
            Ok(bytes::to_value(brotli(input, quality)?))
        }
        #[cfg(feature = "native")]
        "unbrotli" => {
            let (input, cap) = decode_args(&args, "unbrotli")?;
            Ok(capped(
                ::brotli::Decompressor::new(input, 4096),
                cap,
                "unbrotli",
            ))
        }
        "gunzip" => {
            let (input, cap) = decode_args(&args, "gunzip")?;
            Ok(capped(flate2::read::GzDecoder::new(input), cap, "gunzip"))
        }
        "inflate" => {
            let (input, cap) = decode_args(&args, "inflate")?;
            Ok(capped(
                flate2::read::DeflateDecoder::new(input),
                cap,
                "inflate",
            ))
        }
        _ => Err(format!("Unknown compress function: {}", name).into()),
    }
}

pub fn gzip(input: &[u8], level: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(level));
    encoder.write_all(input)?;
    Ok(encoder.finish()?)
}

/// Brotli at `quality` (0..=11; 5 is the web's usual trade of time for
/// bytes, 11 the smallest output). What `Content-Encoding: br` carries.
#[cfg(feature = "native")]
pub fn brotli(input: &[u8], quality: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut out, 4096, quality, 22);
        writer.write_all(input)?;
        writer.flush()?;
    }
    Ok(out)
}

/// The default limit on what one decode may produce: the smaller of
/// 1 GiB and 1000 times the input, so a decompression bomb (194 KB of
/// gzip that is 200 MB of zeros, or a 10 MB request body that is 10 GB)
/// fails fast instead of taking the process's memory (OL-620).
pub const DEFAULT_MAX_BYTES: u64 = 1 << 30;

fn default_cap(input_len: usize) -> u64 {
    DEFAULT_MAX_BYTES.min((input_len as u64).saturating_mul(1000))
}

/// A decoder's arguments: the data, and the limit from an optional
/// `#{ "max_bytes": n }` (or the default).
fn decode_args<'a>(args: &'a [Value], name: &str) -> Result<(&'a [u8], u64), String> {
    let opts = match args.len() {
        1 => None,
        2 => match &args[1] {
            Value::Map(m) => Some(m.as_ref().clone()),
            Value::Unit => None,
            r => match r.record_fields() {
                Some(f) => Some(f.clone()),
                None => {
                    return Err(format!(
                        "compress.{name}(data, opts): opts is a map like #{{ \"max_bytes\": n }}, got {}",
                        r.type_name()
                    ));
                }
            },
        },
        n => return Err(format!("compress.{name} expects 1 or 2 arguments, got {n}")),
    };
    let input = input_bytes(&args[..1], name, 1)?;
    let cap = match opts.as_ref().and_then(|o| o.get("max_bytes")) {
        None | Some(Value::Unit) => default_cap(input.len()),
        Some(Value::Integer(n)) if *n >= 0 => *n as u64,
        Some(_) => {
            return Err(format!(
                "compress.{name}: \"max_bytes\" is a whole number of bytes"
            ));
        }
    };
    Ok((input, cap))
}

/// Read a decoder to its end, but no further than `cap` bytes: one byte
/// past it is an Err rather than more memory.
fn capped(reader: impl Read, cap: u64, name: &str) -> Value {
    let mut out = Vec::new();
    match reader.take(cap.saturating_add(1)).read_to_end(&mut out) {
        Ok(_) if out.len() as u64 > cap => err(format!(
            "{name}: the decoded data is larger than max_bytes ({cap} bytes)"
        )),
        Ok(_) => Value::Ok(Box::new(bytes::to_value(out))),
        Err(e) => err(format!("{name}: {e}")),
    }
}

fn err(message: String) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(message))))
}

/// The first argument as bytes: a string's UTF-8, or a `Bytes` value.
fn input_bytes<'a>(args: &'a [Value], name: &str, arity: usize) -> Result<&'a [u8], String> {
    if args.len() != arity {
        return Err(format!(
            "compress.{name} expects {arity} argument(s), got {}",
            args.len()
        ));
    }
    match &args[0] {
        Value::String(s) => Ok(s.as_bytes()),
        other => bytes::bytes_of(other).map_err(|_| {
            format!(
                "compress.{name}: expected a string or Bytes, got {}",
                other.type_name()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Value {
        Value::String(Arc::new(text.to_string()))
    }

    #[test]
    fn gzip_round_trips_and_shrinks_repetitive_text() {
        let text = "abc ".repeat(2000);
        let packed = call_compress_function("gzip", vec![s(&text)]).unwrap();
        assert!(bytes::bytes_of(&packed).unwrap().len() < text.len() / 10);
        let back = call_compress_function("gunzip", vec![packed]).unwrap();
        match &back {
            Value::Ok(inner) => assert_eq!(bytes::bytes_of(inner).unwrap(), text.as_bytes()),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn deflate_round_trips_and_garbage_is_an_err() {
        let packed = call_compress_function("deflate", vec![s("hello hello hello")]).unwrap();
        let back = call_compress_function("inflate", vec![packed]).unwrap();
        assert!(matches!(back, Value::Ok(_)));
        let bad = call_compress_function("gunzip", vec![s("not gzip")]).unwrap();
        assert!(matches!(bad, Value::Err(_)));
    }

    /// OL-620: a decompression bomb stops at `max_bytes`, and the default
    /// cap (1000x the input) holds without one.
    #[test]
    fn a_decode_past_max_bytes_is_an_err() {
        let bomb = gzip(&vec![0u8; 2_000_000], 9).unwrap();
        let opts = |n| {
            let mut m = crate::ast::ValueMap::default();
            m.insert("max_bytes".to_string(), Value::Integer(n));
            Value::Map(Arc::new(m))
        };
        let message = |v: Value| match &v {
            Value::Err(e) => e.to_string(),
            other => panic!("expected Err, got {:?}", other),
        };
        let packed = bytes::to_value(bomb.clone());
        assert!(
            message(call_compress_function("gunzip", vec![packed.clone(), opts(1000)]).unwrap())
                .contains("larger than max_bytes")
        );
        assert!(matches!(
            call_compress_function("gunzip", vec![packed, opts(2_000_000)]).unwrap(),
            Value::Ok(_)
        ));
        // ~2 KB of gzip is 2 MB of zeros: past 1000x its size
        assert!(bomb.len() * 1000 < 2_000_000);
        assert!(
            message(call_compress_function("gunzip", vec![bytes::to_value(bomb)]).unwrap())
                .contains("larger than max_bytes")
        );
        let deflated = call_compress_function("deflate", vec![s(&"a".repeat(5000))]).unwrap();
        assert!(
            message(call_compress_function("inflate", vec![deflated, opts(10)]).unwrap())
                .contains("larger than max_bytes")
        );
    }

    #[cfg(feature = "native")]
    #[test]
    fn brotli_round_trips_at_every_quality() {
        let text = "olang ".repeat(3000);
        for q in [0, 5, 11] {
            let packed =
                call_compress_function("brotli_level", vec![s(&text), Value::Integer(q)]).unwrap();
            assert!(bytes::bytes_of(&packed).unwrap().len() < text.len() / 10);
            match &call_compress_function("unbrotli", vec![packed]).unwrap() {
                Value::Ok(inner) => assert_eq!(bytes::bytes_of(inner).unwrap(), text.as_bytes()),
                other => panic!("expected Ok, got {:?}", other),
            }
        }
        assert!(matches!(
            call_compress_function("unbrotli", vec![s("not brotli at all")]).unwrap(),
            Value::Err(_)
        ));
        assert!(call_compress_function("brotli_level", vec![s("x"), Value::Integer(12)]).is_err());
    }

    #[test]
    fn module_lists_its_functions() {
        // gzip, gzip_level, gunzip, deflate, inflate — and brotli,
        // brotli_level and unbrotli natively.
        let expected = if cfg!(feature = "native") { 8 } else { 5 };
        match &create_compress_module() {
            Value::Struct { fields, .. } => assert_eq!(fields.len(), expected),
            _ => panic!("module"),
        }
    }
}

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
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;

pub fn create_compress_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("gzip", 1),
        ("gunzip", 1),
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
        type_name: "Module".to_string(),
        fields: Arc::new(module),
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
        "gunzip" => {
            let input = input_bytes(&args, "gunzip", 1)?;
            let mut out = Vec::new();
            Ok(
                match flate2::read::GzDecoder::new(input).read_to_end(&mut out) {
                    Ok(_) => Value::Ok(Box::new(bytes::to_value(out))),
                    Err(e) => err(format!("gunzip: {}", e)),
                },
            )
        }
        "inflate" => {
            let input = input_bytes(&args, "inflate", 1)?;
            let mut out = Vec::new();
            Ok(
                match flate2::read::DeflateDecoder::new(input).read_to_end(&mut out) {
                    Ok(_) => Value::Ok(Box::new(bytes::to_value(out))),
                    Err(e) => err(format!("inflate: {}", e)),
                },
            )
        }
        _ => Err(format!("Unknown compress function: {}", name).into()),
    }
}

fn gzip(input: &[u8], level: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(level));
    encoder.write_all(input)?;
    Ok(encoder.finish()?)
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
        match back {
            Value::Ok(inner) => assert_eq!(bytes::bytes_of(&inner).unwrap(), text.as_bytes()),
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

    #[test]
    fn module_lists_its_functions() {
        match create_compress_module() {
            Value::Struct { fields, .. } => assert_eq!(fields.len(), 5),
            _ => panic!("module"),
        }
    }
}

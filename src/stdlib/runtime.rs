//! `runtime` — what this olang binary carries: its version and the
//! browser runtime it embeds (`runtime.wasm()`), so a program that serves
//! its own routes can hand a page the wasm, its hash, and its compressed
//! forms without a file on disk.

use crate::ast::{BuiltinFunction, Value};
use std::collections::HashMap;
use std::sync::Arc;

fn builtin(name: &str, arity: usize) -> Value {
    Value::Builtin(BuiltinFunction {
        name: format!("runtime.{}", name),
        arity,
    })
}

pub fn create_runtime_module() -> Value {
    let mut module = HashMap::new();
    module.insert("wasm".to_string(), builtin("wasm", 0));
    module.insert("version".to_string(), builtin("version", 0));
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

pub fn call_runtime_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "version" => Ok(Value::String(Arc::new(crate::version::VERSION.to_string()))),
        "wasm" => runtime_wasm(args),
        _ => Err(format!("Unknown runtime function: {}", name).into()),
    }
}

/// `runtime.wasm()` — `Ok(#{ "bytes", "hash", "gzip", "br", "version" })`:
/// the embedded browser runtime, its content hash (the name in
/// `/olang.<hash>.wasm`), its gzip and brotli forms, and the olang version
/// it is — or `Err` naming how to build a binary that carries one.
fn runtime_wasm(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("runtime.wasm expects 0 arguments, got {}", args.len()).into());
    }
    let Some(bytes) = crate::runtime_wasm::embedded() else {
        return Ok(Value::Err(Box::new(Value::String(Arc::new(
            "this olang binary carries no browser runtime: build the wasm first (`cargo xtask wasm`, or `make wasm`), then rebuild or reinstall olang (`make install`)"
                .to_string(),
        )))));
    };
    // Built once per process: the bytes values share their buffers, so a
    // program that asks per request pays a handle clone, not a copy of
    // the runtime and its compressed forms.
    static ANSWER: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    if let Some(answer) = ANSWER.get() {
        return Ok(Value::Ok(Box::new(answer.clone())));
    }
    let mut out = HashMap::new();
    out.insert(
        "bytes".to_string(),
        crate::stdlib::bytes::to_value(bytes.to_vec()),
    );
    out.insert(
        "hash".to_string(),
        Value::String(Arc::new(
            crate::runtime_wasm::hash().unwrap_or_default().to_string(),
        )),
    );
    out.insert(
        "gzip".to_string(),
        crate::runtime_wasm::gzipped()
            .map(|b| crate::stdlib::bytes::to_value(b.to_vec()))
            .unwrap_or(Value::Unit),
    );
    out.insert(
        "br".to_string(),
        crate::runtime_wasm::brotli()
            .map(|b| crate::stdlib::bytes::to_value(b.to_vec()))
            .unwrap_or(Value::Unit),
    );
    out.insert(
        "version".to_string(),
        Value::String(Arc::new(crate::version::VERSION.to_string())),
    );
    let answer = Value::Map(Arc::new(out));
    let answer = ANSWER.get_or_init(|| answer).clone();
    Ok(Value::Ok(Box::new(answer)))
}

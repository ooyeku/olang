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
    module.insert("memory".to_string(), builtin("memory", 0));
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
        "memory" => runtime_memory(args),
        _ => Err(format!("Unknown runtime function: {}", name).into()),
    }
}

/// `runtime.memory()` — `#{ "heap", "program", "values", "embedded",
/// "tasks": [#{ "name", "bytes" }] }`, in bytes: what is allocated and not
/// freed; the share the loaded program holds (the heap's growth across
/// parsing the entry file and each `use`); the rest; the browser runtime
/// the binary's image carries (file-backed, not heap); and each spawned
/// task and http worker with what it allocated and has not itself freed,
/// largest first. See `src/memory.rs` for how attribution works.
fn runtime_memory(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("runtime.memory expects 0 arguments, got {}", args.len()).into());
    }
    let heap = crate::memory::heap_bytes();
    let program = crate::memory::program_bytes().min(heap);
    let embedded = if crate::runtime_wasm::embedded().is_some() {
        crate::runtime_wasm::embedded_bytes()
    } else {
        0
    };
    let tasks: Vec<Value> = crate::memory::tasks()
        .into_iter()
        .map(|(name, bytes)| {
            let mut row = HashMap::new();
            row.insert("name".to_string(), Value::String(Arc::new(name)));
            row.insert("bytes".to_string(), Value::Integer(bytes as i64));
            Value::Map(Arc::new(row))
        })
        .collect();
    let mut out = HashMap::new();
    out.insert("heap".to_string(), Value::Integer(heap as i64));
    out.insert("program".to_string(), Value::Integer(program as i64));
    out.insert(
        "values".to_string(),
        Value::Integer((heap - program) as i64),
    );
    out.insert("embedded".to_string(), Value::Integer(embedded as i64));
    out.insert("tasks".to_string(), Value::List(Arc::new(tasks)));
    Ok(Value::Map(Arc::new(out)))
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
    // Built once per process, and every bytes value borrows the binary's
    // own image: a program that asks per request pays a handle clone, and
    // no form of the runtime is ever copied to the heap.
    static ANSWER: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    if let Some(answer) = ANSWER.get() {
        return Ok(Value::Ok(Box::new(answer.clone())));
    }
    let mut out = HashMap::new();
    out.insert(
        "bytes".to_string(),
        crate::stdlib::bytes::static_value(bytes),
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
            .map(crate::stdlib::bytes::static_value)
            .unwrap_or(Value::Unit),
    );
    out.insert(
        "br".to_string(),
        crate::runtime_wasm::brotli()
            .map(crate::stdlib::bytes::static_value)
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

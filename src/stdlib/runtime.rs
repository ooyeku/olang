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
    module.insert("profile_start".to_string(), builtin("profile_start", 0));
    module.insert("profile_stop".to_string(), builtin("profile_stop", 0));
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
        "profile_start" => runtime_profile_start(args),
        "profile_stop" => runtime_profile_stop(args),
        _ => Err(format!("Unknown runtime function: {}", name).into()),
    }
}

fn err_value(message: String) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(message))))
}

/// `runtime.profile_start()` or `runtime.profile_start(interval_us)` —
/// begin sampling this process with the profiler behind `olang profile`
/// (default 1,000 µs): every thread's olang call stack, by tier. `Ok(())`,
/// or `Err` when a profile is already running.
fn runtime_profile_start(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let interval_us = match args.as_slice() {
        [] => 1000,
        [Value::Integer(n)] if *n >= 50 => *n as u64,
        _ => {
            return Err(
                "runtime.profile_start expects no argument, or an interval in microseconds (50 or more)"
                    .into(),
            );
        }
    };
    Ok(match crate::profile::start_in_process(interval_us) {
        Ok(()) => Value::Ok(Box::new(Value::Unit)),
        Err(e) => err_value(e),
    })
}

/// `runtime.profile_stop()` — end the profile and answer `Ok(#{
/// "interval_us", "ticks", "idle", "blocked", "samples", "rows": [#{
/// "function", "tier", "samples", "share" }] })`: the functions that were
/// running when the sampler looked, most first, each with the tier it
/// ran on and its share of the samples that landed in code. `blocked`
/// counts thread-ticks spent parked — a receive, a sleep, a server
/// waiting for a connection — which are not charged to any function.
fn runtime_profile_stop(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!(
            "runtime.profile_stop expects 0 arguments, got {}",
            args.len()
        )
        .into());
    }
    let summary = match crate::profile::stop_in_process() {
        Ok(summary) => summary,
        Err(e) => return Ok(err_value(e)),
    };
    let rows: Vec<Value> = summary
        .rows
        .iter()
        .map(|(function, tier, samples)| {
            let mut row = HashMap::new();
            row.insert(
                "function".to_string(),
                Value::String(Arc::new(function.clone())),
            );
            row.insert(
                "tier".to_string(),
                Value::String(Arc::new(tier.to_string())),
            );
            row.insert("samples".to_string(), Value::Integer(*samples as i64));
            row.insert(
                "share".to_string(),
                Value::Float(*samples as f64 / summary.samples.max(1) as f64),
            );
            Value::Map(Arc::new(row))
        })
        .collect();
    let mut out = HashMap::new();
    out.insert(
        "interval_us".to_string(),
        Value::Integer(summary.interval_us as i64),
    );
    out.insert("ticks".to_string(), Value::Integer(summary.ticks as i64));
    out.insert("idle".to_string(), Value::Integer(summary.idle as i64));
    out.insert(
        "blocked".to_string(),
        Value::Integer(summary.blocked as i64),
    );
    out.insert(
        "samples".to_string(),
        Value::Integer(summary.samples as i64),
    );
    out.insert("rows".to_string(), Value::List(Arc::new(rows)));
    Ok(Value::Ok(Box::new(Value::Map(Arc::new(out)))))
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

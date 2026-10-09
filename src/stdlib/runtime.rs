//! `runtime` — what this olang binary carries: its version and the
//! browser runtime it embeds (`runtime.wasm()`), so a program that serves
//! its own routes can hand a page the wasm, its hash, and its compressed
//! forms without a file on disk.

use crate::ast::{BuiltinFunction, Value};
use std::sync::Arc;

fn builtin(name: &str, arity: usize) -> Value {
    Value::Builtin(BuiltinFunction {
        name: format!("runtime.{}", name),
        arity,
    })
}

pub fn create_runtime_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    module.insert("wasm".to_string(), builtin("wasm", 0));
    module.insert("version".to_string(), builtin("version", 0));
    module.insert("memory".to_string(), builtin("memory", 0));
    module.insert("profile_start".to_string(), builtin("profile_start", 0));
    module.insert("profile_stop".to_string(), builtin("profile_stop", 0));
    module.insert("profile_live_start".to_string(), builtin("profile_live_start", 1));
    module.insert("profile_live_stop".to_string(), builtin("profile_live_stop", 0));
    module.insert("build".to_string(), builtin("build", 0));
    // A host loading code it did not write (olang Studio's plugins):
    // see `call_host`.
    module.insert("load_module".to_string(), builtin("load_module", 2));
    module.insert("unload_module".to_string(), builtin("unload_module", 1));
    module.insert("module_grant".to_string(), builtin("module_grant", 1));
    module.insert("module_permits".to_string(), builtin("module_permits", 3));
    module.insert("call_budget".to_string(), builtin("call_budget", 3));
    module.insert("last_error".to_string(), builtin("last_error", 0));
    module.insert("shape".to_string(), builtin("shape", 1));
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
        "profile_live_start" => runtime_profile_live_start(args),
        "profile_live_stop" => runtime_profile_live_stop(args),
        "build" => runtime_build(args),
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
            let mut row = crate::ast::ValueMap::default();
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
    let mut out = crate::ast::ValueMap::default();
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

/// `runtime.profile_live_start(dir)` or `runtime.profile_live_start(dir,
/// #{ "every_ms", "interval_us", "label" })` — a live profile of this
/// process, as `olang profile --format json --live DIR` makes of a
/// program it runs: sampled from now (every `interval_us`, 1,000 by
/// default), its snapshot written to `DIR/<pid>.json` every `every_ms`
/// (500 by default, 50 at least) in the same format, cumulative and
/// replaced atomically, so a program can draw its own profile while it
/// runs (olang Studio's "Studio" view). `Ok(())`, or `Err` when a live
/// profile already runs (`--profile-live`, an earlier call) or another
/// profile does. Until it is called the process pays nothing for it.
fn runtime_profile_live_start(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let usage = "runtime.profile_live_start expects a directory, and optionally #{ \"every_ms\", \"interval_us\", \"label\" }";
    let (dir, opts) = match args.as_slice() {
        [Value::String(d)] => (d.to_string(), None),
        [Value::String(d), Value::Map(o)] => (d.to_string(), Some(o.clone())),
        _ => return Err(usage.into()),
    };
    let int_opt = |k: &str, default: u64, least: u64| -> Result<u64, String> {
        match opts.as_ref().and_then(|o| o.get(k)) {
            None | Some(Value::Unit) => Ok(default),
            Some(Value::Integer(n)) if *n >= least as i64 => Ok(*n as u64),
            Some(_) => Err(format!("runtime.profile_live_start: \"{k}\" is a whole number, {least} or more")),
        }
    };
    let every_ms = int_opt("every_ms", crate::profile_live::DEFAULT_EVERY_MS, 50)?;
    let interval_us = int_opt("interval_us", crate::profile_live::ATTACH_INTERVAL_US, 50)?;
    let label = match opts.as_ref().and_then(|o| o.get("label")) {
        Some(Value::String(s)) => s.to_string(),
        _ => std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
    };
    if crate::profile::busy() {
        return Ok(err_value("a profile is already running in this process".to_string()));
    }
    let dir = std::path::PathBuf::from(dir);
    let dir = std::path::absolute(&dir).unwrap_or(dir);
    let started = crate::profile_live::start_by_program(crate::profile_live::LiveSpec {
        dir: Some(dir),
        every_ms,
        from_start: true,
        interval_us,
        label,
    });
    Ok(if started {
        Value::Ok(Box::new(Value::Unit))
    } else {
        err_value("a live profile is already running in this process".to_string())
    })
}

/// `runtime.profile_live_stop()` — end the live profile
/// `runtime.profile_live_start` began: sampling stops, the final snapshot
/// (`"done": true`) is written, and the process pays nothing for it again.
/// `Ok(())`, or `Err` when none runs — or when the one running is not the
/// program's (`olang profile`'s, an armed run's: theirs to end).
fn runtime_profile_live_stop(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("runtime.profile_live_stop expects 0 arguments, got {}", args.len()).into());
    }
    Ok(if crate::profile_live::finish_by_program().is_some() {
        Value::Ok(Box::new(Value::Unit))
    } else {
        err_value("no live profile is running (runtime.profile_live_start starts one)".to_string())
    })
}

/// `runtime.build()` — what this binary is, for an About box or a bug
/// report: `#{ "version", "commit", "branch", "dirty", "date", "profile",
/// "target", "rustc", "features": [..], "crates": [#{ "name", "version",
/// "vendored" }], "exe" }`. `commit` is empty and `dirty` `()` when it
/// was not built from a git checkout; `date` is when its build script last
/// ran (UTC); `crates` the gui engine's and the stdlib's key dependencies,
/// as Cargo.lock had them (`vendored`: a patched copy, vendor/).
fn runtime_build(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err(format!("runtime.build expects 0 arguments, got {}", args.len()).into());
    }
    let s = |v: &str| Value::String(Arc::new(v.to_string()));
    let mut out = crate::ast::ValueMap::default();
    out.insert("version".to_string(), s(crate::version::VERSION));
    out.insert("commit".to_string(), s(crate::version::BUILD_COMMIT));
    out.insert("branch".to_string(), s(crate::version::BUILD_BRANCH));
    out.insert(
        "dirty".to_string(),
        match crate::version::BUILD_DIRTY {
            "true" => Value::Boolean(true),
            "false" => Value::Boolean(false),
            _ => Value::Unit,
        },
    );
    out.insert("date".to_string(), s(crate::version::BUILD_DATE));
    out.insert("profile".to_string(), s(crate::version::BUILD_PROFILE));
    out.insert("target".to_string(), s(crate::version::BUILD_TARGET));
    out.insert("rustc".to_string(), s(crate::version::BUILD_RUSTC));
    out.insert(
        "features".to_string(),
        Value::List(Arc::new(crate::version::features().into_iter().map(s).collect())),
    );
    let crates: Vec<Value> = crate::version::BUILD_CRATES
        .split(';')
        .filter(|c| !c.is_empty())
        .map(|c| {
            let (name, version) = c.split_once(' ').unwrap_or((c, ""));
            let vendored = version.contains("+vendored");
            let mut row = crate::ast::ValueMap::default();
            row.insert("name".to_string(), s(name));
            row.insert("version".to_string(), s(&version.replace("+vendored", "")));
            row.insert("vendored".to_string(), Value::Boolean(vendored));
            Value::Map(Arc::new(row))
        })
        .collect();
    out.insert("crates".to_string(), Value::List(Arc::new(crates)));
    out.insert(
        "exe".to_string(),
        std::env::current_exe().map(|p| s(&p.display().to_string())).unwrap_or(Value::Unit),
    );
    Ok(Value::Map(Arc::new(out)))
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
            let mut row = crate::ast::ValueMap::default();
            row.insert("name".to_string(), Value::String(Arc::new(name)));
            row.insert("bytes".to_string(), Value::Integer(bytes as i64));
            Value::Map(Arc::new(row))
        })
        .collect();
    let mut out = crate::ast::ValueMap::default();
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
    let mut out = crate::ast::ValueMap::default();
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

// ── a host's calls: modules loaded at run time ───────────────────────

fn host_err(message: impl Into<String>) -> crate::interpreter::InterpreterError {
    crate::interpreter::InterpreterError::RuntimeError {
        message: message.into(),
    }
}

fn str_arg<'a>(args: &'a [Value], i: usize, f: &str) -> Result<&'a str, crate::interpreter::InterpreterError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.as_str()),
        other => Err(host_err(format!(
            "runtime.{}: argument {} must be a String, got {}",
            f,
            i + 1,
            other.map(|v| v.type_name()).unwrap_or_else(|| "nothing".into())
        ))),
    }
}

/// The folder a module's grant covers, from a path to its file (or to
/// the folder itself): canonical.
fn module_dir(path: &str) -> std::path::PathBuf {
    let p = crate::caps::gate_path(path);
    if p.is_dir() {
        p
    } else {
        p.parent().map(|d| d.to_path_buf()).unwrap_or(p)
    }
}

fn map_value(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = crate::ast::ValueMap::default();
    for (k, v) in pairs {
        m.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(m))
}

/// The runtime functions that need the interpreter: a module loaded at
/// run time under its own grant (`load_module`, `unload_module`,
/// `module_grant`, `module_permits`), a call held to a time budget
/// (`call_budget`), the trace of the last error caught (`last_error`),
/// and a value's shape (`shape`). None for every other name.
pub fn call_host(
    name: &str,
    args: &[Value],
    interpreter: &mut crate::interpreter::Interpreter,
) -> Option<Result<Value, crate::interpreter::InterpreterError>> {
    Some(match name {
        "load_module" => host_load_module(args, interpreter),
        "unload_module" => (|| {
            let dir = module_dir(str_arg(args, 0, "unload_module")?);
            interpreter.forget_modules_under(&dir);
            Ok(Value::Boolean(crate::caps::unregister_module(&dir)))
        })(),
        "module_grant" => (|| {
            let dir = module_dir(str_arg(args, 0, "module_grant")?);
            Ok(crate::caps::module_at(&dir)
                .map(|g| crate::caps::grant_value(&g))
                .unwrap_or(Value::Unit))
        })(),
        "module_permits" => host_module_permits(args),
        "call_budget" => host_call_budget(args, interpreter),
        "last_error" => Ok(match crate::errtrace::last() {
            None => Value::Unit,
            Some((message, frames)) => map_value(vec![
                ("message", Value::String(Arc::new(message))),
                ("frames", crate::errtrace::frames_value(&frames)),
            ]),
        }),
        "shape" => match args.first() {
            Some(v) => Ok(Value::String(Arc::new(shape_of(v, 0)))),
            None => Err(host_err("runtime.shape expects a value")),
        },
        _ => return None,
    })
}

/// `runtime.load_module(path, grant)`: the olang file at `path` loaded
/// into this running program as a module — read afresh, with whatever it
/// imports from its own folder — its code (everything under the file's
/// folder) answering to `grant` (`#{ name, fs, fs_roots, proc, net, db,
/// env }`; a key left out is denied). `Ok(module)` (its shared names as
/// fields), or `Err(message)` — the module's grant as it was before, and
/// the error's frames in `runtime.last_error()`.
fn host_load_module(
    args: &[Value],
    interpreter: &mut crate::interpreter::Interpreter,
) -> Result<Value, crate::interpreter::InterpreterError> {
    let path = str_arg(args, 0, "load_module")?;
    let file = crate::caps::gate_path(path);
    if !file.is_file() {
        return Ok(err_value(format!("runtime.load_module: {} is not a file", file.display())));
    }
    let dir = file.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let default_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "module".to_string());
    let grant = match crate::caps::grant_from_value(dir.clone(), &default_name, args.get(1).unwrap_or(&Value::Unit)) {
        Ok(g) => g,
        Err(e) => return Ok(err_value(e)),
    };
    let before = crate::caps::register_module(grant);
    crate::errtrace::begin();
    match interpreter.load_module_at(&file) {
        Ok(module) => Ok(Value::Ok(Box::new(module))),
        Err(e) if crate::interpreter::Interpreter::is_control_signal(&e) => Err(e),
        Err(e) => {
            // The previous version's grant stands, as its code does.
            crate::caps::unregister_module(&dir);
            if let Some(old) = before {
                crate::caps::register_module(old);
            }
            interpreter.clear_pending_error();
            let message = crate::builtin::strip_error_prefixes(&e.to_string());
            crate::errtrace::caught(&message);
            Ok(err_value(message))
        }
    }
}

/// `runtime.module_permits(path, builtin, args)`: would the module whose
/// code is under `path` be allowed `builtin(args…)`? The same gate its own
/// call would meet — for a host performing an effect on a module's behalf
/// (a process it asked for). `Ok(())`, or `Err(why)`.
fn host_module_permits(args: &[Value]) -> Result<Value, crate::interpreter::InterpreterError> {
    let dir = module_dir(str_arg(args, 0, "module_permits")?);
    let builtin = str_arg(args, 1, "module_permits")?;
    let call_args: Vec<Value> = match args.get(2) {
        Some(Value::List(items)) => items.iter().cloned().collect(),
        Some(Value::Unit) | None => Vec::new(),
        Some(other) => vec![other.clone()],
    };
    let Some(grant) = crate::caps::module_at(&dir) else {
        return Ok(err_value(format!(
            "runtime.module_permits: no module is loaded from {}",
            dir.display()
        )));
    };
    if let Some(denied) = crate::caps::check(&grant.caps, builtin) {
        return Ok(err_value(format!(
            "capability '{}' denied: {} requires it, and module '{}' is granted {} (runtime.load_module)",
            denied,
            builtin,
            grant.name,
            grant.caps.summary()
        )));
    }
    if let Some(message) = crate::interpreter::Interpreter::scope_check(&grant, builtin, &call_args) {
        return Ok(err_value(message));
    }
    Ok(Value::Ok(Box::new(Value::Unit)))
}

/// `runtime.call_budget(f, args, ms)`: `f(args…)` with a deadline — past
/// `ms` its evaluation is stopped (as an interrupt, on this thread only)
/// — and its failures caught. Answers a Map: `ok`, `value` (when ok),
/// `error` and `frames` (when not), `over` (it ran out of time), `us`
/// (how long it took, in microseconds).
fn host_call_budget(
    args: &[Value],
    interpreter: &mut crate::interpreter::Interpreter,
) -> Result<Value, crate::interpreter::InterpreterError> {
    let f = match args.first() {
        Some(v @ (Value::Function(_) | Value::Builtin(_))) => v.clone(),
        other => {
            return Err(host_err(format!(
                "runtime.call_budget: argument 1 must be a function, got {}",
                other.map(|v| v.type_name()).unwrap_or_else(|| "nothing".into())
            )));
        }
    };
    let call_args: Vec<Value> = match args.get(1) {
        Some(Value::List(items)) => items.iter().cloned().collect(),
        Some(Value::Unit) | None => Vec::new(),
        Some(other) => {
            return Err(host_err(format!(
                "runtime.call_budget: argument 2 must be a List of arguments, got {}",
                other.type_name()
            )));
        }
    };
    let ms = match args.get(2) {
        Some(Value::Integer(n)) if *n > 0 => *n as u64,
        Some(Value::Float(x)) if *x > 0.0 => x.ceil() as u64,
        other => {
            return Err(host_err(format!(
                "runtime.call_budget: argument 3 must be a positive number of milliseconds, got {}",
                other.map(|v| v.to_string()).unwrap_or_else(|| "nothing".into())
            )));
        }
    };
    crate::errtrace::begin();
    let started = std::time::Instant::now();
    crate::interrupt::budget_begin(ms);
    let result = interpreter.call_function(f, call_args);
    let over = crate::interrupt::budget_end();
    let us = Value::Integer(started.elapsed().as_micros() as i64);
    match result {
        Ok(v) => Ok(map_value(vec![
            ("ok", Value::Boolean(true)),
            ("value", v),
            ("over", Value::Boolean(over)),
            ("us", us),
        ])),
        Err(e) if crate::interpreter::Interpreter::is_control_signal(&e) => Err(e),
        Err(e) => {
            interpreter.clear_pending_error();
            let raw = crate::builtin::strip_error_prefixes(&e.to_string());
            let message = if over && crate::interrupt::is_interrupt(&raw) {
                format!("stopped: over its time budget of {} ms", ms)
            } else {
                raw
            };
            let frames = crate::errtrace::caught(&message);
            Ok(map_value(vec![
                ("ok", Value::Boolean(false)),
                ("error", Value::String(Arc::new(message))),
                ("frames", crate::errtrace::frames_value(&frames)),
                ("over", Value::Boolean(over)),
                ("us", us),
            ]))
        }
    }
}

/// A value's shape: its kind, a map's keys with theirs, a list's first
/// item's, a struct's fields' — what a host compares to decide whether a
/// reloaded module's state can be kept. Depth-limited (6).
pub fn shape_of(v: &Value, depth: usize) -> String {
    if depth > 6 {
        return "…".to_string();
    }
    match v {
        Value::Map(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            format!(
                "{{{}}}",
                keys.iter()
                    .map(|k| format!("{}:{}", k, shape_of(m.get(*k).unwrap(), depth + 1)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::List(items) => match items.first() {
            Some(first) => format!("[{}]", shape_of(first, depth + 1)),
            None => "[]".to_string(),
        },
        Value::Struct { type_name, fields } => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            format!(
                "{}{{{}}}",
                type_name,
                keys.iter()
                    .map(|k| format!("{}:{}", k, shape_of(fields.get(*k).unwrap(), depth + 1)))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        Value::Ok(inner) => format!("Ok({})", shape_of(inner, depth + 1)),
        Value::Err(inner) => format!("Err({})", shape_of(inner, depth + 1)),
        other => other.type_name().to_string(),
    }
}

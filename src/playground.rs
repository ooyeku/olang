//! The playground entry point: olang compiled to WebAssembly.
//!
//! This module only exists on the non-native (wasm32) build. It exposes a
//! hand-rolled C-ABI boundary — no wasm-bindgen — that the page's worker
//! drives directly:
//!
//! - `olang_alloc(len) -> ptr` / `olang_dealloc(ptr, len)` — the caller
//!   allocates a buffer, writes UTF-8 source into wasm memory, and frees
//!   it after the call.
//! - `olang_run(ptr, len) -> ptr` — parses and evaluates the source with
//!   the bytecode tier enabled (the same execution model as the CLI) and
//!   returns a result buffer: 4 little-endian length bytes followed by a
//!   JSON object `{"output", "value", "error", "ms", "version"}`. Free it
//!   with `olang_result_free(ptr)`.
//!
//! Safety comes from the platform, not from trust in this code: the wasm
//! instance touches nothing but its own linear memory (no filesystem,
//! network, process, or DOM imports — the only imports are three host
//! functions: two clocks and an entropy source), and the page runs it
//! inside a Web Worker it terminates on timeout, which is what bounds
//! infinite loops.

use crate::ast::Value;
use crate::interpreter::Interpreter;
use crate::parser::Parser as OlangParser;

// The `random` module (and the crypto crates) ride on the getrandom crate,
// whose browser backend normally needs wasm-bindgen. Instead, the host
// hands us entropy through one import (crypto.getRandomValues under it).
// Two getrandom majors coexist in the wasm graph, each with its own
// custom-backend mechanism, both fed by the same import:
//
// - getrandom 0.4 (rand 0.10, bcrypt): selected by the
//   `--cfg getrandom_backend="custom"` rustflag (.cargo/config.toml sets it
//   for wasm32 builds); the backend is the `__getrandom_v03_custom` symbol
//   defined below (0.4 kept the v03 hook name for compatibility).
// - getrandom 0.2 (rand_core 0.6 era: rsa, aes-gcm, argon2): the "custom"
//   cargo feature plus `register_custom_getrandom!`.
#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_random_bytes(ptr: *mut u8, len: usize);
}

/// getrandom 0.4's custom backend: the entire wasm entropy path.
///
/// # Safety
/// getrandom calls this with a valid, writable `dest..dest+len`; the host
/// import fills exactly that range.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    unsafe { host_random_bytes(dest, len) };
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn hosted_getrandom(buf: &mut [u8]) -> Result<(), getrandom02::Error> {
    unsafe { host_random_bytes(buf.as_mut_ptr(), buf.len()) };
    Ok(())
}

#[cfg(target_arch = "wasm32")]
getrandom02::register_custom_getrandom!(hosted_getrandom);

// panic=abort still runs the panic hook before the trap reaches the host,
// so the hook stashes the message here and the page reads it back through
// `olang_last_panic` after catching the RuntimeError.
static LAST_PANIC: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            if let Ok(mut slot) = LAST_PANIC.lock() {
                *slot = Some(info.to_string());
            }
        }));
    });
}

/// Returns the last panic message as a result buffer (see `olang_run`), or
/// null if nothing panicked. Frees like any other result buffer.
#[unsafe(no_mangle)]
pub extern "C" fn olang_last_panic() -> *mut u8 {
    match LAST_PANIC.lock().ok().and_then(|mut s| s.take()) {
        Some(msg) => result_buffer(msg),
        None => std::ptr::null_mut(),
    }
}

fn run_source(source: &str) -> (String, Option<String>, Option<String>) {
    let parser = OlangParser::new();
    let program = match parser.parse(source) {
        Ok(p) => p,
        Err(e) => return (crate::output::drain_captured(), None, Some(e.to_string())),
    };

    let mut interpreter = Interpreter::new();
    interpreter.enable_bytecode_tier(1, false);

    match interpreter.eval_program(program) {
        Ok(value) => {
            let shown = match value {
                Value::Unit => None,
                other => Some(format!("{}", other)),
            };
            (crate::output::drain_captured(), shown, None)
        }
        Err(e) => (crate::output::drain_captured(), None, Some(e.to_string())),
    }
}

// ── the dom bridge: olang as a frontend language ───────────────────────
//
// Elements are opaque handles minted by the page. Strings cross the
// boundary as (ptr, len) into linear memory; the page returns strings by
// writing them into a buffer it allocates with olang_alloc, returning a
// 4-byte-length-prefixed pointer (freed by us). Event handlers live in
// SESSION's registry; the page calls olang_dispatch_event(id) and the
// persistent interpreter re-enters.

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_dom_query(sel: *const u8, len: usize) -> i64;
    fn host_dom_set_text(handle: i64, ptr: *const u8, len: usize);
    fn host_dom_get_text(handle: i64) -> *const u8;
    fn host_dom_set_html(handle: i64, ptr: *const u8, len: usize);
    fn host_dom_get_value(handle: i64) -> *const u8;
    fn host_dom_set_value(handle: i64, ptr: *const u8, len: usize);
    fn host_dom_on(handle: i64, event: *const u8, len: usize, callback_id: i64);
    fn host_dom_focus(handle: i64);
    fn host_dom_set_class(handle: i64, ptr: *const u8, len: usize);
    fn host_dom_fetch(
        method: *const u8,
        method_len: usize,
        path: *const u8,
        path_len: usize,
        body: *const u8,
        body_len: usize,
        callback_id: i64,
    );
    fn host_dom_get_attr(handle: i64, ptr: *const u8, len: usize) -> *const u8;
    fn host_dom_set_attr(handle: i64, n: *const u8, nl: usize, v: *const u8, vl: usize);
    fn host_dom_remove_attr(handle: i64, ptr: *const u8, len: usize);
    /// op: 0 add, 1 remove, 2 toggle
    fn host_dom_class_op(handle: i64, op: i64, ptr: *const u8, len: usize);
    fn host_dom_set_style(handle: i64, n: *const u8, nl: usize, v: *const u8, vl: usize);
    fn host_dom_measure(handle: i64) -> *const u8;
    fn host_dom_create(tag: *const u8, len: usize) -> i64;
    fn host_dom_append(parent: i64, child: i64);
    fn host_dom_remove(handle: i64);
    fn host_dom_scroll_into_view(handle: i64);
    fn host_dom_set_timeout(ms: f64, callback_id: i64);
    fn host_dom_set_interval(ms: f64, callback_id: i64) -> i64;
    fn host_dom_clear_interval(timer_id: i64);
    fn host_dom_request_frame(callback_id: i64);
    fn host_dom_draw(handle: i64, ptr: *const u8, len: usize);
    fn host_dom_draw_points(
        handle: i64,
        pts: *const f64,
        n: usize,
        style_ptr: *const u8,
        style_len: usize,
    );
    fn host_dom_on_frame(callback_id: i64);
    /// before == 0 appends at the end.
    fn host_dom_insert_before(parent: i64, child: i64, before: i64);
    fn host_dom_push_state(ptr: *const u8, len: usize);
    fn host_dom_location() -> *const u8;
    fn host_dom_on_route(callback_id: i64);
    fn host_dom_storage_get(ptr: *const u8, len: usize) -> *const u8;
    fn host_dom_storage_set(kp: *const u8, kl: usize, vp: *const u8, vl: usize);
    fn host_dom_storage_remove(ptr: *const u8, len: usize);
    fn host_dom_worker_spawn(ptr: *const u8, len: usize) -> i64;
    fn host_dom_worker_send(worker: i64, ptr: *const u8, len: usize);
    fn host_dom_worker_on(worker: i64, callback_id: i64);
    fn host_dom_worker_close(worker: i64);
    /// Inside a worker: post a value to the page.
    fn host_dom_post(ptr: *const u8, len: usize);
    /// Inside a worker: register the inbound-message handler.
    fn host_dom_on_message(callback_id: i64);
}

use std::cell::RefCell;

thread_local! {
    /// The persistent browser session: the interpreter that ran the
    /// program stays alive so event handlers can re-enter it.
    static SESSION: RefCell<Option<Interpreter>> = const { RefCell::new(None) };
    /// Registered handlers. Separate from SESSION because dom.on runs
    /// DURING the initial program run, before the interpreter is parked.
    static HANDLERS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
}

#[cfg(target_arch = "wasm32")]
fn read_host_string(ptr: *const u8) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe {
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(std::slice::from_raw_parts(ptr, 4));
        let len = u32::from_le_bytes(len_bytes) as usize;
        let s = String::from_utf8_lossy(std::slice::from_raw_parts(ptr.add(4), len)).into_owned();
        // The page allocated via olang_alloc(4 + len); free it.
        drop(Vec::from_raw_parts(ptr as *mut u8, 0, (4 + len).max(1)));
        s
    }
}

/// High bit on a fetch callback id: the page must deliver the response
/// through the JSON dispatch (parsed value) rather than as raw text.
pub const JSON_CALLBACK_BIT: i64 = 1 << 40;

/// A value as compact JSON, via the same serde bridge as json.stringify.
#[cfg(target_arch = "wasm32")]
fn value_to_json(value: &Value) -> Result<String, Box<dyn std::error::Error>> {
    match crate::stdlib::json::call_json_function("stringify", vec![value.clone()]) {
        Ok(Value::Ok(inner)) => match *inner {
            Value::String(s) => Ok(s.as_ref().clone()),
            other => Ok(format!("{}", other)),
        },
        _ => Err("dom: value cannot be serialized to JSON".into()),
    }
}

/// Dispatch for dom.* builtins on the wasm build.
#[cfg(target_arch = "wasm32")]
pub fn dom_call(name: &str, args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let handle = |v: &Value| -> Result<i64, Box<dyn std::error::Error>> {
        match v {
            Value::Integer(h) => Ok(*h),
            _ => Err("dom: expected an element handle".into()),
        }
    };
    let text = |v: &Value| -> Result<String, Box<dyn std::error::Error>> {
        match v {
            Value::String(s) => Ok(s.as_ref().clone()),
            other => Ok(format!("{}", other)),
        }
    };
    match (name, args.as_slice()) {
        ("query", [sel]) => {
            let s = text(sel)?;
            let h = unsafe { host_dom_query(s.as_ptr(), s.len()) };
            if h == 0 {
                Err(format!("dom.query: no element matches {:?}", s).into())
            } else {
                Ok(Value::Integer(h))
            }
        }
        ("set_text", [el, v]) => {
            let s = text(v)?;
            unsafe { host_dom_set_text(handle(el)?, s.as_ptr(), s.len()) };
            Ok(Value::Unit)
        }
        ("get_text", [el]) => Ok(Value::String(std::sync::Arc::new(read_host_string(
            unsafe { host_dom_get_text(handle(el)?) },
        )))),
        ("set_html", [el, v]) => {
            let s = text(v)?;
            unsafe { host_dom_set_html(handle(el)?, s.as_ptr(), s.len()) };
            Ok(Value::Unit)
        }
        ("value", [el]) => Ok(Value::String(std::sync::Arc::new(read_host_string(
            unsafe { host_dom_get_value(handle(el)?) },
        )))),
        ("set_value", [el, v]) => {
            let s = text(v)?;
            unsafe { host_dom_set_value(handle(el)?, s.as_ptr(), s.len()) };
            Ok(Value::Unit)
        }
        ("on", [el, event, callback]) => {
            let ev = text(event)?;
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            unsafe { host_dom_on(handle(el)?, ev.as_ptr(), ev.len(), id) };
            Ok(Value::Unit)
        }
        ("focus", [el]) => {
            unsafe { host_dom_focus(handle(el)?) };
            Ok(Value::Unit)
        }
        ("set_class", [el, v]) => {
            let s = text(v)?;
            unsafe { host_dom_set_class(handle(el)?, s.as_ptr(), s.len()) };
            Ok(Value::Unit)
        }
        ("fetch", [method, path, body, callback]) => {
            let (m, pa, b) = (text(method)?, text(path)?, text(body)?);
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            unsafe {
                host_dom_fetch(
                    m.as_ptr(),
                    m.len(),
                    pa.as_ptr(),
                    pa.len(),
                    b.as_ptr(),
                    b.len(),
                    id,
                )
            };
            Ok(Value::Unit)
        }
        ("get_attr", [el, n]) => {
            let a = text(n)?;
            Ok(Value::String(std::sync::Arc::new(read_host_string(
                unsafe { host_dom_get_attr(handle(el)?, a.as_ptr(), a.len()) },
            ))))
        }
        ("set_attr", [el, n, v]) => {
            let (a, b) = (text(n)?, text(v)?);
            unsafe { host_dom_set_attr(handle(el)?, a.as_ptr(), a.len(), b.as_ptr(), b.len()) };
            Ok(Value::Unit)
        }
        ("remove_attr", [el, n]) => {
            let a = text(n)?;
            unsafe { host_dom_remove_attr(handle(el)?, a.as_ptr(), a.len()) };
            Ok(Value::Unit)
        }
        ("class_add", [el, n]) | ("class_remove", [el, n]) | ("class_toggle", [el, n]) => {
            let op = match name {
                "class_add" => 0,
                "class_remove" => 1,
                _ => 2,
            };
            let a = text(n)?;
            unsafe { host_dom_class_op(handle(el)?, op, a.as_ptr(), a.len()) };
            Ok(Value::Unit)
        }
        ("set_style", [el, n, v]) => {
            let (a, b) = (text(n)?, text(v)?);
            unsafe { host_dom_set_style(handle(el)?, a.as_ptr(), a.len(), b.as_ptr(), b.len()) };
            Ok(Value::Unit)
        }
        ("measure", [el]) => {
            let raw = read_host_string(unsafe { host_dom_measure(handle(el)?) });
            // The page answers with a JSON rect; hand back a Map.
            match crate::stdlib::json::call_json_function(
                "parse",
                vec![Value::String(std::sync::Arc::new(raw))],
            ) {
                Ok(Value::Ok(inner)) => Ok(*inner),
                _ => Err("dom.measure: host returned an unreadable rect".into()),
            }
        }
        ("create", [tag]) => {
            let t = text(tag)?;
            let h = unsafe { host_dom_create(t.as_ptr(), t.len()) };
            if h == 0 {
                Err(format!("dom.create: cannot create element {:?}", t).into())
            } else {
                Ok(Value::Integer(h))
            }
        }
        ("append", [parent, child]) => {
            unsafe { host_dom_append(handle(parent)?, handle(child)?) };
            Ok(Value::Unit)
        }
        ("remove", [el]) => {
            unsafe { host_dom_remove(handle(el)?) };
            Ok(Value::Unit)
        }
        ("scroll_into_view", [el]) => {
            unsafe { host_dom_scroll_into_view(handle(el)?) };
            Ok(Value::Unit)
        }
        ("set_timeout", [ms, callback]) => {
            let ms = match ms {
                Value::Integer(n) => *n as f64,
                Value::Float(f) => *f,
                _ => return Err("dom.set_timeout expects a millisecond number".into()),
            };
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            unsafe { host_dom_set_timeout(ms, id) };
            Ok(Value::Unit)
        }
        ("set_interval", [ms, callback]) => {
            let ms = match ms {
                Value::Integer(n) => *n as f64,
                Value::Float(f) => *f,
                _ => return Err("dom.set_interval expects a millisecond number".into()),
            };
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            let timer = unsafe { host_dom_set_interval(ms, id) };
            Ok(Value::Integer(timer))
        }
        ("clear_interval", [timer]) => {
            unsafe { host_dom_clear_interval(handle(timer)?) };
            Ok(Value::Unit)
        }
        ("draw", [el, ops]) => {
            // The whole scene crosses the boundary once, as JSON; the
            // page replays it onto the canvas 2D context.
            let json = match crate::stdlib::json::call_json_function("stringify", vec![ops.clone()])
            {
                Ok(Value::Ok(inner)) => match *inner {
                    Value::String(s) => s.as_ref().clone(),
                    other => format!("{}", other),
                },
                _ => return Err("dom.draw: ops must be a list of draw operations".into()),
            };
            unsafe { host_dom_draw(handle(el)?, json.as_ptr(), json.len()) };
            Ok(Value::Unit)
        }
        ("draw_points", [el, xs, ys, style]) => {
            fn points_f64(v: &Value) -> Result<Vec<Option<f64>>, String> {
                use olang_ods::Scalar;
                if let Some(s) = crate::ods::series_of(v) {
                    return (0..s.len())
                        .map(|i| match s.scalar_at(i) {
                            Scalar::F64(f) => Ok(Some(f)),
                            Scalar::I64(n) => Ok(Some(n as f64)),
                            Scalar::Null => Ok(None),
                            _ => Err("dom.draw_points: coordinates must be numeric".to_string()),
                        })
                        .collect();
                }
                match v {
                    Value::List(items) => items
                        .iter()
                        .map(|it| match it {
                            Value::Float(f) => Ok(Some(*f)),
                            Value::Integer(n) => Ok(Some(*n as f64)),
                            Value::Unit => Ok(None),
                            other => Err(format!(
                                "dom.draw_points: coordinates must be numeric, got {}",
                                other.type_name()
                            )),
                        })
                        .collect(),
                    other => Err(format!(
                        "dom.draw_points: coordinates must be a Series or list, got {}",
                        other.type_name()
                    )),
                }
            }
            // The bulk path: coordinates cross as ONE packed f64 buffer
            // the page reads as a zero-copy typed-array view — no JSON,
            // no per-point boundary cost. Series are the fast lane;
            // plain lists work too. Nulls drop pairwise, like plot.
            let xv = points_f64(xs)?;
            let yv = points_f64(ys)?;
            if xv.len() != yv.len() {
                return Err(format!(
                    "dom.draw_points: x and y lengths differ ({} vs {})",
                    xv.len(),
                    yv.len()
                )
                .into());
            }
            let mut packed = Vec::with_capacity(xv.len() * 2);
            for i in 0..xv.len() {
                match (xv[i], yv[i]) {
                    (Some(x), Some(y)) => {
                        packed.push(x);
                        packed.push(y);
                    }
                    _ => {}
                }
            }
            let style_json =
                match crate::stdlib::json::call_json_function("stringify", vec![style.clone()]) {
                    Ok(Value::Ok(inner)) => match *inner {
                        Value::String(s) => s.as_ref().clone(),
                        other => format!("{}", other),
                    },
                    _ => return Err("dom.draw_points: style must be a map".into()),
                };
            unsafe {
                host_dom_draw_points(
                    handle(el)?,
                    packed.as_ptr(),
                    packed.len() / 2,
                    style_json.as_ptr(),
                    style_json.len(),
                )
            };
            Ok(Value::Unit)
        }
        ("on_frame", [callback]) => {
            // The persistent animation loop: register once, the page
            // re-arms requestAnimationFrame and dispatches every frame
            // (no per-frame handler registration).
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            unsafe { host_dom_on_frame(id) };
            Ok(Value::Unit)
        }
        ("insert_before", [parent, child, before]) => {
            let b = match before {
                Value::Integer(h) => *h,
                _ => 0,
            };
            unsafe { host_dom_insert_before(handle(parent)?, handle(child)?, b) };
            Ok(Value::Unit)
        }
        ("push_state", [path]) => {
            let p = text(path)?;
            unsafe { host_dom_push_state(p.as_ptr(), p.len()) };
            Ok(Value::Unit)
        }
        ("location", []) => {
            let raw = read_host_string(unsafe { host_dom_location() });
            match crate::stdlib::json::call_json_function(
                "parse",
                vec![Value::String(std::sync::Arc::new(raw))],
            ) {
                Ok(Value::Ok(inner)) => Ok(*inner),
                _ => Err("dom.location: host returned an unreadable location".into()),
            }
        }
        ("on_route", [callback]) => {
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            unsafe { host_dom_on_route(id) };
            Ok(Value::Unit)
        }
        ("storage_get", [key]) => {
            let k = text(key)?;
            Ok(Value::String(std::sync::Arc::new(read_host_string(
                unsafe { host_dom_storage_get(k.as_ptr(), k.len()) },
            ))))
        }
        ("storage_set", [key, val]) => {
            let (k, v) = (text(key)?, text(val)?);
            unsafe { host_dom_storage_set(k.as_ptr(), k.len(), v.as_ptr(), v.len()) };
            Ok(Value::Unit)
        }
        ("storage_remove", [key]) => {
            let k = text(key)?;
            unsafe { host_dom_storage_remove(k.as_ptr(), k.len()) };
            Ok(Value::Unit)
        }
        ("worker", [src]) => {
            let p = text(src)?;
            let h = unsafe { host_dom_worker_spawn(p.as_ptr(), p.len()) };
            if h == 0 {
                Err(format!("dom.worker: cannot spawn worker for {:?}", p).into())
            } else {
                Ok(Value::Integer(h))
            }
        }
        ("worker_send", [worker, value]) => {
            let json = value_to_json(value)?;
            unsafe { host_dom_worker_send(handle(worker)?, json.as_ptr(), json.len()) };
            Ok(Value::Unit)
        }
        ("worker_on", [worker, callback]) => {
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            unsafe { host_dom_worker_on(handle(worker)?, id) };
            Ok(Value::Unit)
        }
        ("worker_close", [worker]) => {
            unsafe { host_dom_worker_close(handle(worker)?) };
            Ok(Value::Unit)
        }
        ("post", [value]) => {
            let json = value_to_json(value)?;
            unsafe { host_dom_post(json.as_ptr(), json.len()) };
            Ok(Value::Unit)
        }
        ("on_message", [callback]) => {
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            unsafe { host_dom_on_message(id) };
            Ok(Value::Unit)
        }
        ("fetch_json", [method, path, body, callback]) => {
            // fetch, but the handler receives the parsed value instead of
            // raw text — the JSON dispatch does the parsing.
            let (m, pa, b) = (text(method)?, text(path)?, text(body)?);
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            // Reuse host_dom_fetch; the page routes fetch_json callbacks
            // through the JSON dispatch (id offset marks them).
            unsafe {
                host_dom_fetch(
                    m.as_ptr(),
                    m.len(),
                    pa.as_ptr(),
                    pa.len(),
                    b.as_ptr(),
                    b.len(),
                    id | JSON_CALLBACK_BIT,
                )
            };
            Ok(Value::Unit)
        }
        ("request_frame", [callback]) => {
            let id = HANDLERS.with(|h| {
                let mut h = h.borrow_mut();
                h.push(callback.clone());
                (h.len() - 1) as i64
            });
            unsafe { host_dom_request_frame(id) };
            Ok(Value::Unit)
        }
        _ => Err(format!("dom.{}: unknown function or wrong arity", name).into()),
    }
}

/// Run a program and KEEP the interpreter alive as the page's session,
/// so dom.on handlers can re-enter it. Result buffer as olang_run.
///
/// # Safety
/// Same contract as olang_run.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn olang_session_start(ptr: *const u8, len: usize) -> *mut u8 {
    install_panic_hook();
    unsafe {
        let source = match std::str::from_utf8(std::slice::from_raw_parts(ptr, len)) {
            Ok(s) => s.to_string(),
            Err(_) => {
                return result_buffer(
                    r#"{"output":"","value":null,"error":"source was not valid UTF-8","ms":0}"#
                        .to_string(),
                );
            }
        };
        SESSION.with(|s| *s.borrow_mut() = None);
        HANDLERS.with(|h| h.borrow_mut().clear());
        let started = crate::clock::Instant::now();
        let parser = OlangParser::new();
        let (output, value, error) = match parser.parse(&source) {
            Err(e) => (crate::output::drain_captured(), None, Some(e.to_string())),
            Ok(program) => {
                let mut interpreter = Interpreter::new();
                interpreter.enable_bytecode_tier(1, false);
                let r = match interpreter.eval_program(program) {
                    Ok(v) => {
                        let shown = match v {
                            Value::Unit => None,
                            other => Some(format!("{}", other)),
                        };
                        (crate::output::drain_captured(), shown, None)
                    }
                    Err(e) => (crate::output::drain_captured(), None, Some(e.to_string())),
                };
                SESSION.with(|s| *s.borrow_mut() = Some(interpreter));
                r
            }
        };
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let json = format!(
            r#"{{"output":{},"value":{},"error":{},"ms":{:.1},"version":{}}}"#,
            json_escape(&output),
            value
                .as_deref()
                .map(json_escape)
                .unwrap_or_else(|| "null".to_string()),
            error
                .as_deref()
                .map(json_escape)
                .unwrap_or_else(|| "null".to_string()),
            ms,
            json_escape(crate::version::VERSION),
        );
        result_buffer(json)
    }
}

/// Re-enter the session for one event. Returns a result buffer whose
/// JSON carries any printed output and error from the handler.
///
/// # Safety
/// Called by the page with an id previously given to host_dom_on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn olang_dispatch_event(callback_id: i64) -> *mut u8 {
    unsafe { olang_dispatch_event_with(callback_id, std::ptr::null(), 0) }
}

/// Re-enter the session for one event carrying a string payload (fetch
/// responses, input values). The handler's arity decides: 1-parameter
/// handlers receive the payload, 0-parameter handlers ignore it.
///
/// # Safety
/// `ptr`, when non-null, points at `len` bytes of UTF-8 the page wrote
/// into wasm memory via olang_alloc (we free nothing — caller deallocs).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn olang_dispatch_event_with(
    callback_id: i64,
    ptr: *const u8,
    len: usize,
) -> *mut u8 {
    let handler = HANDLERS.with(|h| h.borrow().get(callback_id as usize).cloned());
    let outcome = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let Some(interpreter) = s.as_mut() else {
            return Err("no active session".to_string());
        };
        let Some(handler) = handler else {
            return Err(format!("unknown handler id {}", callback_id));
        };
        let arity = match &handler {
            Value::Function(f) => f.parameters.len(),
            _ => 0,
        };
        let args = if arity >= 1 {
            let payload = if ptr.is_null() {
                String::new()
            } else {
                String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
                    .into_owned()
            };
            vec![Value::String(std::sync::Arc::new(payload))]
        } else {
            Vec::new()
        };
        interpreter
            .call_function(handler, args)
            .map(|_| ())
            .map_err(|e| e.to_string())
    });
    let output = crate::output::drain_captured();
    let json = match outcome {
        Ok(()) => format!(
            r#"{{"output":{},"value":null,"error":null,"ms":0}}"#,
            json_escape(&output)
        ),
        Err(e) => format!(
            r#"{{"output":{},"value":null,"error":{},"ms":0}}"#,
            json_escape(&output),
            json_escape(&e)
        ),
    };
    result_buffer(json)
}

/// Re-enter the session for one STRUCTURED event: the page delivers a
/// JSON object (type, target id, value, key, pointer coordinates,
/// modifiers, data-* attributes — or a frame's delta), parsed here into
/// the Map the handler receives. Unparseable payloads fall back to the
/// raw string rather than dropping the event.
///
/// # Safety
/// `ptr`, when non-null, points at `len` bytes of UTF-8 the page wrote
/// into wasm memory via olang_alloc (caller deallocates).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn olang_dispatch_event_json(
    callback_id: i64,
    ptr: *const u8,
    len: usize,
) -> *mut u8 {
    let handler = HANDLERS.with(|h| h.borrow().get(callback_id as usize).cloned());
    let outcome = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let Some(interpreter) = s.as_mut() else {
            return Err("no active session".to_string());
        };
        let Some(handler) = handler else {
            return Err(format!("unknown handler id {}", callback_id));
        };
        let arity = match &handler {
            Value::Function(f) => f.parameters.len(),
            _ => 0,
        };
        let args = if arity >= 1 {
            let raw = if ptr.is_null() {
                String::new()
            } else {
                String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
                    .into_owned()
            };
            let event = match crate::stdlib::json::call_json_function(
                "parse",
                vec![Value::String(std::sync::Arc::new(raw.clone()))],
            ) {
                Ok(Value::Ok(inner)) => *inner,
                _ => Value::String(std::sync::Arc::new(raw)),
            };
            vec![event]
        } else {
            Vec::new()
        };
        interpreter
            .call_function(handler, args)
            .map(|_| ())
            .map_err(|e| e.to_string())
    });
    let output = crate::output::drain_captured();
    let json = match outcome {
        Ok(()) => format!(
            r#"{{"output":{},"value":null,"error":null,"ms":0}}"#,
            json_escape(&output)
        ),
        Err(e) => format!(
            r#"{{"output":{},"value":null,"error":{},"ms":0}}"#,
            json_escape(&output),
            json_escape(&e)
        ),
    };
    result_buffer(json)
}

fn json_escape(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// Build the length-prefixed JSON result buffer and leak it to the caller.
fn result_buffer(json: String) -> *mut u8 {
    let bytes = json.into_bytes();
    let len = bytes.len() as u32;
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&bytes);
    let boxed = out.into_boxed_slice();
    Box::into_raw(boxed) as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn olang_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` must come from `olang_alloc(len)` with the same `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn olang_dealloc(ptr: *mut u8, len: usize) {
    unsafe {
        if !ptr.is_null() {
            drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
        }
    }
}

/// # Safety
/// `ptr..ptr+len` must be valid UTF-8 written by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn olang_run(ptr: *const u8, len: usize) -> *mut u8 {
    unsafe {
        install_panic_hook();
        let source = match std::str::from_utf8(std::slice::from_raw_parts(ptr, len)) {
            Ok(s) => s,
            Err(_) => {
                return result_buffer(
                    r#"{"output":"","value":null,"error":"source was not valid UTF-8","ms":0}"#
                        .to_string(),
                );
            }
        };

        let started = crate::clock::Instant::now();
        let (output, value, error) = run_source(source);
        let ms = started.elapsed().as_secs_f64() * 1000.0;

        let json = format!(
            r#"{{"output":{},"value":{},"error":{},"ms":{:.1},"version":{}}}"#,
            json_escape(&output),
            value
                .as_deref()
                .map(json_escape)
                .unwrap_or_else(|| "null".to_string()),
            error
                .as_deref()
                .map(json_escape)
                .unwrap_or_else(|| "null".to_string()),
            ms,
            json_escape(crate::version::VERSION),
        );
        result_buffer(json)
    }
}

/// # Safety
/// `ptr` must come from `olang_run` and be freed exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn olang_result_free(ptr: *mut u8) {
    unsafe {
        if ptr.is_null() {
            return;
        }
        let mut len_bytes = [0u8; 4];
        len_bytes.copy_from_slice(std::slice::from_raw_parts(ptr, 4));
        let total = 4 + u32::from_le_bytes(len_bytes) as usize;
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
            ptr, total,
        )));
    }
}

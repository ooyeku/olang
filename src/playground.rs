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

// The `random` module rides on the getrandom crate, whose browser backend
// normally needs wasm-bindgen. Instead, its "custom" feature lets the host
// hand us entropy through one more import (crypto.getRandomValues under it).
#[cfg(target_arch = "wasm32")]
extern "C" {
    fn host_random_bytes(ptr: *mut u8, len: usize);
}

#[cfg(target_arch = "wasm32")]
fn hosted_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    unsafe { host_random_bytes(buf.as_mut_ptr(), buf.len()) };
    Ok(())
}

#[cfg(target_arch = "wasm32")]
getrandom::register_custom_getrandom!(hosted_getrandom);

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
#[no_mangle]
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
extern "C" {
    fn host_dom_query(sel: *const u8, len: usize) -> i64;
    fn host_dom_set_text(handle: i64, ptr: *const u8, len: usize);
    fn host_dom_get_text(handle: i64) -> *const u8;
    fn host_dom_set_html(handle: i64, ptr: *const u8, len: usize);
    fn host_dom_get_value(handle: i64) -> *const u8;
    fn host_dom_set_value(handle: i64, ptr: *const u8, len: usize);
    fn host_dom_on(handle: i64, event: *const u8, len: usize, callback_id: i64);
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
        _ => Err(format!("dom.{}: unknown function or wrong arity", name).into()),
    }
}

/// Run a program and KEEP the interpreter alive as the page's session,
/// so dom.on handlers can re-enter it. Result buffer as olang_run.
///
/// # Safety
/// Same contract as olang_run.
#[no_mangle]
pub unsafe extern "C" fn olang_session_start(ptr: *const u8, len: usize) -> *mut u8 {
    let source = match std::str::from_utf8(std::slice::from_raw_parts(ptr, len)) {
        Ok(s) => s.to_string(),
        Err(_) => {
            return result_buffer(
                r#"{"output":"","value":null,"error":"source was not valid UTF-8","ms":0}"#
                    .to_string(),
            )
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

/// Re-enter the session for one event. Returns a result buffer whose
/// JSON carries any printed output and error from the handler.
///
/// # Safety
/// Called by the page with an id previously given to host_dom_on.
#[no_mangle]
pub unsafe extern "C" fn olang_dispatch_event(callback_id: i64) -> *mut u8 {
    let handler = HANDLERS.with(|h| h.borrow().get(callback_id as usize).cloned());
    let outcome = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let Some(interpreter) = s.as_mut() else {
            return Err("no active session".to_string());
        };
        let Some(handler) = handler else {
            return Err(format!("unknown handler id {}", callback_id));
        };
        interpreter
            .call_function(handler, vec![])
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

#[no_mangle]
pub extern "C" fn olang_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` must come from `olang_alloc(len)` with the same `len`.
#[no_mangle]
pub unsafe extern "C" fn olang_dealloc(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        drop(Vec::from_raw_parts(ptr, 0, len.max(1)));
    }
}

/// # Safety
/// `ptr..ptr+len` must be valid UTF-8 written by the caller.
#[no_mangle]
pub unsafe extern "C" fn olang_run(ptr: *const u8, len: usize) -> *mut u8 {
    install_panic_hook();
    let source = match std::str::from_utf8(std::slice::from_raw_parts(ptr, len)) {
        Ok(s) => s,
        Err(_) => {
            return result_buffer(
                r#"{"output":"","value":null,"error":"source was not valid UTF-8","ms":0}"#
                    .to_string(),
            )
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

/// # Safety
/// `ptr` must come from `olang_run` and be freed exactly once.
#[no_mangle]
pub unsafe extern "C" fn olang_result_free(ptr: *mut u8) {
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

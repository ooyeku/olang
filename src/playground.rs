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

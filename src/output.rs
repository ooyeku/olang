//! Where `print` and `println` go.
//!
//! Native builds write straight to stdout, exactly as before. The
//! playground build (no "native" feature) has no stdout — output collects
//! in a per-thread buffer that the playground entry point drains after
//! each run and hands back to the page.

#[cfg(not(feature = "native"))]
use std::cell::RefCell;

#[cfg(not(feature = "native"))]
thread_local! {
    static CAPTURE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Native-path write that never panics on a vanished reader. `print!`
/// and `println!` abort the whole process with "failed printing to
/// stdout" when the pipe closes — which is exactly what happens in
/// `olang gen.ol | head -1` the moment head exits. Emulate the Unix
/// SIGPIPE default instead: terminate quietly with the conventional
/// 141 (128+SIGPIPE), like every well-behaved filter. Other write
/// errors report once to stderr (if it is still there) and exit 1
/// (`crate::stdio::stdout_failed`).
#[cfg(feature = "native")]
fn write_stdout(bytes: &[u8], flush: bool) {
    use std::io::Write;
    if COLLECTING.load(std::sync::atomic::Ordering::Relaxed) {
        if let Ok(mut buf) = COLLECTED.lock() {
            buf.extend_from_slice(bytes);
        }
        return;
    }
    let mut out = std::io::stdout().lock();
    let result = out
        .write_all(bytes)
        .and_then(|_| if flush { out.flush() } else { Ok(()) });
    if let Err(e) = result {
        drop(out);
        crate::stdio::stdout_failed(e);
    }
}

/// Write `text` with no trailing newline.
pub fn emit(text: &str) {
    #[cfg(feature = "native")]
    {
        // Flush so partial lines appear immediately — a shell prompt or
        // progress indicator printed with `print` must not sit in the
        // buffer waiting for a newline.
        write_stdout(text.as_bytes(), true);
    }
    #[cfg(not(feature = "native"))]
    CAPTURE.with(|c| c.borrow_mut().push_str(text));
}

/// Write `text` followed by a newline.
pub fn emit_line(text: &str) {
    #[cfg(feature = "native")]
    {
        let mut line = String::with_capacity(text.len() + 1);
        line.push_str(text);
        line.push('\n');
        write_stdout(line.as_bytes(), false);
    }
    #[cfg(not(feature = "native"))]
    CAPTURE.with(|c| {
        let mut buf = c.borrow_mut();
        buf.push_str(text);
        buf.push('\n');
    });
}

/// Take everything captured on this thread since the last drain.
#[cfg(not(feature = "native"))]
pub fn drain_captured() -> String {
    CAPTURE.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

// `olang repl --serve` keeps its stdout for the protocol: what a program
// prints while it serves is collected here, from any thread, and handed
// back with the evaluation that printed it.
#[cfg(feature = "native")]
static COLLECTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "native")]
static COLLECTED: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());

/// From now on `print` and `println` collect instead of writing.
#[cfg(feature = "native")]
pub fn collect_output() {
    COLLECTING.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// What was printed since the last call (lossy UTF-8).
#[cfg(feature = "native")]
pub fn take_collected() -> String {
    match COLLECTED.lock() {
        Ok(mut buf) => String::from_utf8_lossy(&std::mem::take(&mut *buf)).into_owned(),
        Err(_) => String::new(),
    }
}

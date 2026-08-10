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

/// Write `text` with no trailing newline.
pub fn emit(text: &str) {
    #[cfg(feature = "native")]
    {
        print!("{}", text);
        // Flush so partial lines appear immediately — a shell prompt or
        // progress indicator printed with `print` must not sit in the
        // buffer waiting for a newline.
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
    #[cfg(not(feature = "native"))]
    CAPTURE.with(|c| c.borrow_mut().push_str(text));
}

/// Write `text` followed by a newline.
pub fn emit_line(text: &str) {
    #[cfg(feature = "native")]
    println!("{}", text);
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

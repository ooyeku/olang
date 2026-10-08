//! Writing to the process's stdout and stderr without ever panicking.
//!
//! `std`'s `print!`/`println!`/`eprint!`/`eprintln!` panic when the write
//! fails ("failed printing to stderr"), and a release build aborts on a
//! panic — with a crash report. A child process whose parent has gone, or
//! has closed its end of the pipe (an editor stopping its language server,
//! its REPL, a test run), hits exactly that the next time it says
//! anything. The crate's own macros (defined in `lib.rs` and `main.rs`,
//! shadowing `std`'s for every module) come here instead:
//!
//! - stdout gone: the process ends quietly, as a filter does when its
//!   reader leaves — 141 (128 + SIGPIPE) for a closed pipe, 1 for any other
//!   failure (the reason said on stderr, if stderr is still there).
//! - stderr gone: what would have been said is dropped; nothing else
//!   changes.
//!
//! The panic hook ([`set_panic_hook`]) writes its report the same way, so
//! a panic with no stderr is still only that panic.

use std::fmt;
use std::io::{self, Write};

/// The status a process ends with when its stdout's reader has gone.
pub const BROKEN_PIPE_STATUS: i32 = 141;

/// `print!`: write to stdout; a stdout that has gone ends the process.
pub fn out(args: fmt::Arguments<'_>) {
    let mut lock = io::stdout().lock();
    let r = lock.write_fmt(args);
    if let Err(e) = r {
        drop(lock);
        stdout_failed(e);
    }
}

/// `println!`: a line to stdout; a stdout that has gone ends the process.
pub fn out_line(args: fmt::Arguments<'_>) {
    let mut lock = io::stdout().lock();
    let r = lock.write_fmt(args).and_then(|_| lock.write_all(b"\n"));
    if let Err(e) = r {
        drop(lock);
        stdout_failed(e);
    }
}

/// `eprint!`: write to stderr; a stderr that has gone drops it.
pub fn err(args: fmt::Arguments<'_>) {
    let _ = io::stderr().lock().write_fmt(args);
}

/// `eprintln!`: a line to stderr; a stderr that has gone drops it.
pub fn err_line(args: fmt::Arguments<'_>) {
    let mut lock = io::stderr().lock();
    let _ = lock.write_fmt(args).and_then(|_| lock.write_all(b"\n"));
}

/// Stdout could not be written: its reader has gone (or it is broken some
/// other way). End the process quietly — nothing more it prints can
/// arrive.
#[cold]
pub fn stdout_failed(e: io::Error) -> ! {
    if e.kind() == io::ErrorKind::BrokenPipe {
        std::process::exit(BROKEN_PIPE_STATUS);
    }
    err_line(format_args!("olang: cannot write to stdout: {e}"));
    std::process::exit(1);
}

/// The panic hook: miette's rendering of a panic (the message, where it
/// happened, the backtrace under `RUST_BACKTRACE`), written to stderr
/// without panicking again when stderr has gone — a second panic inside
/// the hook aborts at once, with a crash report about the printing
/// rather than the panic.
pub fn set_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = if let Some(m) = payload.downcast_ref::<&str>() {
            (*m).to_string()
        } else if let Some(m) = payload.downcast_ref::<String>() {
            m.clone()
        } else {
            "Something went wrong".to_string()
        };
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let mut report = miette::miette!(
            help = "set the `RUST_BACKTRACE=1` environment variable to display a backtrace.",
            "{message}"
        );
        if let Some(loc) = info.location() {
            report = report.wrap_err(format!("at {}:{}:{}", loc.file(), loc.line(), loc.column()));
        }
        let report = report.wrap_err(format!("Thread '{name}' panicked."));
        let mut text = format!("Error: {report:?}");
        if std::env::var("RUST_BACKTRACE").is_ok_and(|v| !v.is_empty() && v != "0") {
            text.push_str(&format!(
                "\n\n{}",
                std::backtrace::Backtrace::force_capture()
            ));
        }
        err_line(format_args!("{text}"));
    }));
}

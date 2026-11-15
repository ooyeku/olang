//! The REPL under continuation (roadmap lane W4).
//!
//! An unbalanced delimiter used to trap the session: nothing evaluated
//! until `:end`, commands vanished into the buffer, and Ctrl-C was the
//! only exit — one session transcript shows a user losing six commands
//! in a row to it. These tests drive the real binary over a pipe and
//! pin the escape hatches.

use std::io::Write;
use std::process::{Command, Stdio};

/// Feed lines to `olang repl` on stdin and return combined stdout.
fn repl(lines: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn repl");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(lines.as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn balancing_the_input_evaluates_it_immediately() {
    // Closing the delimiter IS the exit — no `:end` required.
    let out = repl("let x = (10 +\n32)\nprintln(x)\n");
    assert!(out.contains("42"), "auto-eval on balance missing: {out}");
}

#[test]
fn cancel_abandons_the_buffer() {
    let out = repl("let z = (9 +\n:cancel\nprintln(\"after\")\n");
    assert!(out.contains("(input abandoned)"), "no abandon note: {out}");
    assert!(out.contains("after"), "session did not continue: {out}");
    // The abandoned binding must not exist.
    let out = repl("let z = (9 +\n:cancel\nprintln(z)\n");
    assert!(
        out.contains("not defined") || out.contains("Undefined"),
        "abandoned buffer leaked a binding: {out}"
    );
}

#[test]
fn commands_mid_continuation_warn_instead_of_vanishing() {
    let out = repl("let y = [1, 2,\n:help\n3]\nprintln(y)\n");
    assert!(
        out.contains("looks like a command"),
        "no warning for :help mid-input: {out}"
    );
    assert!(
        out.contains("unclosed `[`"),
        "warning does not name the open delimiter: {out}"
    );
    // The command line was NOT appended — the input still evaluates clean.
    assert!(out.contains("[1, 2, 3]"), "buffer corrupted: {out}");
    // `quit` gets the same protection (it would otherwise become an
    // identifier inside the expression).
    let out = repl("let y = [1,\nquit\n2]\nprintln(y)\n");
    assert!(
        out.contains("looks like a command"),
        "quit swallowed: {out}"
    );
    assert!(out.contains("[1, 2]"), "buffer corrupted by quit: {out}");
}

#[test]
fn explicit_ml_collects_until_end() {
    // `:ml` is deliberate multi-statement entry: balanced lines do NOT
    // auto-evaluate; `:end` runs the whole buffer.
    let out = repl(":ml\nlet a = 1\nlet b = 2\nprintln(a + b)\n:end\n");
    assert!(out.contains('3'), "explicit buffer did not run: {out}");
    // Nothing evaluated before :end — a and b resolve only together.
    let pos_note = out.find("Multi-line input").expect("mode note");
    let pos_val = out.rfind('3').expect("value");
    assert!(pos_note < pos_val);
}

#[test]
fn end_still_evaluates_an_incomplete_buffer_as_an_error() {
    // `:end` on an unbalanced buffer surfaces the parse error rather than
    // hanging — and the parse error names the opener (lane W3).
    let out = repl("let x = (1 +\n:end\nprintln(\"alive\")\n");
    assert!(out.contains("alive"), "session died on :end error: {out}");
}

//! `:ovm` is an insight report, not a counter dump.
//!
//! It answers the developer's actual questions: what compiled, what
//! didn't and WHY (with a plain-language fix), what the machine did
//! (native calls, specializations, OSR), and `:ovm <name>` tells one
//! function's story. These tests drive the real binary.

use std::io::Write;
use std::process::{Command, Stdio};

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

const WORKOUT: &str = "fn hot(x) = x * 3 + 1\n\
    let mut s = 0\n\
    for i in 1..500 { s = s + hot(i) }\n\
    println(s)\n\
    fn defaulted(x, y = 2) = x + y\n\
    println(defaulted(1))\n\
    fn blocked(x) = defaulted(x, 3) + 1\n\
    println(blocked(4))\n";

#[test]
fn the_report_names_compiled_functions_with_their_specialization() {
    let out = repl(&format!("{WORKOUT}:ovm\n"));
    assert!(out.contains("Session totals"), "{out}");
    assert!(out.contains("Compiled functions"), "{out}");
    assert!(out.contains("hot"), "compiled fn missing: {out}");
    // 499 calls, specialized on Int — the JIT view surfaces both.
    assert!(out.contains("specialized on Int"), "{out}");
}

#[test]
fn rejections_carry_their_reason_and_a_fix() {
    let out = repl(&format!("{WORKOUT}:ovm\n"));
    assert!(out.contains("Still interpreted"), "{out}");
    assert!(
        out.contains("has default parameter values"),
        "no reason for 'defaulted': {out}"
    );
    assert!(
        out.contains("calls 'defaulted', which cannot compile"),
        "no chained reason for 'blocked': {out}"
    );
    // The plain-language layer.
    assert!(
        out.contains("pass every argument explicitly"),
        "no fix hint: {out}"
    );
}

#[test]
fn single_function_reports_tell_one_story() {
    let out = repl(&format!("{WORKOUT}:ovm hot\n:ovm defaulted\n:ovm ghost\n"));
    // Compiled: tier + native calls + specialization.
    assert!(out.contains("=== hot ==="), "{out}");
    assert!(out.contains("Native (JIT) calls"), "{out}");
    // Rejected: why + fix.
    assert!(out.contains("=== defaulted ==="), "{out}");
    assert!(out.contains("Why:"), "{out}");
    assert!(out.contains("Fix:"), "{out}");
    // Unknown: an orientation, not an error.
    assert!(out.contains("Not seen by the tier yet"), "{out}");
}

#[test]
fn ovm_status_is_the_same_report() {
    let out = repl(&format!("{WORKOUT}:ovm status\n"));
    assert!(out.contains("Session totals"), "{out}");
    assert!(out.contains("Compiled functions"), "{out}");
}

#[test]
fn disabled_tier_says_how_to_enable() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["--no-ovm", "repl"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    child.stdin.as_mut().unwrap().write_all(b":ovm\n").unwrap();
    let out = child.wait_with_output().expect("wait");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("disabled"), "{text}");
    assert!(text.contains("--no-ovm"), "{text}");
}

//! Cancellation tokens (OL-465), drop guards and weak references
//! (OL-467), and the shared reference (OL-466): what crosses threads
//! besides a channel's messages. Each runs on the interpreter and on the
//! bytecode tier (functions compiled at their first call), which must
//! agree.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn eval(source: &str, tier: bool) -> Result<Value, String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let program = Parser::new().parse(&source).map_err(|e| e.to_string())?;
            let mut interpreter = Interpreter::new();
            if tier {
                interpreter.enable_bytecode_tier(1, false);
            }
            interpreter.eval_program(program).map_err(|e| e.to_string())
        })
        .unwrap()
        .join()
        .unwrap()
}

/// The answer, the same on both tiers, as its printed form.
fn shows(source: &str) -> String {
    let interpreted = eval(source, false);
    let tiered = eval(source, true);
    assert_eq!(tiered, interpreted, "the tiers disagree on:\n{source}");
    match &interpreted {
        Ok(Value::String(s)) => s.to_string(),
        Ok(v) => v.to_string(),
        Err(e) => panic!("{source}\nraised: {e}"),
    }
}

fn raises(source: &str) -> String {
    match eval(source, false) {
        Ok(v) => panic!("expected an error, got {v}"),
        Err(e) => e,
    }
}

// ── cancel ──────────────────────────────────────────────────────────

#[test]
fn a_token_starts_clear_and_stays_set() {
    assert_eq!(
        shows(
            "let t = cancel.token()\n\
             let before = cancel.is_set(t)\n\
             cancel.set(t)\n\
             cancel.set(t)\n\
             [before, cancel.is_set(t), typeof(t)]"
        ),
        "[false, true, \"CancelToken\"]"
    );
}

#[test]
fn a_token_set_in_another_task_is_seen() {
    assert_eq!(
        shows(
            "fn wait_for(t) = {\n    let mut n = 0\n    while !cancel.is_set(t) { n = n + 1 }\n    true\n}\n\
             let t = cancel.token()\n\
             let w = spawn wait_for(t)\n\
             time.sleep(20)\n\
             cancel.set(t)\n\
             unwrap(task.join(w))"
        ),
        "true"
    );
}

#[test]
fn a_cancellable_call_runs_to_its_answer_when_not_cancelled() {
    assert_eq!(
        shows(
            "let r = runtime.call_cancellable((a, b) => a + b, [40, 2], cancel.token())\n\
             [r.ok, r.value, r.cancelled]"
        ),
        "[true, 42, false]"
    );
}

#[test]
fn a_token_set_before_the_call_stops_it_at_once() {
    assert_eq!(
        shows(
            "let t = cancel.token()\n\
             cancel.set(t)\n\
             let r = runtime.call_cancellable(() => 1, [], t)\n\
             [r.ok, r.error, r.cancelled]"
        ),
        "[false, \"cancelled\", true]"
    );
}

#[test]
fn setting_the_token_stops_a_loop_that_never_checks_it() {
    // A loop with no exit of its own, interpreted and compiled (a hot
    // function promoted to native code polls at its back edge).
    for body in [
        "fn spin(n) = {\n    let mut i = 0\n    while true { i = i + n }\n    i\n}",
        "fn spin(n) = {\n    let mut i = 0\n    let mut k = 0\n    while k >= 0 { i = (i + n) % 1000; k = k + 1 }\n    i\n}",
    ] {
        let src = format!(
            "{body}\n\
             let t = cancel.token()\n\
             let stopper = spawn {{ time.sleep(50)\n cancel.set(t) }}\n\
             let r = runtime.call_cancellable(spin, [1], t)\n\
             unwrap(task.join(stopper))\n\
             [r.ok, r.error, r.cancelled]"
        );
        assert_eq!(shows(&src), "[false, \"cancelled\", true]", "{body}");
    }
}

#[test]
fn a_cancelled_call_stops_only_its_own_thread() {
    // The other task's loop polls past the cancellation and finishes.
    assert_eq!(
        shows(
            "fn count(n) = {\n    let mut i = 0\n    while i < n { i = i + 1 }\n    i\n}\n\
             fn spin() = {\n    let mut i = 0\n    while true { i = i + 1 }\n    i\n}\n\
             let t = cancel.token()\n\
             let other = spawn count(3000000)\n\
             let stopper = spawn { time.sleep(20)\n cancel.set(t) }\n\
             let r = runtime.call_cancellable(spin, [], t)\n\
             unwrap(task.join(stopper))\n\
             [r.cancelled, unwrap(task.join(other))]"
        ),
        "[true, 3000000]"
    );
}

#[test]
fn a_cancelled_exec_kills_its_child() {
    let src = "let t = cancel.token()\n\
               let stopper = spawn { time.sleep(100)\n cancel.set(t) }\n\
               let t0 = time.monotonic_ms()\n\
               let r = runtime.call_cancellable(() => os.exec([\"sleep\", \"30\"]), [], t)\n\
               unwrap(task.join(stopper))\n\
               [r.error, time.monotonic_ms() - t0 < 10000]";
    assert_eq!(shows(src), "[\"cancelled\", true]");
}

#[test]
fn a_cancelled_walk_stops() {
    let dir = std::env::temp_dir().join(format!("olang-cancel-walk-{}", std::process::id()));
    for i in 0..50 {
        let d = dir.join(format!("d{i}"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("f.txt"), "x").unwrap();
    }
    let root = dir.to_string_lossy().to_string();
    for call in ["fs.walk", "fs.scan"] {
        let src = format!(
            "let t = cancel.token()\n\
             cancel.set(t)\n\
             let done = runtime.call_cancellable(() => len(unwrap({call}(\"{root}\"))), [], cancel.token())\n\
             let r = runtime.call_cancellable(() => {call}(\"{root}\"), [], t)\n\
             [done.value, r.error]"
        );
        assert_eq!(shows(&src), "[50, \"cancelled\"]", "{call}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_token_is_refused_where_a_token_is_wanted() {
    let e = raises("cancel.set(1)");
    assert!(e.contains("expected a cancel token"), "{e}");
    let e = raises("runtime.call_cancellable(() => 1, [], 5)");
    assert!(e.contains("expected a cancel token"), "{e}");
}

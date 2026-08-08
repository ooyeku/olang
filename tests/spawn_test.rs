//! `spawn` runs its expression on a real background thread. 0.29
//! consolidation item C2: previously spawn evaluated eagerly and wrapped a
//! resolved promise — these tests prove genuine concurrency and pin the
//! semantics (capture-by-value, memoized await, failure as rejection).

use olang::{Interpreter, Parser, Value};
use std::time::Instant;

fn eval(src: &str) -> Result<Value, String> {
    let program = Parser::new().parse(src).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    interp.eval_program(program).map_err(|e| e.to_string())
}

#[test]
fn spawned_tasks_run_in_parallel() {
    // Three 80ms tasks awaited together: parallel execution finishes in
    // roughly one task's time, eager-in-disguise would take three.
    let src = r#"
fn slow(n) = {
    time.sleep(80)
    n
}
let a = spawn slow(1)
let b = spawn slow(2)
let c = spawn slow(3)
await a + await b + await c
"#;
    let started = Instant::now();
    assert_eq!(eval(src).unwrap(), Value::Integer(6));
    let elapsed = started.elapsed().as_millis();
    assert!(
        elapsed < 200,
        "3 x 80ms tasks took {elapsed}ms — not parallel"
    );
}

#[test]
fn spawn_returns_before_the_task_finishes() {
    let src = r#"
let t0 = time.monotonic_ms()
let p = spawn { time.sleep(120) }
let spawn_cost = time.monotonic_ms() - t0
await p
spawn_cost < 60
"#;
    assert_eq!(eval(src).unwrap(), Value::Boolean(true));
}

#[test]
fn awaiting_a_cloned_promise_is_memoized() {
    let src = r#"
fn work() = {
    time.sleep(20)
    42
}
let p = spawn work()
let first = await p
let second = await p
first + second
"#;
    assert_eq!(eval(src).unwrap(), Value::Integer(84));
}

#[test]
fn spawn_captures_bindings_by_value() {
    let src = r#"
let base = 100
let p = spawn (base + 1)
let base = 999
await p
"#;
    assert_eq!(eval(src).unwrap(), Value::Integer(101));
}

#[test]
fn a_failing_task_rejects_instead_of_crashing() {
    let src = r#"
fn boom() = unwrap(Err("exploded"))
let p = spawn boom()
await p
"#;
    let err = eval(src).unwrap_err();
    assert!(err.contains("spawned task failed"), "got: {err}");
}

#[test]
fn spawned_tasks_settle_through_promise_all() {
    let src = r#"
fn slow(n) = {
    time.sleep(50)
    n * 2
}
let jobs = [spawn slow(1), spawn slow(2), spawn slow(3)]
let results = await Promise.all(jobs)
sum(results)
"#;
    let started = Instant::now();
    assert_eq!(eval(src).unwrap(), Value::Integer(12));
    assert!(started.elapsed().as_millis() < 200);
}

//! `spawn` runs its expression on a real background thread.
//!
//! Originally the 0.29 consolidation item C2: spawn used to evaluate
//! eagerly and wrap a resolved promise, and these tests prove genuine
//! concurrency. 0.63 removed promises — `spawn` now returns a task handle
//! and `task.join` collects it — but every property below is about the
//! *thread*, not the vocabulary, so they still earn their place.
//! [`task_test.rs`](task_test.rs) covers the `task` module's own surface.

use olang::{Interpreter, Parser, Value};
use std::time::Instant;

fn eval(src: &str) -> Result<Value, String> {
    let program = Parser::new().parse(src).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    interp.eval_program(program).map_err(|e| e.to_string())
}

#[test]
fn spawned_tasks_run_in_parallel() {
    // Three 80ms tasks joined together: parallel execution finishes in
    // roughly one task's time, eager-in-disguise would take three.
    let src = r#"
fn slow(n) = {
    time.sleep(80)
    n
}
let a = spawn slow(1)
let b = spawn slow(2)
let c = spawn slow(3)
task.join(a) + task.join(b) + task.join(c)
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
task.join(p)
spawn_cost < 60
"#;
    assert_eq!(eval(src).unwrap(), Value::Boolean(true));
}

#[test]
fn joining_a_cloned_handle_is_memoized() {
    let src = r#"
fn work() = {
    time.sleep(20)
    42
}
let p = spawn work()
let first = task.join(p)
let second = task.join(p)
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
task.join(p)
"#;
    assert_eq!(eval(src).unwrap(), Value::Integer(101));
}

#[test]
fn a_failing_task_settles_as_an_err_value() {
    // Joining a failed task yields Err(e) — a value, not a crash — so
    // match / try-catch / unwrap_or / `?` all recover from worker failure.
    let src = r#"
fn boom() = unwrap(Err("exploded"))
let p = spawn boom()
match task.join(p) {
    Err(e) => "handled: " + show(e),
    v => "unexpected"
}
"#;
    match eval(src).unwrap() {
        Value::String(s) => {
            assert!(s.contains("handled: "), "got: {s}");
            assert!(s.contains("exploded"), "the cause should survive: {s}");
        }
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn a_failed_task_does_not_stop_its_siblings() {
    let src = r#"
fn work(n) = if n == 1 => unwrap(Err("died")) else => n * 10
let jobs = [spawn work(0), spawn work(1), spawn work(2)]
jobs |> map((j) => match task.join(j) { Err(e) => 0 - 1, v => v })
"#;
    match eval(src).unwrap() {
        Value::List(items) => {
            assert_eq!(
                items.as_ref().to_vec(),
                vec![Value::Integer(0), Value::Integer(-1), Value::Integer(20)]
            );
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn a_list_of_tasks_is_joined_with_map() {
    // What `Promise.all` used to do. It needs no API of its own: the tasks
    // are already running, so joining them in order costs nothing.
    let src = r#"
fn slow(n) = {
    time.sleep(50)
    n * 2
}
let jobs = [spawn slow(1), spawn slow(2), spawn slow(3)]
sum(jobs |> map(task.join))
"#;
    let started = Instant::now();
    assert_eq!(eval(src).unwrap(), Value::Integer(12));
    assert!(started.elapsed().as_millis() < 200);
}

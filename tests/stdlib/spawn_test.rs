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
unwrap(task.join(a)) + unwrap(task.join(b)) + unwrap(task.join(c))
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
unwrap(task.join(p))
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
let first = unwrap(task.join(p))
let second = unwrap(task.join(p))
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
unwrap(task.join(p))
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
        Value::String(ref s) => {
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
jobs |> map((j) => match task.join(j) { Err(e) => 0 - 1, Ok(v) => v })
"#;
    match eval(src).unwrap() {
        Value::List(ref items) => {
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
sum(jobs |> map((t) => unwrap(task.join(t))))
"#;
    let started = Instant::now();
    assert_eq!(eval(src).unwrap(), Value::Integer(12));
    assert!(started.elapsed().as_millis() < 200);
}

/// The median time of `spawn` and of a task's start, in ms, for a
/// program declaring `functions` functions besides its own.
fn spawn_times(functions: usize) -> (f64, f64) {
    let mut src = String::new();
    for i in 0..functions {
        src.push_str(&format!("fn f{i}(x) = x + {i}\n"));
    }
    src.push_str(
        r#"
fn started() = time.monotonic()
let mut waits = []
let mut costs = []
for i in range(0, 201) {
    let t0 = time.monotonic()
    let t = spawn started()
    let t1 = time.monotonic()
    let s = unwrap(task.join(t))
    waits = waits + [s - t0]
    costs = costs + [t1 - t0]
}
println(show(sort(costs)[100]) + " " + show(sort(waits)[100]))
"#,
    );
    let dir = std::env::temp_dir().join(format!(
        "olang-spawn-cost-{}-{functions}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.ol");
    std::fs::write(&file, src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .arg(&file)
        .output()
        .expect("run olang");
    let _ = std::fs::remove_dir_all(&dir);
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text.split_whitespace().map(|p| p.parse::<f64>().unwrap());
    (parts.next().unwrap(), parts.next().unwrap())
}

#[test]
fn a_spawn_costs_about_the_same_in_a_large_program() {
    // OL-330: a worker's tier (and the bridge it builds for its first
    // builtin call) noted every function of the program again, about
    // 0.3 µs each: with 4,000 functions a spawn took 0.69 ms and its
    // task started 1.5 ms after it was asked for, against 0.05 and 0.12
    // in a small program. The tables are taken whole now.
    let (small_cost, small_wait) = spawn_times(0);
    let (large_cost, large_wait) = spawn_times(4000);
    assert!(
        large_cost < small_cost * 5.0 + 0.1,
        "spawn: {large_cost} ms with 4,000 functions, {small_cost} ms without"
    );
    assert!(
        large_wait < small_wait * 5.0 + 0.3,
        "a task's start: {large_wait} ms with 4,000 functions, {small_wait} ms without"
    );
}

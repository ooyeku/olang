//! `spawn` + `task` — the concurrency model after 0.63 removed
//! `async`/`await` and the `Promise` API (roadmap lane S5).
//!
//! What this pins: `spawn` returns a task handle; `task.join` blocks and
//! yields the value (or `Err(e)` on failure, never a crash); joining is
//! composable with `map`; `task.join_timeout` bounds the wait and not the
//! work; and the removed surface produces migration errors rather than
//! generic parse failures.

use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn run(source: &str) -> Result<String, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    let mut interpreter = Interpreter::new();
    interpreter
        .eval_program(program)
        .map(|v| match v {
            olang::ast::Value::String(s) => s.to_string(),
            other => other.to_string(),
        })
        .map_err(|e| e.to_string())
}

fn err(source: &str) -> String {
    match run(source) {
        Ok(v) => panic!("expected an error, got {v}"),
        Err(e) => e,
    }
}

// ── spawn and join ────────────────────────────────────────────────────

#[test]
fn spawn_returns_a_task_that_join_collects() {
    // `spawn` takes a *call* — `spawn f(x)`, not a bare expression, so
    // `spawn 1 + 1` would parse as `(spawn 1) + 1`.
    assert_eq!(
        run("fn answer() = 42
typeof(spawn answer())
")
        .unwrap(),
        "Task"
    );
    assert_eq!(
        run("fn answer() = 20 + 22
task.join(spawn answer())
")
        .unwrap(),
        "42"
    );
}

#[test]
fn tasks_run_concurrently() {
    // Three 60ms sleeps joined together must take about 60ms, not 180 —
    // the whole point of `spawn` being a thread rather than a deferred
    // computation.
    let out = run("fn slow(n) = { time.sleep(60); n }\n\
                   let t0 = time.monotonic_ms()\n\
                   let jobs = [spawn slow(1), spawn slow(2), spawn slow(3)]\n\
                   let got = jobs |> map(task.join)\n\
                   let ms = time.monotonic_ms() - t0\n\
                   to_string(sum(got)) + \" \" + to_string(ms < 150)\n")
    .unwrap();
    assert_eq!(
        out, "6 true",
        "three concurrent 60ms tasks should not serialize"
    );
}

#[test]
fn joining_many_is_just_map() {
    // No `join_all`: every task is already running, so `map` over the
    // handles is the fan-in. This is the documented idiom.
    assert_eq!(
        run("fn id(n) = n\nlet jobs = [spawn id(1), spawn id(2), spawn id(3)]\nto_string(jobs |> map(task.join))\n").unwrap(),
        "[1, 2, 3]"
    );
}

#[test]
fn a_task_sees_a_snapshot_of_its_spawning_scope() {
    // Capture by value, exactly like a closure — which is what makes
    // threads safe without locks.
    let out = run("let factor = 10\n\
                   fn scaled() = 5 * factor\n\
                   let t = spawn scaled()\n\
                   let factor = 999\n\
                   task.join(t)\n")
    .unwrap();
    assert_eq!(out, "50");
}

#[test]
fn joining_twice_returns_the_memoized_result() {
    let out = run("fn once(n) = { time.sleep(10); n }\n\
                   let t = spawn once(7)\n\
                   to_string(task.join(t)) + \" \" + to_string(task.join(t))\n")
    .unwrap();
    assert_eq!(out, "7 7");
}

// ── failure is a value ────────────────────────────────────────────────

#[test]
fn a_failed_task_yields_err_rather_than_aborting() {
    let out = run("fn boom(n) = n + \"not a number\"\n\
                   match task.join(spawn boom(1)) { Err(e) => \"handled\", v => \"unexpected\" }\n")
    .unwrap();
    assert_eq!(out, "handled");
}

#[test]
fn one_failed_task_does_not_kill_the_batch() {
    let out = run(
        "fn work(n) = if n == 1 => unwrap(Err(\"died\")) else => n * 10\n\
                   let jobs = [spawn work(0), spawn work(1), spawn work(2)]\n\
                   to_string(jobs |> map((j) => match task.join(j) { Err(e) => -1, v => v }))\n",
    )
    .unwrap();
    assert_eq!(out, "[0, -1, 20]");
}

// ── timeouts bound the wait, not the work ─────────────────────────────

#[test]
fn join_timeout_reports_a_timeout_without_stopping_the_task() {
    // The load-bearing property, and the one an `async` model would have
    // let us lie about: the task keeps running and stays collectible.
    let out = run("fn slow(n) = { time.sleep(120); n }\n\
                   let t = spawn slow(9)\n\
                   let first = task.join_timeout(t, 5)\n\
                   let later = task.join_timeout(t, 5000)\n\
                   show(first) + \" then \" + show(later)\n")
    .unwrap();
    assert_eq!(out, "Err(\"timed out\") then Ok(9)");
}

#[test]
fn join_timeout_wraps_success_so_failure_is_distinguishable() {
    // `Ok(Err(e))` is "the task failed"; `Err("timed out")` is "we gave
    // up waiting". Collapsing them would lose the difference.
    let out =
        run("fn boom(n) = n + \"nope\"\nshow(task.join_timeout(spawn boom(1), 5000))\n").unwrap();
    assert!(out.starts_with("Ok(Err("), "{out}");
}

#[test]
fn join_timeout_rejects_a_negative_budget() {
    let e = err("fn one() = 1\ntask.join_timeout(spawn one(), -5)\n");
    assert!(e.contains("non-negative"), "{e}");
}

// ── errors on non-tasks ───────────────────────────────────────────────

#[test]
fn task_operations_reject_values_that_are_not_tasks() {
    for source in ["task.join(5)", "task.join_timeout([1], 10)"] {
        let e = err(source);
        assert!(e.contains("expected a task"), "{source}: {e}");
    }
    // A cell is a native handle too, and must not be mistaken for one.
    let e = err("task.join(cell(0))");
    assert!(e.contains("Cell handle"), "{e}");
}

// ── the removed surface ───────────────────────────────────────────────

#[test]
fn the_async_vocabulary_is_gone_and_its_words_are_free() {
    // 0.63 removed async/await/Promise. The grammar carried migration
    // stubs for a release; with no olang code outside this repository
    // there was nobody to migrate, so the stubs came out and the words
    // are ordinary identifiers.
    assert_eq!(
        run("let async = 1\nlet await = 2\nlet Promise = 3\nto_string(async + await + Promise)\n")
            .unwrap(),
        "6"
    );
    // `await_all` reads fine as a helper name now.
    assert_eq!(
        run("fn one() = 7\n\
             fn await_all(ts) = ts |> map(task.join)\n\
             to_string(sum(await_all([spawn one(), spawn one()])))\n")
        .unwrap(),
        "14"
    );
}

// ── the tiers agree ───────────────────────────────────────────────────

#[test]
fn task_handles_survive_promotion_to_the_bytecode_tier() {
    // A hot function that spawns and joins must behave identically once
    // compiled: the handle is a native value shared verbatim across the
    // tier boundary.
    let out = run("fn double(n) = n * 2\n\
                   fn both(n) = task.join(spawn double(n)) + task.join(spawn double(n))\n\
                   let mut last = 0\n\
                   for i in 0..200 { last = both(i) }\n\
                   to_string(last)\n")
    .unwrap();
    assert_eq!(out, "796");
}

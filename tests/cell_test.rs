//! `cell` — the thread-confined mutable location (roadmap lane S4).
//!
//! The rules this file pins: a cell holds and replaces a value; it is
//! confined to the thread that created it; `update` refuses re-entrant
//! access instead of silently clobbering; equality is identity; and every
//! execution tier agrees, since a cell is a `Value::Native` handle that
//! crosses tier boundaries verbatim.

use olang::interpreter::Interpreter;
use olang::parser::Parser;

/// Run a program and return whatever the last expression evaluated to.
/// A String result is returned bare, so an assertion reads as the text the
/// program produced rather than as its quoted display form.
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

// ── the basics ────────────────────────────────────────────────────────

#[test]
fn a_cell_holds_and_replaces_a_value() {
    assert_eq!(run("let c = cell(7)\ncell.get(c)\n").unwrap(), "7");
    assert_eq!(
        run("let c = cell(7)\ncell.set(c, 9)\ncell.get(c)\n").unwrap(),
        "9"
    );
}

#[test]
fn the_module_is_callable_and_new_is_the_same_function() {
    // `cell(0)` is sugar for `cell.new(0)`; both must produce a cell that
    // behaves identically, so the short spelling costs nothing.
    assert_eq!(run("typeof(cell(0))").unwrap(), "Cell");
    assert_eq!(run("typeof(cell.new(0))").unwrap(), "Cell");
    assert_eq!(
        run("let a = cell.new(1)\ncell.set(a, 2)\ncell.get(a)\n").unwrap(),
        "2"
    );
}

#[test]
fn update_applies_stores_and_returns_the_new_value() {
    assert_eq!(
        run("let c = cell(10)\ncell.update(c, (n) => n * 3)\n").unwrap(),
        "30"
    );
    assert_eq!(
        run("let c = cell(10)\ncell.update(c, (n) => n * 3)\ncell.get(c)\n").unwrap(),
        "30"
    );
}

#[test]
fn a_cell_holds_any_value_not_just_numbers() {
    assert_eq!(
        run("let c = cell([1, 2])\ncell.set(c, cell.get(c) + [3])\nto_string(cell.get(c))\n")
            .unwrap(),
        "[1, 2, 3]"
    );
    assert_eq!(
        run("let c = cell(#{ \"n\": 1 })\nmap_get(cell.get(c), \"n\")\n").unwrap(),
        "1"
    );
}

#[test]
fn a_cell_is_the_accumulator_a_closure_cannot_be() {
    // The motivating case: state updated from inside a closure, which
    // capture-by-value makes impossible with an ordinary binding.
    let out = run("let total = cell(0)\n\
                   let add = (n) => cell.update(total, (t) => t + n)\n\
                   for n in [1, 2, 3, 4] { add(n) }\n\
                   cell.get(total)\n")
    .unwrap();
    assert_eq!(out, "10");
}

// ── identity ──────────────────────────────────────────────────────────

#[test]
fn cells_compare_by_identity_not_contents() {
    // A cell is a location. Two cells holding equal values are still two
    // places to write, so they are not equal.
    assert_eq!(run("cell(1) == cell(1)").unwrap(), "false");
    assert_eq!(run("let c = cell(1)\nc == c\n").unwrap(), "true");
    // Binding it elsewhere aliases the same location.
    assert_eq!(
        run("let a = cell(1)\nlet b = a\ncell.set(b, 5)\ncell.get(a)\n").unwrap(),
        "5"
    );
}

#[test]
fn a_cell_shows_its_contents() {
    assert_eq!(run("show(cell(41))").unwrap(), "cell(41)");
}

// ── confinement ───────────────────────────────────────────────────────

#[test]
fn reading_a_cell_from_a_spawned_task_is_an_error() {
    // `spawn` snapshots the whole environment, so the cell rides along —
    // and the access on the other side is what fails, naming both threads.
    let out = run("let c = cell(0)\nshow(task.join(spawn { cell.get(c) }))\n").unwrap();
    assert!(out.contains("cell escaped its thread"), "{out}");
    assert!(out.contains("olang-spawn"), "{out}");
}

#[test]
fn writing_a_cell_from_a_spawned_task_is_an_error() {
    let out = run("let c = cell(0)\nshow(task.join(spawn { cell.set(c, 1) }))\n").unwrap();
    assert!(out.contains("cell escaped its thread"), "{out}");
}

#[test]
fn a_cell_created_inside_a_task_belongs_to_that_task() {
    // Confinement is about the creating thread, not about `spawn`: a cell
    // made and used entirely inside a task is perfectly legal.
    let out = run("let t = spawn {\n\
                       let c = cell(0)\n\
                       cell.set(c, 41)\n\
                       cell.get(c) + 1\n\
                   }\n\
                   task.join(t)\n")
    .unwrap();
    assert_eq!(out, "42");
}

#[test]
fn a_cell_merely_in_scope_during_a_task_stays_legal() {
    // The reason confinement is enforced at access rather than at the
    // crossing: a task that never touches the cell must not be penalised
    // for one existing in the enclosing scope.
    let out = run("let c = cell(0)\n\
                   let t = spawn { 1 + 1 }\n\
                   let n = task.join(t)\n\
                   cell.set(c, 5)\n\
                   to_string(n) + \" \" + to_string(cell.get(c))\n")
    .unwrap();
    assert_eq!(out, "2 5");
}

#[test]
fn par_map_cannot_reach_a_cell_from_the_calling_thread() {
    let e = err("let c = cell(0)\npar_map([1, 2, 3, 4], (n) => cell.get(c) + n)\n");
    assert!(e.contains("cell escaped its thread"), "{e}");
}

#[test]
fn sending_a_cell_through_a_channel_is_refused_at_the_send() {
    // The one crossing where the value is in hand, so the error lands on
    // the send rather than on the receiver's first access.
    let e = err("let ch = chan.new()\nlet c = cell(1)\nchan.send(ch, c)\n");
    assert!(e.contains("cannot be sent through a channel"), "{e}");
}

#[test]
fn a_cell_nested_inside_a_sent_value_is_also_refused() {
    // A shallow check would wave these through, and the mistake would then
    // surface as a confinement error in unrelated code much later.
    for source in [
        "let ch = chan.new()\nchan.send(ch, [1, cell(2), 3])\n",
        "let ch = chan.new()\nchan.send(ch, #{ \"state\": cell(0) })\n",
        "let ch = chan.new()\nchan.send(ch, Ok(cell(0)))\n",
    ] {
        let e = err(source);
        assert!(
            e.contains("cannot be sent through a channel"),
            "{source}: {e}"
        );
    }
}

#[test]
fn sending_a_cells_contents_is_the_supported_route() {
    let out = run("let ch = chan.new()\n\
                   let c = cell(11)\n\
                   chan.send(ch, cell.get(c))\n\
                   show(chan.recv(ch))\n")
    .unwrap();
    assert_eq!(out, "Ok(11)");
}

// ── re-entrancy ───────────────────────────────────────────────────────

#[test]
fn touching_a_cell_from_inside_its_own_update_is_an_error() {
    // Silently clobbering the inner write is exactly the bug class cells
    // exist to make visible, so it is loud.
    let e = err("let c = cell(1)\ncell.update(c, (n) => { cell.set(c, 0); n + 1 })\n");
    assert!(e.contains("cell is being updated"), "{e}");
    let e = err("let c = cell(1)\ncell.update(c, (n) => n + cell.get(c))\n");
    assert!(e.contains("cell is being updated"), "{e}");
    // Exactly one "Runtime error:" prefix — the callback's error is
    // propagated, not stringified and re-wrapped.
    assert_eq!(e.matches("Runtime error").count(), 1, "{e}");
}

#[test]
fn a_different_cell_is_reachable_during_an_update() {
    // The flag is per cell, not global: only the cell being updated is
    // off limits.
    let out = run("let a = cell(1)\n\
                   let b = cell(10)\n\
                   cell.update(a, (n) => n + cell.get(b))\n")
    .unwrap();
    assert_eq!(out, "11");
}

#[test]
fn a_failed_update_leaves_the_cell_usable() {
    // The in-update flag is cleared whether the function returns or
    // raises, so one bad update does not brick the cell.
    // The in-update flag is cleared whether the function returns or fails,
    // so one bad update must not leave the cell permanently unreadable.
    // A runtime error is not catchable in-language (`catch` unwraps
    // Results), so the two halves run as two programs against one
    // interpreter — the same way the REPL continues after an error.
    let mut interpreter = Interpreter::new();
    let mut eval = |source: &str| -> Result<String, String> {
        let program = Parser::new().parse(source).expect("parses");
        interpreter
            .eval_program(program)
            .map(|v| v.to_string())
            .map_err(|e| e.to_string())
    };
    eval("let c = cell(1)\n").expect("setup");
    let failed = eval("cell.update(c, (n) => n + \"nope\")\n");
    assert!(failed.is_err(), "the update should have failed");
    // The cell is still readable, still holds its original value, and
    // still accepts a good update.
    assert_eq!(eval("cell.get(c)\n").unwrap(), "1");
    assert_eq!(eval("cell.update(c, (n) => n + 1)\n").unwrap(), "2");
}

// ── errors on non-cells ───────────────────────────────────────────────

#[test]
fn cell_operations_reject_values_that_are_not_cells() {
    for source in [
        "cell.get(5)",
        "cell.set([1], 2)",
        "cell.update(\"x\", (n) => n)",
    ] {
        let e = err(source);
        assert!(e.contains("expected a cell"), "{source}: {e}");
    }
    // A channel is a native handle too, and must not be mistaken for one.
    let e = err("cell.get(chan.new())");
    assert!(e.contains("Channel handle"), "{e}");
}

// ── the tiers agree ───────────────────────────────────────────────────

#[test]
fn cell_state_survives_promotion_to_the_bytecode_tier() {
    // A function hot enough to be compiled must observe the same cell as
    // the interpreter did on its earlier calls: the handle is one `Arc`
    // shared verbatim across the tier boundary, not a converted copy.
    let out = run("let seen = cell(0)\n\
                   fn bump(n) = {\n\
                       cell.set(seen, cell.get(seen) + n)\n\
                       cell.get(seen)\n\
                   }\n\
                   let mut last = 0\n\
                   for i in 0..500 { last = bump(1) }\n\
                   to_string(last) + \" \" + to_string(cell.get(seen))\n")
    .unwrap();
    assert_eq!(out, "500 500");
}

#[test]
fn a_cell_passed_through_a_hot_function_keeps_its_identity() {
    let out = run("fn poke(c, v) = { cell.set(c, v); cell.get(c) }\n\
                   let c = cell(0)\n\
                   let mut last = 0\n\
                   for i in 0..500 { last = poke(c, i) }\n\
                   to_string(cell.get(c))\n")
    .unwrap();
    assert_eq!(out, "499");
}

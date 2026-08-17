//! Interpreter-level semantics tests for `par for` — the parallel loop
//! statement. Parallel effects target worker snapshots (like spawn and
//! par_map); errors report the first the sequential loop would hit;
//! break/return cannot cross the parallel boundary.

use olang::interpreter::Interpreter;
use olang::parser::Parser as OlangParser;

fn run_program(source: &str) -> String {
    let parser = OlangParser::new();
    let program = parser.parse(source).expect("parse");
    let mut interp = Interpreter::new();
    match interp.eval_program(program) {
        Ok(v) => format!("{}", v),
        Err(e) => panic!("unexpected error: {e}"),
    }
}

fn run_program_expect_error(source: &str) -> String {
    let parser = OlangParser::new();
    let program = parser.parse(source).expect("parse");
    let mut interp = Interpreter::new();
    match interp.eval_program(program) {
        Ok(v) => panic!("expected error, got {v}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn par_for_runs_with_snapshot_semantics() {
    // Each worker runs the body against its own snapshot, so a write to
    // an enclosing binding could never reach the caller. That used to
    // simply produce 0 here; it is refused before the program runs now,
    // because the old silence was also length-dependent — a one-item list
    // took the sequential path and the write landed.
    let err = run_program_expect_error(
        r#"
let mut hits = 0
par for i in 0..100 { hits = hits + 1 }
hits
"#,
    );
    assert!(err.contains("cannot assign to 'hits'"), "{err}");
    assert!(err.contains("par for"), "{err}");

    // What the loop can do: read the enclosing scope, write its own
    // locals, and hand results back over a channel.
    let result = run_program(
        r#"
let step = 2
let c = chan.new()
par for i in 0..100 {
    let mut n = 0
    n = n + step
    chan.send(c, n)
}
let mut hits = 0
for i in 0..100 { hits = hits + unwrap(chan.recv(c)) }
hits
"#,
    );
    assert_eq!(result, "200");
}

#[test]
fn par_for_tuple_destructuring_ranges_and_strings() {
    let result = run_program(
        r#"
par for (a, b) in [(1, 2), (30, 4)] { let _ = a + b }
par for i in 0..3 { let _ = i }
par for c in "ab" { let _ = c }
"done"
"#,
    );
    assert_eq!(result, "\"done\"");
}

#[test]
fn par_for_reports_the_first_sequential_error() {
    let err = run_program_expect_error(
        r#"
par for i in [1, 0, 2, 0] { let _ = 10 / i }
"#,
    );
    assert!(err.to_lowercase().contains("zero"), "got: {err}");
}

#[test]
fn par_for_rejects_break_and_bad_iterables() {
    let err = run_program_expect_error("par for i in 0..4 { break }");
    assert!(err.contains("par for boundary"), "got: {err}");
    let err2 = run_program_expect_error("par for i in 42 { let _ = i }");
    assert!(err2.contains("cannot iterate"), "got: {err2}");
}

#[test]
fn par_for_empty_and_single_element() {
    let result = run_program(
        r#"
par for i in [] { let _ = i }
par for i in [7] { let _ = i }
"ok"
"#,
    );
    assert_eq!(result, "\"ok\"");
}

#[test]
fn par_is_not_a_reserved_word() {
    // `par` only means something directly before `for`.
    let result = run_program("let par = 5\npar + 1");
    assert_eq!(result, "6");
}

// ── Review findings: forms that had no way to be written ──────────────

/// Both tiers, same source, same answer. The three forms below are new,
/// and a new form is exactly where the tiers drift apart.
fn both_tiers(source: &str) -> String {
    let run = |bytecode: bool| {
        let program = olang::parser::Parser::new().parse(source).expect("parse");
        let mut interpreter = olang::interpreter::Interpreter::new();
        if bytecode {
            interpreter.enable_bytecode_tier(2, false);
        }
        interpreter
            .eval_program(program)
            .map(|v| v.to_string())
            .unwrap_or_else(|e| format!("error: {e}"))
    };
    let interpreted = run(false);
    let compiled = run(true);
    assert_eq!(
        interpreted, compiled,
        "tiers disagree on:\n{source}\n  interpreter: {interpreted}\n  bytecode:    {compiled}"
    );
    interpreted
}

#[test]
fn unit_has_a_literal() {
    // `()` is the value the language hands back from an empty branch and
    // from an `X | ()` field, and it had no spelling — every use of it
    // was a parse error.
    assert_eq!(both_tiers("let u = ()\nu"), "()");
    assert_eq!(both_tiers("fn id(x) = x\nid(())"), "()");
}

#[test]
fn unit_is_matchable() {
    // Without a pattern there was no way to *test* for Unit either, so
    // an optional field could be produced and never inspected.
    assert_eq!(
        both_tiers(
            r#"
fn kind(x) = match x { () => "empty", v => "value" }
kind(()) + "/" + kind(1)
"#
        ),
        "\"empty/value\""
    );
}

#[test]
fn unit_wins_over_a_nullary_lambda_after_a_comparison() {
    // `if t == () => 0 else => 1` reads as "t is Unit, then this branch".
    // Ordered choice would otherwise let the nullary-lambda rule swallow
    // the `=>` that belongs to the `if`.
    assert_eq!(both_tiers("let u = ()\nif u == () => 1 else => 2"), "1");
    // And the nullary lambda itself still parses where it is meant.
    assert_eq!(both_tiers("let f = () => 42\nf()"), "42");
}

#[test]
fn a_loop_can_discard_its_binding() {
    // `_` is not an identifier — identifiers start with a letter — so a
    // loop that ignores its item needed an invented name.
    assert_eq!(
        both_tiers("let mut n = 0\nfor _ in 0..3 { n = n + 1 }\nn"),
        "3"
    );
}

#[test]
fn a_tuple_iterates() {
    assert_eq!(
        both_tiers("let mut s = 0\nfor x in (1, 2, 3) { s = s + x }\ns"),
        "6"
    );
}

#[test]
fn assertions_work_inside_nested_blocks() {
    // Assertions were parsed only as a direct child of a test block, so
    // one inside an `if`, a `for`, or a `while` fell through to an
    // ordinary call and failed with "Undefined variable: assert_eq" —
    // surprising, since a loop over cases is where an assertion belongs.
    let source = r#"
test "nested" {
    for i in 0..3 { assert_eq(i * 2, i + i) }
    if true => { assert_true(1 < 2) }
    let mut n = 0
    while n < 2 {
        assert_ne(n, 99)
        n = n + 1
    }
}
"#;
    let program = olang::parser::Parser::new().parse(source).expect("parse");
    let mut interpreter = olang::interpreter::Interpreter::new();
    interpreter.eval_program(program).expect("assertions run");
}

#[test]
fn a_failing_assertion_in_a_loop_still_fails() {
    // The fix must not make assertions unreachable in the other
    // direction — a nested assertion that fails has to be reported.
    let source = r#"
test "nested failure" {
    for i in 0..3 { assert_eq(i, 99) }
}
"#;
    let program = olang::parser::Parser::new().parse(source).expect("parse");
    let mut interpreter = olang::interpreter::Interpreter::new();
    let result = interpreter.eval_program(program);
    assert!(result.is_err(), "a failing nested assertion must be caught");
}

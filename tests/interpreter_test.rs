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
    // Effects into worker snapshots are not visible to the caller —
    // exactly the spawn/par_map contract.
    let result = run_program(
        r#"
let mut hits = 0
par for i in 0..100 { hits = hits + 1 }
hits
"#,
    );
    assert_eq!(result, "0");
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

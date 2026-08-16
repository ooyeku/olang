//! Runtime errors are attributed to source positions: the innermost
//! located statement being evaluated when the error surfaced, plus the
//! interpreter call stack. The location is a side channel — error
//! Display strings are unchanged — taken by top-level reporters.

use olang::{Interpreter, Parser};

fn run_expect_error(source: &str) -> (String, Option<olang::ast::ErrorLocation>) {
    let parser = Parser::new();
    let program = parser.parse(source).expect("parses");
    let mut interp = Interpreter::new();
    let err = interp.eval_program(program).expect_err("should error");
    (err.to_string(), interp.take_error_location())
}

#[test]
fn top_level_error_carries_its_line() {
    let (msg, loc) = run_expect_error("let a = 1\nlet b = 2\nprintln(missing_here)\n");
    assert!(msg.contains("missing_here"));
    let loc = loc.expect("location captured");
    assert_eq!(loc.line, 3);
    assert!(loc.call_stack.is_empty());
}

#[test]
fn error_inside_calls_carries_inner_line_and_call_stack() {
    let src = "fn deep(x) = {\n    let y = x * 2\n    println(gone)\n    y\n}\nfn mid(a) = deep(a + 1)\nmid(5)\n";
    let (msg, loc) = run_expect_error(src);
    assert!(msg.contains("gone"));
    let loc = loc.expect("location captured");
    assert_eq!(loc.line, 3, "innermost statement line");
    assert_eq!(loc.call_stack, vec!["mid".to_string(), "deep".to_string()]);
}

#[test]
fn recovered_error_does_not_leak_location_to_later_success() {
    // A handled Err (a value, not a hard error — hard errors are not
    // recoverable by design) then a clean statement: no stale location.
    let parser = Parser::new();
    let program = parser
        .parse("fn may() = Err(\"nope\")\nlet r = match may() { Err(e) => -1, v => v }\nprintln(to_string(r))\n")
        .expect("parses");
    let mut interp = Interpreter::new();
    interp.eval_program(program).expect("runs clean");
    assert!(interp.take_error_location().is_none());
}

#[test]
fn control_flow_signals_never_capture_locations() {
    let parser = Parser::new();
    let program = parser
        .parse("fn f() = {\n    for i in [1, 2, 3] {\n        if i == 2 => { break }\n    }\n    42\n}\nprintln(to_string(f()))\n")
        .expect("parses");
    let mut interp = Interpreter::new();
    interp.eval_program(program).expect("runs clean");
    assert!(interp.take_error_location().is_none());
}

#[test]
fn undefined_variable_offers_did_you_mean() {
    let (msg, loc) =
        run_expect_error("fn compute_total(xs) = sum(xs)\nprintln(to_string(compute_totl([1])))\n");
    assert!(msg.contains("compute_totl"));
    let hint = loc.expect("location").hint.expect("hint");
    assert!(hint.contains("compute_total"), "hint was: {hint}");
}

#[test]
fn no_hint_when_nothing_is_close() {
    let (_, loc) = run_expect_error("println(zzqxwv_nothing_like_this)\n");
    assert!(loc.expect("location").hint.is_none());
}

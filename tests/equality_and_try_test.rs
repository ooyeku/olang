//! Two fixes surfaced by writing the language reference: structural equality
//! for compound values, and `?` actually propagating an `Err` to the caller
//! instead of aborting the program.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    let mut interp = Interpreter::new();
    interp.eval_program(program).expect("eval")
}

fn eval_res(src: &str) -> Result<Value, String> {
    let program = Parser::new().parse(src).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    interp.eval_program(program).map_err(|e| e.to_string())
}

fn s(v: Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

// ── structural equality ────────────────────────────────────────────────

#[test]
fn lists_compare_structurally() {
    assert_eq!(eval("[1, 2] == [1, 2]"), Value::Boolean(true));
    assert_eq!(eval("[1, 2] == [1, 3]"), Value::Boolean(false));
    assert_eq!(eval("[1, 2] != [1, 3]"), Value::Boolean(true));
    assert_eq!(eval("[[1], [2]] == [[1], [2]]"), Value::Boolean(true));
}

#[test]
fn tuples_compare_structurally() {
    assert_eq!(eval(r#"(1, "a") == (1, "a")"#), Value::Boolean(true));
    assert_eq!(eval(r#"(1, "a") == (1, "b")"#), Value::Boolean(false));
}

#[test]
fn maps_compare_structurally() {
    assert_eq!(
        eval(r#"#{ "k": 1, "j": 2 } == #{ "j": 2, "k": 1 }"#),
        Value::Boolean(true)
    );
    assert_eq!(eval(r#"#{ "k": 1 } != #{ "k": 2 }"#), Value::Boolean(true));
}

#[test]
fn structs_and_objects_compare_structurally() {
    assert_eq!(eval("{ x: 1 } == { x: 1 }"), Value::Boolean(true));
    assert_eq!(eval("{ x: 1 } == { x: 2 }"), Value::Boolean(false));
    let src = r#"
type P = struct { x: Int }
P { x: 1 } == P { x: 1 }
"#;
    assert_eq!(eval(src), Value::Boolean(true));
}

// ── `?` propagation ────────────────────────────────────────────────────

#[test]
fn question_mark_unwraps_ok() {
    let src = r#"
fn f() = Ok(41)
fn g() = {
    let v = f()?
    Ok(v + 1)
}
to_string(g())
"#;
    assert_eq!(s(eval(src)), "Ok(42)");
}

#[test]
fn question_mark_returns_the_err_to_the_caller() {
    let src = r#"
fn f() = Err("boom")
fn g() = {
    let v = f()?
    Ok(v + 1)
}
to_string(g())
"#;
    assert_eq!(s(eval(src)), "Err(\"boom\")");
}

#[test]
fn question_mark_chains_stop_at_the_first_err() {
    let src = r#"
fn parse_both(a, b) = {
    let x = str.parse_int(a)?
    let y = str.parse_int(b)?
    Ok(x + y)
}
to_string(is_ok(parse_both("2", "40"))) + "," + to_string(is_err(parse_both("2", "oops")))
"#;
    assert_eq!(s(eval(src)), "true,true");
}

#[test]
fn propagation_crosses_only_one_call_boundary() {
    // The caller of g receives the Err as a value and can handle it —
    // it does not keep unwinding through h.
    let src = r#"
fn f() = Err("inner")
fn g() = {
    let v = f()?
    Ok(v)
}
fn h() = match g() {
    Ok(v) => "ok",
    Err(e) => "handled: " + e
}
h()
"#;
    assert_eq!(s(eval(src)), "handled: inner");
}

#[test]
fn question_mark_at_top_level_is_an_error() {
    assert!(eval_res("let x = Err(\"e\")?\nx").is_err());
}

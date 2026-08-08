//! Roadmap Tier 2: `return expr` (early exit from functions), `break value`
//! (loops as expressions), and `error` declarations with real semantics.

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

// ── return ─────────────────────────────────────────────────────────────

#[test]
fn return_exits_the_function_early() {
    let src = r#"
fn classify(n) = {
    if n < 0 => return "negative"
    if n == 0 => return "zero"
    "positive"
}
classify(-1) + "/" + classify(0) + "/" + classify(1)
"#;
    assert_eq!(s(eval(src)), "negative/zero/positive");
}

#[test]
fn bare_return_yields_unit() {
    let src = r#"
fn f(x) = {
    if x => return
    42
}
typeof(f(true)) + "/" + show(f(false))
"#;
    assert_eq!(s(eval(src)), "Unit/42");
}

#[test]
fn return_escapes_loops_within_the_function() {
    let src = r#"
fn first_even(xs) = {
    for x in xs { if x % 2 == 0 => return x }
    0 - 1
}
show(first_even([3, 5, 8, 9])) + "/" + show(first_even([1, 3]))
"#;
    assert_eq!(s(eval(src)), "8/-1");
}

#[test]
fn return_exits_only_the_nearest_function() {
    let src = r#"
fn inner(x) = { return x * 2 }
fn outer(x) = {
    let doubled = inner(x)
    doubled + 1
}
outer(10)
"#;
    assert_eq!(eval(src), Value::Integer(21));
}

#[test]
fn return_works_in_lambdas() {
    let src = r#"
let f = (x) => { if x => return "yes"
"no" }
f(true) + "/" + f(false)
"#;
    assert_eq!(s(eval(src)), "yes/no");
}

#[test]
fn return_at_top_level_is_an_error() {
    assert!(eval_res("return 5").is_err());
}

// ── break value ────────────────────────────────────────────────────────

#[test]
fn loop_breaks_with_a_value() {
    let src = r#"
let mut n = 0
loop {
    n = n + 1
    if n * n > 50 => break n
}
"#;
    assert_eq!(eval(src), Value::Integer(8));
}

#[test]
fn while_and_for_break_with_values() {
    let src = r#"
let mut i = 0
let w = while i < 100 { i = i + 1
if i == 7 => break i * 10 }
let f = for x in [1, 2, 3] { if x == 2 => break x + 100 }
show(w) + "/" + show(f)
"#;
    assert_eq!(s(eval(src)), "70/102");
}

#[test]
fn bare_break_behavior_is_unchanged() {
    let src = r#"
let mut total = 0
for i in 0..10 { if i > 4 => break
total = total + i }
total
"#;
    assert_eq!(eval(src), Value::Integer(10));
}

// ── error declarations ─────────────────────────────────────────────────

#[test]
fn error_variants_construct_and_match() {
    let src = r#"
error AppError { NotFound, Invalid: { msg: String } }
fn lookup(id) = {
    if id == 0 => return Err(NotFound)
    if id < 0 => return Err(Invalid("negative"))
    Ok(id * 10)
}
fn describe(r) = match r {
    Ok(v) => "ok " + show(v),
    Err(NotFound) => "missing",
    Err(Invalid(m)) => "bad: " + m
}
describe(lookup(4)) + "/" + describe(lookup(0)) + "/" + describe(lookup(0 - 2))
"#;
    assert_eq!(s(eval(src)), "ok 40/missing/bad: negative");
}

#[test]
fn error_variants_compare_and_typeof_like_enums() {
    let src = r#"
error E { A, B }
show(A == A) + "/" + show(A == B) + "/" + typeof(A)
"#;
    // typeof reports the declared error type's name, like structs and enums.
    assert_eq!(s(eval(src)), "true/false/E");
}

#[test]
fn error_unit_variants_do_not_shadow_bindings() {
    // A binding sub-pattern with an unrelated name still binds.
    let src = r#"
error E { Sentinel }
match 42 { x => x }
"#;
    assert_eq!(eval(src), Value::Integer(42));
}

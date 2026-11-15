//! `&&` and `||` short-circuit: the right operand is evaluated only when the
//! left doesn't already decide the result. Before this, both sides always ran,
//! so a guard like `x != 0 && y / x > 0` still divided by zero. Surfaced by
//! dogfooding a regex engine, whose backtracking matcher relies on it (the
//! `Star` progress guard would otherwise recurse forever).

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

#[test]
fn or_does_not_evaluate_right_when_left_is_true() {
    // `1 / 0` would raise; short-circuit means it is never reached.
    assert_eq!(eval("true || (1 / 0 == 0)"), Value::Boolean(true));
}

#[test]
fn and_does_not_evaluate_right_when_left_is_false() {
    assert_eq!(eval("false && (1 / 0 == 0)"), Value::Boolean(false));
}

#[test]
fn guard_pattern_protects_the_right_side() {
    // The canonical use: a bounds/zero check guarding the operation after it.
    assert_eq!(eval("(0 != 0) && ((10 / 0) > 1)"), Value::Boolean(false));
    assert_eq!(eval("(5 != 0) && ((10 / 5) > 1)"), Value::Boolean(true));
}

#[test]
fn both_sides_still_combine_when_the_left_does_not_decide() {
    assert_eq!(eval("true && false"), Value::Boolean(false));
    assert_eq!(eval("true && true"), Value::Boolean(true));
    assert_eq!(eval("false || true"), Value::Boolean(true));
    assert_eq!(eval("false || false"), Value::Boolean(false));
}

#[test]
fn non_short_circuit_type_errors_are_preserved() {
    // When the left doesn't short-circuit, a non-boolean right still errors,
    // matching the strict behavior of the operators.
    assert!(eval_res("true && 5").is_err());
}

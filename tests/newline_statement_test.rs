//! Newlines separate statements: a parenthesized expression (e.g. a tuple)
//! on the line after another statement is its own statement, not a call on
//! the previous value. Regression for `let a = [1,2]` / `(a, a)` parsing as
//! `[1,2](a, a)`.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

#[test]
fn tuple_as_final_block_expression() {
    // The classic failure: a block whose last expression is a bare tuple,
    // right after a let. Previously misparsed as a call.
    let src = r#"
fn pair(x) = {
    let a = x * 2
    (a, a)
}
pair(5)
"#;
    assert_eq!(
        eval(src),
        Value::Tuple(vec![Value::Integer(10), Value::Integer(10)].into())
    );
}

#[test]
fn parenthesized_statement_after_another() {
    // A parenthesized expression statement following another statement.
    let src = "let a = 5\n(a)\n";
    // Evaluates to the last expression, (a) == 5.
    assert_eq!(eval(src), Value::Integer(5));
}

#[test]
fn multiline_pipelines_still_parse() {
    let src = r#"
range(1, 6)
    |> map((x) => x * x)
    |> fold(0, (acc, x) => acc + x)
"#;
    assert_eq!(eval(src), Value::Integer(1 + 4 + 9 + 16 + 25));
}

#[test]
fn multiline_binary_expressions_still_parse() {
    let src = "let x = 1 + 2\n    + 3\n    + 4\nx";
    assert_eq!(eval(src), Value::Integer(10));
}

#[test]
fn multiline_call_arguments_still_parse() {
    let src = "fn add3(a, b, c) = a + b + c\nadd3(\n  1,\n  2,\n  3\n)";
    assert_eq!(eval(src), Value::Integer(6));
}

#[test]
fn same_line_calls_are_still_calls() {
    let src = "fn f(x) = x + 1\nf(10)";
    assert_eq!(eval(src), Value::Integer(11));
}

#[test]
fn trailing_comma_in_list_is_allowed() {
    assert_eq!(
        eval("[1, 2, 3,]"),
        Value::List(vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)].into())
    );
}

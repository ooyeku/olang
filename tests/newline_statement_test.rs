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
    // A sum continues when its line ends with the operator, or when the
    // next line starts with `+` or a binary-looking `- ` (a space after
    // it); `-1` glued to its operand is a statement of its own.
    let src = "let x = 1 + 2 +\n    3 +\n    4\nx";
    assert_eq!(eval(src), Value::Integer(10));
    assert_eq!(eval("let x = 1 + 2\n    + 3\nx"), Value::Integer(6));
    assert_eq!(eval("let x = 1.0 + 2.0\n    - 10.0\nx"), Value::Float(-7.0));
    assert_eq!(eval("let x = 10\n    - 2\n    + 5\n    -\n    1\nx"), Value::Integer(12));
    assert_eq!(eval("let x = 10\n    // why\n    - 2\nx"), Value::Integer(8));
    let src = "let x = 7\n-1\nx";
    assert_eq!(eval(src), Value::Integer(7));
    assert_eq!(eval("fn f() = {\n    let a = 3\n    -a\n}\nf()"), Value::Integer(-3));
    assert_eq!(eval("fn f() = {\n    - 3\n}\nf()"), Value::Integer(-3));
    let src = "let x = [1, 2]\n    |> len\n    == 2\nx";
    assert_eq!(eval(src), Value::Boolean(true));
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

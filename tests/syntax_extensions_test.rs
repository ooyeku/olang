//! Syntax and operator extensions surfaced by dogfooding a template engine:
//! list concatenation with `+`, `else if` chains, and multi-line `use` import
//! lists with a trailing comma.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

fn ints(v: Value) -> Vec<i64> {
    match v {
        Value::List(items) => items
            .iter()
            .map(|x| match x {
                Value::Integer(n) => *n,
                other => panic!("expected int, got {:?}", other),
            })
            .collect(),
        other => panic!("expected list, got {:?}", other),
    }
}

fn s(v: Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn list_plus_concatenates() {
    assert_eq!(ints(eval("[1, 2] + [3, 4]")), vec![1, 2, 3, 4]);
}

#[test]
fn list_plus_with_empty_operands() {
    assert_eq!(ints(eval("[] + [1]")), vec![1]);
    assert_eq!(ints(eval("[1] + []")), vec![1]);
    assert_eq!(ints(eval("[] + []")), Vec::<i64>::new());
}

#[test]
fn list_plus_builds_up_in_a_loop() {
    // The pattern the engine's lexer relies on: append one element per step.
    let src = r#"
let mut acc = []
let mut i = 0
while i < 4 {
    acc = acc + [i * i]
    i = i + 1
}
acc
"#;
    assert_eq!(ints(eval(src)), vec![0, 1, 4, 9]);
}

#[test]
fn else_if_chain_selects_the_right_arm() {
    let src = r#"
fn classify(n) = if n > 0 => "pos"
    else if n < 0 => "neg"
    else => "zero"
classify(-2) + "/" + classify(0) + "/" + classify(5)
"#;
    assert_eq!(s(eval(src)), "neg/zero/pos");
}

#[test]
fn else_if_chain_of_several_arms() {
    let src = r#"
fn grade(x) = if x >= 90 => "A"
    else if x >= 80 => "B"
    else if x >= 70 => "C"
    else => "F"
grade(95) + grade(85) + grade(72) + grade(40)
"#;
    assert_eq!(s(eval(src)), "ABCF");
}

#[test]
fn plain_if_else_still_parses_before_a_declaration() {
    // Guards the grammar change: an `if/else` function body followed by
    // another declaration must still parse.
    let src = r#"
fn a(x) = if x => "y" else => "n"
fn b(x) = a(x)
b(true)
"#;
    assert_eq!(s(eval(src)), "y");
}

#[test]
fn multiline_use_list_with_trailing_comma_parses() {
    // Parsing only — the module need not resolve; we just assert the import
    // list spanning lines with a trailing comma is accepted by the grammar.
    let src = "use lib.things {\n    alpha,\n    beta,\n    gamma,\n}\n";
    assert!(Parser::new().parse(src).is_ok());
}

#[test]
fn single_line_use_list_still_parses() {
    let src = "use lib.things { alpha, beta }\n";
    assert!(Parser::new().parse(src).is_ok());
}

#[test]
fn let_with_a_forgotten_equals_is_a_parse_error() {
    // `let scores #{...}` used to silently parse as an uninitialized `let`
    // plus a stray expression statement, leaving `scores` bound to Unit.
    // A let must either have an initializer or end the statement.
    assert!(
        Parser::new()
            .parse("let scores #{ \"ada\": 99 }\n")
            .is_err()
    );
    assert!(Parser::new().parse("let x [1, 2, 3]\n").is_err());
    assert!(Parser::new().parse("let y \"oops\"\n").is_err());
}

#[test]
fn uninitialized_let_still_parses_and_is_unit() {
    assert!(Parser::new().parse("let pending\n").is_ok());
    assert!(
        Parser::new()
            .parse("let pending // fill in later\n")
            .is_ok()
    );
    assert!(Parser::new().parse("let typed: Int\n").is_ok());
    // Unbound-until-assigned semantics are unchanged.
    let src = "let pending\ntypeof(pending)";
    match eval(src) {
        Value::String(s) => assert_eq!(s.to_string(), "Unit"),
        other => panic!("expected type name, got {:?}", other),
    }
}

#[test]
fn assertions_accept_multiline_arguments() {
    // A multi-line assert_eq used to fall out of the assertion grammar and
    // parse as a call to an undefined `assert_eq` function.
    let src = "test \"multiline\" {\n    assert_eq(\n        1 + 1,\n        2\n    )\n    assert_eq(\"a\" + \"b\",\n        \"ab\", \"concat\")\n    assert(\n        true\n    )\n}\n\"done\"";
    match eval(src) {
        Value::String(s) => assert_eq!(s.to_string(), "done"),
        other => panic!("expected done, got {:?}", other),
    }
}

#[test]
fn multiline_assertion_failures_still_fail() {
    let src = "test \"fails\" {\n    assert_eq(\n        1,\n        2\n    )\n}";
    let program = Parser::new().parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    assert!(interpreter.eval_program(program).is_err());
}

#[test]
fn let_mut_is_one_statement_with_no_stray_binding() {
    // `let mut x = 1` must not leave a binding named `mut` behind.
    let src = "let mut x = 1\nx = x + 1\nx";
    assert_eq!(eval(src), Value::Integer(2));
    let parser = Parser::new();
    let program = parser.parse("let mut x = 1\ntypeof(mut)").expect("parse");
    let mut interpreter = Interpreter::new();
    assert!(
        interpreter.eval_program(program).is_err(),
        "`mut` must not be bound by `let mut x`"
    );
}

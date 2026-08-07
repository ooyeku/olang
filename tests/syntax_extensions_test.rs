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

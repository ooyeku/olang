//! Enum variant construction: unit and tuple variants, pattern matching,
//! equality, typeof, arity checking, and bytecode-tier transparency.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

/// Evaluate with the bytecode tier enabled (first-call promotion).
fn eval_tiered(src: &str) -> Result<Value, String> {
    let parser = Parser::new();
    let program = parser.parse(src).map_err(|e| e.to_string())?;
    let mut interpreter = Interpreter::new();
    interpreter.enable_bytecode_tier(1, false);
    interpreter.eval_program(program).map_err(|e| e.to_string())
}

#[test]
fn unit_variant_constructs() {
    let src = r#"
type Color = enum { Red, Green, Blue }
typeof(Red)
"#;
    assert_eq!(eval(src), Value::String("Color".to_string().into()));
}

#[test]
fn tuple_variant_constructs_and_matches() {
    let src = r#"
type Shape = enum { Circle(Float), Rectangle(Float, Float) }
fn area(s) = match s {
    Circle(r) => 3 * r * r,
    Rectangle(w, h) => w * h
}
[area(Circle(2.0)), area(Rectangle(3.0, 4.0))]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::Float(12.0));
            assert_eq!(items[1], Value::Float(12.0));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn unit_variants_are_equal_by_variant() {
    let src = r#"
type Color = enum { Red, Green }
[Red == Red, Red == Green]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::Boolean(true));
            assert_eq!(items[1], Value::Boolean(false));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn tuple_variants_are_equal_by_payload() {
    let src = r#"
type Point = enum { P(Int, Int) }
[P(1, 2) == P(1, 2), P(1, 2) == P(1, 3)]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::Boolean(true));
            assert_eq!(items[1], Value::Boolean(false));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn generic_style_enum_works_for_any_payload() {
    // Type parameters are erased at runtime, so a "generic" Option constructs
    // for any argument.
    let src = r#"
type Option = enum { Some(Int), None }
fn get(o, d) = match o {
    Some(v) => v,
    None => d
}
get(Some(42), 0) + get(None, -1)
"#;
    assert_eq!(eval(src), Value::Integer(41));
}

#[test]
fn wrong_arity_is_an_error() {
    let src = r#"
type Shape = enum { Circle(Float) }
Circle(1.0, 2.0)
"#;
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    assert!(interpreter.eval_program(program).is_err());
}

#[test]
fn nested_enum_patterns_bind() {
    let src = r#"
type Tree = enum { Leaf(Int), Node(Int, Int) }
fn sum(t) = match t {
    Leaf(n) => n,
    Node(a, b) => a + b
}
sum(Leaf(5)) + sum(Node(3, 4))
"#;
    assert_eq!(eval(src), Value::Integer(12));
}

#[test]
fn enum_using_functions_are_tier_transparent() {
    // A function that constructs/matches enums references global constructors,
    // so the tier leaves it interpreted — but the result must be identical
    // with the tier on and off.
    let src = r#"
type Shape = enum { Circle(Int), Square(Int), Empty }
fn area(s) = match s {
    Circle(r) => r * r * 3,
    Square(w) => w * w,
    Empty => 0
}
fn total() = {
    let shapes = [Circle(2), Square(3), Empty, Circle(1)]
    let mut acc = 0
    for s in shapes {
        acc = acc + area(s)
    }
    acc
}
total()
"#;
    let plain = eval(src);
    let tiered = eval_tiered(src).expect("tiered eval");
    assert_eq!(plain, tiered);
    // Circle(2)=12, Square(3)=9, Empty=0, Circle(1)=3
    assert_eq!(plain, Value::Integer(24));
}

#[test]
fn unit_variants_dispatch_correctly_in_match() {
    // Regression: a unit-variant pattern must match by variant, not bind as
    // a catch-all. Before the fix, `North =>` captured every direction.
    let src = r#"
type Direction = enum { North, South, East, West }
fn dx(d) = match d {
    North => 0,
    South => 0,
    East => 1,
    West => 0 - 1
}
[dx(North), dx(South), dx(East), dx(West)]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(items[0], Value::Integer(0));
            assert_eq!(items[1], Value::Integer(0));
            assert_eq!(items[2], Value::Integer(1));
            assert_eq!(items[3], Value::Integer(-1));
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn a_plain_binding_still_binds() {
    // A lowercase name that is NOT an enum variant still binds normally.
    let src = r#"
fn describe(x) = match x {
    0 => "zero",
    other => "got " + to_string(other)
}
describe(7)
"#;
    assert_eq!(eval(src), Value::String("got 7".to_string().into()));
}

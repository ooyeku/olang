//! An identifier pattern binds a fresh variable unless its name is a *declared*
//! enum variant. The variant test used to fire whenever the name merely
//! resolved to a unit-variant value in scope — so a binding sub-pattern like
//! `b` in `Node(a, b)` silently became an equality test and failed to match
//! whenever some `b` already in scope held a unit variant. Surfaced by
//! dogfooding a regex engine (`Concat(a, b)` with `b` bound to the `$` anchor).

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    let mut interp = Interpreter::new();
    interp.eval_program(program).expect("eval")
}

fn s(v: Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn identifier_pattern_binds_even_when_it_shadows_a_unit_variant_value() {
    // `g` holds a unit variant, but `g` is not itself a declared variant name,
    // so the pattern `g` binds the scrutinee rather than testing equality.
    let src = r#"
type Color = enum { Red, Green }
let g = Green
match 42 { g => g }
"#;
    assert_eq!(eval(src), Value::Integer(42));
}

#[test]
fn declared_unit_variants_still_match_by_equality() {
    let src = r#"
type Color = enum { Red, Green }
match Red { Red => "red", Green => "green" }
"#;
    assert_eq!(s(eval(src)), "red");
}

#[test]
fn nested_binding_subpattern_works_with_a_unit_variant_sibling() {
    // The regex shape: an outer `Node(a, b)` binds `b` to the unit variant
    // `Nil`, then recurses into `a`, which must re-bind `a`/`b` for a deeper
    // `Node`. Before the fix the inner `b` failed to bind.
    let src = r#"
type Tree = enum { Leaf(Int), Node(Tree, Tree), Nil }
fn sum(t) = match t {
    Nil => 0,
    Leaf(n) => n,
    Node(a, b) => sum(a) + sum(b)
}
sum(Node(Node(Leaf(1), Leaf(2)), Nil))
"#;
    assert_eq!(eval(src), Value::Integer(3));
}

#[test]
fn unit_variant_as_a_nested_pattern_still_tests_equality() {
    // Ensure the fix didn't turn a genuine nested variant pattern into a bind.
    let src = r#"
type Opt = enum { None, Some(Int) }
type Box = enum { Wrap(Opt) }
match Wrap(None) { Wrap(None) => "empty", Wrap(Some(n)) => "val" }
"#;
    assert_eq!(s(eval(src)), "empty");
}

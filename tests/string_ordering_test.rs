//! Strings compare with the ordering operators (`<`, `<=`, `>`, `>=`), not
//! only `==`/`!=`. The interpreter rejected them with "Invalid binary
//! operation" while the bytecode tier accepted them — a tier inconsistency
//! surfaced by dogfooding a parser combinator library, where character-range
//! checks like `c >= "0" && c <= "9"` are everywhere. Ordering is by Unicode
//! scalar value, lexicographically.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    let mut interp = Interpreter::new();
    interp.eval_program(program).expect("eval")
}

fn b(v: Value) -> bool {
    match v {
        Value::Boolean(b) => b,
        other => panic!("expected bool, got {:?}", other),
    }
}

fn s(v: Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn less_and_greater_on_single_chars() {
    assert!(b(eval(r#""a" < "b""#)));
    assert!(b(eval(r#""b" > "a""#)));
    assert!(!b(eval(r#""b" < "a""#)));
}

#[test]
fn less_equal_and_greater_equal() {
    assert!(b(eval(r#""a" <= "a""#)));
    assert!(b(eval(r#""a" <= "b""#)));
    assert!(b(eval(r#""b" >= "b""#)));
    assert!(!b(eval(r#""b" <= "a""#)));
}

#[test]
fn digit_range_check() {
    // The parser-combinator use case.
    assert!(b(eval(r#"("5" >= "0") && ("5" <= "9")"#)));
    assert!(!b(eval(r#"("a" >= "0") && ("a" <= "9")"#)));
}

#[test]
fn lexicographic_multichar() {
    assert!(b(eval(r#""apple" < "banana""#)));
    assert!(b(eval(r#""app" < "apple""#))); // prefix sorts first
    assert!(b(eval(r#""Zebra" < "apple""#))); // uppercase precedes lowercase
}

#[test]
fn sort_orders_strings() {
    assert_eq!(
        s(eval(r#"sort(["pear", "apple", "cherry", "banana"])[0]"#)),
        "apple"
    );
}

#[test]
fn equality_still_works() {
    assert!(b(eval(r#""x" == "x""#)));
    assert!(b(eval(r#""x" != "y""#)));
}

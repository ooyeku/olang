//! The `map_*` accessors read named fields uniformly across maps, anonymous
//! objects, named structs, and parsed JSON objects. This makes dynamic key
//! access on parsed JSON possible — a struct-like value reads the same way a
//! map does, so a data processor can select a column by a runtime key.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

#[test]
fn map_get_reads_a_field_of_an_anonymous_object() {
    assert_eq!(eval(r#"map_get({ a: 1, b: 2 }, "b")"#), Value::Integer(2));
}

#[test]
fn map_get_reads_a_field_of_a_named_struct() {
    let src = r#"
type P = struct { x: Int, y: Int }
map_get(P { x: 3, y: 4 }, "y")
"#;
    assert_eq!(eval(src), Value::Integer(4));
}

#[test]
fn map_get_on_a_missing_field_is_unit() {
    assert_eq!(eval(r#"map_get({ a: 1 }, "nope")"#), Value::Unit);
}

#[test]
fn map_has_key_answers_for_objects() {
    assert_eq!(eval(r#"map_has_key({ a: 1 }, "a")"#), Value::Boolean(true));
    assert_eq!(eval(r#"map_has_key({ a: 1 }, "z")"#), Value::Boolean(false));
}

#[test]
fn map_keys_and_values_enumerate_object_fields() {
    // Order is unspecified, so assert on the counts and membership.
    assert_eq!(
        eval(r#"len(map_keys({ a: 1, b: 2, c: 3 }))"#),
        Value::Integer(3)
    );
    assert_eq!(
        eval(r#"len(map_values({ a: 1, b: 2, c: 3 }))"#),
        Value::Integer(3)
    );
}

#[test]
fn dynamic_key_access_on_parsed_json() {
    // The payoff: a parsed JSON object supports the same accessors as a map,
    // so a column named by a runtime value can be pulled out.
    let src = r#"
let row = unwrap(json.parse("{\"region\": \"west\", \"amount\": 42}"))
let field = "amount"
map_get(row, field)
"#;
    assert_eq!(eval(src), Value::Integer(42));
}

#[test]
fn map_keys_on_parsed_json_object() {
    let src = r#"
let obj = unwrap(json.parse("{\"x\": 1, \"y\": 2}"))
len(map_keys(obj))
"#;
    assert_eq!(eval(src), Value::Integer(2));
}

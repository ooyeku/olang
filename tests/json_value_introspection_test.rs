//! The JSON value model that a schema validator (and any JSON-walking program)
//! depends on: `typeof` distinguishes every parsed-JSON kind, `null` reads as
//! `Unit`, and `map_has_key` tells a present-but-null field from an absent one.
//! No bug was found dogfooding a JSON Schema validator — this pins the surface
//! it relies on so it can't silently regress.

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
fn typeof_distinguishes_every_json_kind() {
    let src = r#"
let d = unwrap(json.parse("{\"s\":\"x\",\"i\":42,\"f\":3.5,\"b\":true,\"n\":null,\"a\":[1],\"o\":{\"k\":1}}"))
typeof(map_get(d, "s")) + "," + typeof(map_get(d, "i")) + "," +
typeof(map_get(d, "f")) + "," + typeof(map_get(d, "b")) + "," +
typeof(map_get(d, "n")) + "," + typeof(map_get(d, "a")) + "," +
typeof(map_get(d, "o"))
"#;
    assert_eq!(s(eval(src)), "String,Int,Float,Bool,Unit,List,JsonObject");
}

#[test]
fn json_null_reads_as_unit() {
    let src = r#"typeof(map_get(unwrap(json.parse("{\"n\":null}")), "n"))"#;
    assert_eq!(s(eval(src)), "Unit");
}

#[test]
fn map_has_key_separates_present_null_from_absent() {
    let src = r#"
let d = unwrap(json.parse("{\"n\":null}"))
to_string(map_has_key(d, "n")) + "," + to_string(map_has_key(d, "missing"))
"#;
    assert_eq!(s(eval(src)), "true,false");
}

#[test]
fn integer_and_number_are_distinguishable() {
    // "integer" vs "number" schema types hinge on this distinction.
    let src = r#"
let d = unwrap(json.parse("{\"i\":10,\"f\":10.0}"))
typeof(map_get(d, "i")) + "," + typeof(map_get(d, "f"))
"#;
    assert_eq!(s(eval(src)), "Int,Float");
}

#[test]
fn parsed_json_values_compare_for_enum_membership() {
    let src = r#"
let allowed = unwrap(json.parse("[\"red\",\"green\"]"))
let v = map_get(unwrap(json.parse("{\"c\":\"green\"}")), "c")
let mut found = false
for a in allowed { if a == v => { found = true } }
found
"#;
    assert_eq!(eval(src), Value::Boolean(true));
}

#[test]
fn nested_objects_and_arrays_traverse() {
    let src = r#"
let d = unwrap(json.parse("{\"u\":{\"tags\":[\"a\",\"b\",\"c\"]}}"))
len(map_get(map_get(d, "u"), "tags"))
"#;
    assert_eq!(eval(src), Value::Integer(3));
}

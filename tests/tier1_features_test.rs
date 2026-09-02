//! Roadmap Tier 1: display rendering (`show`), map/object iteration
//! (`entries`, `for (k, v)`), write-side struct-likeness for the map
//! builtins, and strings iterating by character.

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

// ── show ───────────────────────────────────────────────────────────────

#[test]
fn show_renders_strings_bare_and_others_like_to_string() {
    assert_eq!(s(eval(r#"show("hi")"#)), "hi");
    assert_eq!(s(eval("show(42)")), "42");
    assert_eq!(s(eval("show(3.5)")), "3.5");
    // to_string agrees: a string is already its own text (the quoted
    // form was a trap for generic code; meta.lit renders the literal).
    assert_eq!(s(eval(r#"to_string("hi")"#)), "hi");
    assert_eq!(s(eval(r#"to_string(["hi"])"#)), "[\"hi\"]");
}

// ── entries ────────────────────────────────────────────────────────────

#[test]
fn entries_yields_sorted_key_value_tuples() {
    let src = r#"
let parts = entries(#{ "b": 2, "a": 1 }) |> map((p) => p[0] + "=" + show(p[1]))
join(parts, ",")
"#;
    assert_eq!(s(eval(src)), "a=1,b=2");
}

#[test]
fn entries_reads_objects_and_parsed_json() {
    assert_eq!(eval("len(entries({ x: 1, y: 2 }))"), Value::Integer(2));
    let src = r#"len(entries(unwrap(json.parse("{\"k\": 9}"))))"#;
    assert_eq!(eval(src), Value::Integer(1));
}

// ── write-side struct-likeness ─────────────────────────────────────────

#[test]
fn map_set_updates_parsed_json_preserving_its_kind() {
    let src = r#"
let d = unwrap(json.parse("{\"a\": 1}"))
let d2 = map_set(d, "b", 2)
typeof(d2) + ":" + show(map_get(d2, "b")) + ":" + show(map_has_key(d, "b"))
"#;
    assert_eq!(s(eval(src)), "JsonObject:2:false");
}

#[test]
fn map_set_updates_structs_preserving_type_name() {
    let src = r#"
type P = struct { x: Int }
let p2 = map_set(P { x: 1 }, "y", 2)
typeof(p2) + ":" + show(map_get(p2, "y"))
"#;
    assert_eq!(s(eval(src)), "P:2");
}

#[test]
fn map_remove_works_on_objects() {
    let src = r#"show(map_has_key(map_remove({ a: 1, b: 2 }, "a"), "a"))"#;
    assert_eq!(s(eval(src)), "false");
}

#[test]
fn map_set_on_a_plain_map_still_returns_a_map() {
    let src = r#"typeof(map_set(#{ "a": 1 }, "b", 2))"#;
    assert_eq!(s(eval(src)), "Map");
}

// ── for-loop upgrades ──────────────────────────────────────────────────

#[test]
fn for_iterates_strings_by_character() {
    let src = r#"
let mut out = ""
for c in "abc" { out = out + c + "." }
out
"#;
    assert_eq!(s(eval(src)), "a.b.c.");
}

#[test]
fn for_iterates_unicode_by_character_not_byte() {
    let src = r#"
let mut n = 0
for c in "héllo" { n = n + 1 }
n
"#;
    assert_eq!(eval(src), Value::Integer(5));
}

#[test]
fn for_destructures_tuple_bindings() {
    let src = r#"
let mut out = ""
for (i, x) in enumerate(["a", "b"]) { out = out + show(i) + ":" + x + " " }
out
"#;
    assert_eq!(s(eval(src)), "0:a 1:b ");
}

#[test]
fn for_destructures_entries_pairs() {
    let src = r#"
let mut out = ""
for (k, v) in entries(#{ "b": 2, "a": 1 }) { out = out + k + "=" + show(v) + " " }
out
"#;
    assert_eq!(s(eval(src)), "a=1 b=2 ");
}

#[test]
fn for_destructures_three_names() {
    let src = r#"
let mut total = 0
for (a, b, c) in [(1, 2, 3), (4, 5, 6)] { total = total + a + b + c }
total
"#;
    assert_eq!(eval(src), Value::Integer(21));
}

#[test]
fn plain_for_loops_are_unchanged() {
    let src = r#"
let mut total = 0
for x in [1, 2, 3] { total = total + x }
for i in 0..3 { total = total + i }
total
"#;
    assert_eq!(eval(src), Value::Integer(9));
}

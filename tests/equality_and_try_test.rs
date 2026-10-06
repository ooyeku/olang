//! Two fixes surfaced by writing the language reference: structural equality
//! for compound values, and `?` actually propagating an `Err` to the caller
//! instead of aborting the program.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    let mut interp = Interpreter::new();
    interp.eval_program(program).expect("eval")
}

fn eval_res(src: &str) -> Result<Value, String> {
    let program = Parser::new().parse(src).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    interp.eval_program(program).map_err(|e| e.to_string())
}

fn s(v: Value) -> String {
    match v {
        Value::String(ref s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

// ── structural equality ────────────────────────────────────────────────

#[test]
fn lists_compare_structurally() {
    assert_eq!(eval("[1, 2] == [1, 2]"), Value::Boolean(true));
    assert_eq!(eval("[1, 2] == [1, 3]"), Value::Boolean(false));
    assert_eq!(eval("[1, 2] != [1, 3]"), Value::Boolean(true));
    assert_eq!(eval("[[1], [2]] == [[1], [2]]"), Value::Boolean(true));
}

#[test]
fn tuples_compare_structurally() {
    assert_eq!(eval(r#"(1, "a") == (1, "a")"#), Value::Boolean(true));
    assert_eq!(eval(r#"(1, "a") == (1, "b")"#), Value::Boolean(false));
}

#[test]
fn maps_compare_structurally() {
    assert_eq!(
        eval(r#"#{ "k": 1, "j": 2 } == #{ "j": 2, "k": 1 }"#),
        Value::Boolean(true)
    );
    assert_eq!(eval(r#"#{ "k": 1 } != #{ "k": 2 }"#), Value::Boolean(true));
}

#[test]
fn structs_and_objects_compare_structurally() {
    assert_eq!(eval("{ x: 1 } == { x: 1 }"), Value::Boolean(true));
    assert_eq!(eval("{ x: 1 } == { x: 2 }"), Value::Boolean(false));
    let src = r#"
type P = struct { x: Int }
P { x: 1 } == P { x: 1 }
"#;
    assert_eq!(eval(src), Value::Boolean(true));
}

// ── `?` propagation ────────────────────────────────────────────────────

#[test]
fn assert_eq_compares_as_equality_does() {
    // `==` holds between a parsed JSON object and the `#{}` map it reads
    // as; `assert_eq` failed on the same pair (Foundry). Every form: the
    // statement, the call in expression position, and testing.assert_eq.
    let src = r#"
let j = unwrap(json.parse("{\"a\":1,\"b\":[{\"c\":2}]}"))
let m = #{ "a": 1, "b": [#{ "c": 2 }] }
assert_eq(j, m)
assert_eq(json.parse("{\"a\":1}"), Ok(#{ "a": 1 }))
let inline = match true { true => assert_eq([j], [m]), false => () }
assert_ne(j, #{ "a": 2 })
[j == m, is_ok(testing.assert_eq(j, m)), is_err(testing.assert_eq(j, #{ "a": 2 }))]
"#;
    assert_eq!(eval(src), Value::List(vec![Value::Boolean(true); 3].into()));
    let e = eval_res("assert_eq(unwrap(json.parse(\"{\\\"a\\\":1}\")), #{ \"a\": 2 })")
        .expect_err("unequal contents still fail");
    assert!(e.contains("Assertion failed") && e.contains("!="), "{e}");
}

#[test]
fn question_mark_unwraps_ok() {
    let src = r#"
fn f() = Ok(41)
fn g() = {
    let v = f()?
    Ok(v + 1)
}
to_string(g())
"#;
    assert_eq!(s(eval(src)), "Ok(42)");
}

#[test]
fn question_mark_returns_the_err_to_the_caller() {
    let src = r#"
fn f() = Err("boom")
fn g() = {
    let v = f()?
    Ok(v + 1)
}
to_string(g())
"#;
    assert_eq!(s(eval(src)), "Err(\"boom\")");
}

#[test]
fn question_mark_chains_stop_at_the_first_err() {
    let src = r#"
fn parse_both(a, b) = {
    let x = str.parse_int(a)?
    let y = str.parse_int(b)?
    Ok(x + y)
}
to_string(is_ok(parse_both("2", "40"))) + "," + to_string(is_err(parse_both("2", "oops")))
"#;
    assert_eq!(s(eval(src)), "true,true");
}

#[test]
fn propagation_crosses_only_one_call_boundary() {
    // The caller of g receives the Err as a value and can handle it —
    // it does not keep unwinding through h.
    let src = r#"
fn f() = Err("inner")
fn g() = {
    let v = f()?
    Ok(v)
}
fn h() = match g() {
    Ok(v) => "ok",
    Err(e) => "handled: " + e
}
h()
"#;
    assert_eq!(s(eval(src)), "handled: inner");
}

#[test]
fn question_mark_at_top_level_is_an_error() {
    assert!(eval_res("let x = Err(\"e\")?\nx").is_err());
}

// ── functions ───────────────────────────────────────────────────────

#[test]
fn inside_a_value_a_function_equals_itself_and_not_one_over_other_values() {
    let v = eval(
        r#"
        fn mk(x) = (y) => x + y
        let f = mk(1)
        let g = f
        // `==` refuses two bare functions; inside a value they compare
        [[f] == [g], [f] == [mk(2)], #{ "f": len } == #{ "f": len }, [len] == [str.length]]
    "#,
    );
    assert_eq!(format!("{v}"), "[true, false, true, false]");
}

#[test]
fn comparing_closures_does_not_walk_what_they_capture() {
    // A closure captures the module's functions, each with its own
    // environment; comparing them structurally took seconds or nothing,
    // depending on hash order. Captured functions compare by identity.
    let src = r#"
        fn helper1(x) = x + 1
        fn helper2(x) = helper1(x) * 2
        fn helper3(x) = helper2(x) - helper1(x)
        fn view(m) = #{ "rows": #{ "count": 1000000, "cell": (i, j) => helper3(m + i + j) } }
        let mut same = 0
        let t0 = time.monotonic()
        for k in 0..2000 {
            let a = view(k)
            let b = view(k + 1)
            if map_get(a, "rows") == map_get(b, "rows") => { same = same + 1 }
        }
        [same, time.monotonic() - t0 < 2000.0]
    "#;
    assert_eq!(format!("{}", eval(src)), "[0, true]");
}

// Compiled code (a function hot enough for the bytecode tier) compares
// collections in place: a value with itself at once, others element by
// element, with the interpreter's answers — `[1] != [1.0]` inside a
// collection, `1 == 1.0` at the top.
#[test]
fn compiled_equality_reads_collections_in_place() {
    let v = eval(
        r#"
fn same(a, b) = a == b
let big = map(range(0, 2000), (i) => #{ "a": i, "b": [i, "x"] })
let other = map(range(0, 2000), (i) => #{ "a": i, "b": [i, "x"] })
let off = map(range(0, 2000), (i) => #{ "a": i, "b": [i, if i == 1999 => "y" else => "x"] })
let mut out = []
for i in range(0, 200) {
    out = [same(big, big), same(big, other), same(big, off), same([1], [1.0]), same(#{ "k": [1, 2] }, #{ "k": [1, 2] }),
           same((1, "a"), (1, "b")), same(1, 1.0), same(#{ "m": #{ "n": 1 } }, #{ "m": #{ "n": 1.0 } })]
}
show(out)
"#,
    );
    assert_eq!(s(v), "[true, true, false, false, true, false, true, false]");
}

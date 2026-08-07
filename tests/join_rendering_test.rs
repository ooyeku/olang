//! `join` renders string elements by their content, not their quoted debug
//! form. Regression: it used `Value::to_string()`, so `join(["a", "b"], ",")`
//! produced `"a","b"` instead of `a,b` — surfaced building an audit log in a
//! state-machine engine.

use olang::{Interpreter, Parser, Value};

fn s(src: &str) -> String {
    let program = Parser::new().parse(src).expect("parse");
    let mut interp = Interpreter::new();
    match interp.eval_program(program).expect("eval") {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn join_strings_are_bare() {
    assert_eq!(s(r#"join(["a", "b", "c"], ",")"#), "a,b,c");
}

#[test]
fn join_keeps_a_multichar_separator() {
    assert_eq!(s(r#"join(["x", "y"], " -> ")"#), "x -> y");
}

#[test]
fn join_still_renders_non_strings() {
    assert_eq!(s(r#"join([1, 2, 3], "-")"#), "1-2-3");
}

#[test]
fn join_empty_list_is_empty_string() {
    assert_eq!(s(r#"join([], ",")"#), "");
}

#[test]
fn join_matches_str_join_for_strings() {
    // The two joins should agree on the common case.
    assert_eq!(
        s(r#"join(["p", "q"], "/")"#),
        s(r#"str.join(["p", "q"], "/")"#)
    );
}

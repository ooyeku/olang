//! The 1.0 bug hunt's stdlib answers (OL-385, OL-343, OL-341, OL-335,
//! OL-377, OL-380): what a call answers, as stdlib.md's rules say — input
//! that can be wrong is a `Result`, misuse raises, a position is never
//! said as a wrapped unsigned.

use olang::{Interpreter, Parser};

/// The program's last value, shown as olang shows it (`show`).
fn show(src: &str) -> String {
    let program = Parser::new().parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    let v = interpreter.eval_program(program).expect("eval");
    v.to_string()
}

// OL-385: a key of the wrong length or not hex is an Err, not a raise
#[test]
fn aes_answers_err_for_a_key_it_cannot_take() {
    let out = show("crypto.encrypt_aes(\"x\", \"short\")");
    assert!(out.starts_with("Err("), "{out}");
    assert!(out.contains("key must be valid hex"), "{out}");
    let out = show("crypto.encrypt_aes(\"x\", \"abcd\")");
    assert!(out.contains("key must be 32 bytes"), "{out}");
    let out = show("crypto.decrypt_aes(\"zz\", crypto.random_hex(32))");
    assert!(out.starts_with("Err("), "{out}");
    let out = show("crypto.decrypt_aes(\"00\", \"abcd\", \"00\")");
    assert!(out.starts_with("Err("), "{out}");
    // a round trip still works
    let out = show(
        "let k = crypto.random_hex(32)\ncrypto.decrypt_aes(unwrap(crypto.encrypt_aes(\"hi\", k)), k)",
    );
    assert_eq!(out, "Ok(\"hi\")");
}

// OL-343: a negative position is said as itself, never as 18446744073709551615
#[test]
fn a_negative_position_is_said_negative() {
    let out = show("json.array_get(\"[1,2]\", -1)");
    assert!(out.contains("position -1 is negative"), "{out}");
    assert!(!out.contains("18446744073709551615"), "{out}");
    let out = show("attempt(() => csv.read_cell(unwrap(csv.parse(\"a,b\")), 0, -1))");
    assert!(out.contains("column position -1 is negative"), "{out}");
    let out = show("attempt(() => ods.series([1, 2]) + ods.series([1, 2, 3]))");
    assert!(out.contains("ods: series length mismatch: 2 vs 3"), "{out}");
}

// OL-341: `$1_n` names no group: an Err that says to brace it, never a silent ""
#[test]
fn a_replacement_naming_no_group_is_an_err() {
    let out = show("re.replace(\"(\\\\d+)\", \"a1 b2\", \"$1_n\")");
    assert!(out.starts_with("Err("), "{out}");
    assert!(out.contains("write `${1}_n`"), "{out}");
    assert_eq!(show("re.replace(\"(\\\\d+)\", \"a1 b2\", \"${1}_n\")"), "Ok(\"a1_n b2_n\")");
    assert_eq!(show("re.replace(\"(\\\\d+)\", \"a1 b2\", \"<$1>\")"), "Ok(\"a<1> b<2>\")");
    assert_eq!(show("re.replace(\"(?P<n>\\\\d+)\", \"a1\", \"$n\")"), "Ok(\"a1\")");
    assert_eq!(show("re.replace(\"a\", \"a\", \"$$\")"), "Ok(\"$\")");
}

// OL-335: csv.stringify answers one shape: Ok(text), or Err
#[test]
fn csv_stringify_is_always_a_result() {
    assert_eq!(show("csv.stringify([[\"a\", \"b\"], [\"1\", \"2\"]])"), "Ok(\"a,b\\n1,2\\n\")");
    assert!(show("csv.stringify([[\"a\"], [\"b\", \"c\"]])").starts_with("Err("));
    assert!(show("csv.stringify([[()]])").starts_with("Err("));
    assert_eq!(show("unwrap(csv.stringify([[\"x\"]]))"), "\"x\\n\"");
}

// OL-380: a tree kill of a child spawned without group answers Err, as the docs say
#[test]
fn a_tree_kill_without_group_is_an_err() {
    let out = show(
        "let p = unwrap(proc.spawn([\"sleep\", \"5\"]))\nlet r = proc.kill(p, #{ \"tree\": true })\nlet _k = proc.kill(p)\nr",
    );
    assert!(out.starts_with("Err("), "{out}");
    assert!(out.contains("group: true"), "{out}");
}

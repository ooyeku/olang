//! `os.exec(program, args)` runs an external program to completion and returns
//! `{ code, stdout, stderr }`, or an `Err` when the program can't be launched.
//! Added so an olang program can drive other programs — e.g. the examples test
//! harness (`examples/run_all.ol`) runs every example in a subprocess.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    let mut interp = Interpreter::new();
    interp.eval_program(program).expect("eval")
}

#[test]
fn exec_captures_exit_code_zero() {
    assert_eq!(
        eval(r#"unwrap(os.exec("echo", ["hi"])).code"#),
        Value::Integer(0)
    );
}

#[test]
fn exec_captures_stdout() {
    let v = eval(r#"str.trim(unwrap(os.exec("echo", ["hello"])).stdout)"#);
    match v {
        Value::String(s) => assert_eq!(s.to_string(), "hello"),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn exec_reports_a_nonzero_exit_code() {
    // `sh -c "exit 7"` exits 7; the code is captured, not turned into an Err.
    assert_eq!(
        eval(r#"unwrap(os.exec("sh", ["-c", "exit 7"])).code"#),
        Value::Integer(7)
    );
}

#[test]
fn exec_returns_err_when_the_program_is_missing() {
    let src = r#"
match os.exec("this_binary_does_not_exist_37f2", []) {
    Ok(v) => "ran",
    Err(e) => "launch failed"
}
"#;
    match eval(src) {
        Value::String(s) => assert_eq!(s.to_string(), "launch failed"),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn exec_rejects_a_non_string_in_the_argument_list() {
    let src = r#"
match os.exec("echo", [1, 2]) {
    Ok(v) => "ran",
    Err(e) => "bad args"
}
"#;
    match eval(src) {
        Value::String(s) => assert_eq!(s.to_string(), "bad args"),
        other => panic!("expected string, got {:?}", other),
    }
}

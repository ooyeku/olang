//! `os.exec(argv)` runs an external program to completion and returns
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
        eval(r#"unwrap(os.exec(["echo", "hi"])).code"#),
        Value::Integer(0)
    );
}

#[test]
fn exec_captures_stdout() {
    let v = eval(r#"str.trim(unwrap(os.exec(["echo", "hello"])).stdout)"#);
    match v {
        Value::String(ref s) => assert_eq!(s.to_string(), "hello"),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn exec_reports_a_nonzero_exit_code() {
    // `sh -c "exit 7"` exits 7; the code is captured, not turned into an Err.
    assert_eq!(
        eval(r#"unwrap(os.exec(["sh", "-c", "exit 7"])).code"#),
        Value::Integer(7)
    );
}

#[test]
fn exec_returns_err_when_the_program_is_missing() {
    let src = r#"
match os.exec(["this_binary_does_not_exist_37f2"]) {
    Ok(v) => "ran",
    Err(e) => "launch failed"
}
"#;
    match eval(src) {
        Value::String(ref s) => assert_eq!(s.to_string(), "launch failed"),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn exec_rejects_a_non_string_in_the_argument_list() {
    // A non-string in the argument list is misuse, not an environmental
    // failure, so 0.64 raises instead of returning Err — the caller cannot
    // sensibly recover from passing the wrong type.
    let program = Parser::new()
        .parse(r#"os.exec(["echo", 1, 2])"#)
        .expect("parses");
    let err = Interpreter::new()
        .eval_program(program)
        .expect_err("a non-string argument is misuse");
    assert!(
        err.to_string().contains("argv must contain only strings"),
        "got: {err}"
    );
}

#[test]
fn exec_takes_argv_and_refuses_the_program_and_args_form() {
    // the program and its arguments as one list, as proc.spawn takes them
    assert_eq!(
        eval(r#"str.trim(unwrap(os.exec(["sh", "-c", "pwd"], #{ "cwd": "/" })).stdout)"#),
        Value::String(std::sync::Arc::new("/".to_string()))
    );
    for refused in [
        r#"os.exec("echo", ["hi"])"#,
        r#"os.exec("echo", ["hi"], #{})"#,
        r#"os.exec("echo")"#,
        r#"os.exec([])"#,
        r#"os.exec()"#,
    ] {
        let program = Parser::new().parse(refused).expect("parses");
        assert!(
            Interpreter::new().eval_program(program).is_err(),
            "{refused} is refused"
        );
    }
}

#[test]
fn the_checker_counts_argv_as_the_one_required_argument() {
    let program = Parser::new()
        .parse_raw("let a = os.exec([\"ls\"])\nlet b = os.exec([\"ls\"], #{ \"cwd\": \"/\" })\n")
        .unwrap();
    let said: Vec<String> = olang::tools::check::check_program(&program)
        .iter()
        .map(|d| d.message.clone())
        .collect();
    assert!(said.is_empty(), "{said:?}");
}

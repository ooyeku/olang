//! The `proc` module: streaming child processes and pipelines. These
//! drive real subprocesses (POSIX `cat`, `printf`, `grep`, `wc` — the
//! native build targets Unix), so they exercise the whole path: piped
//! stdin/stdout, off-thread draining, exit codes, and pipeline chaining.

use olang::ast::Value;
use olang::{Interpreter, Parser};

fn with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn eval(source: &str) -> Result<Value, String> {
    let source = source.to_string();
    with_big_stack(move || {
        let parser = Parser::new();
        let program = parser.parse(&source).map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        interpreter.eval_program(program).map_err(|e| e.to_string())
    })
}

fn assert_all_true(source: &str, n: usize) {
    let result = eval(source).unwrap();
    assert_eq!(result, Value::List(vec![Value::Boolean(true); n].into()));
}

#[test]
fn streaming_a_filter_through_stdin_and_stdout() {
    // `cat` echoes its stdin: feed two lines, close, read them back, and
    // confirm a clean exit. EOF is reported once stdout closes.
    assert_all_true(
        r#"
let p = unwrap(proc.spawn("cat", []))
proc.write_line(p, "first")
proc.write_line(p, "second")
proc.close_stdin(p)
let a = unwrap(proc.read_line(p))
let b = unwrap(proc.read_line(p))
let eof = match proc.read_line(p) { Ok(_) => false, Err(_) => true }
let exit = unwrap(proc.wait(p))
[ a == "first", b == "second", eof, exit.code == 0 ]
"#,
        4,
    );
}

#[test]
fn spawn_reports_a_nonzero_exit_code() {
    // `grep` with no match exits 1; the code surfaces through wait.
    assert_all_true(
        r#"
let p = unwrap(proc.spawn("grep", ["needle"]))
proc.write_line(p, "haystack")
proc.close_stdin(p)
let eof = match proc.read_line(p) { Ok(_) => false, Err(_) => true }
let exit = unwrap(proc.wait(p))
[ eof, exit.code == 1 ]
"#,
        2,
    );
}

#[test]
fn pipeline_chains_stdout_to_stdin_with_per_stage_codes() {
    // printf three lines | grep 'o' | wc -l  -> 3 (one, two, four all have 'o').
    // Every stage exits 0, and the per-stage codes come back in order.
    assert_all_true(
        r#"
let r = unwrap(proc.pipeline([
    ["printf", "one\ntwo\nfour\n"],
    ["grep", "o"],
    ["wc", "-l"]
]))
[
    str.trim(r.stdout) == "3",
    r.code == 0,
    len(r.codes) == 3,
    r.codes[0] == 0 && r.codes[1] == 0 && r.codes[2] == 0
]
"#,
        4,
    );
}

#[test]
fn pipeline_feeds_initial_stdin_and_reports_a_failing_stage() {
    // opts.stdin feeds the first stage; a middle grep that matches nothing
    // exits 1, so the final `wc -l` sees empty input (0) and the codes
    // record the failure in the middle.
    assert_all_true(
        r#"
let r = unwrap(proc.pipeline(
    [ ["cat"], ["grep", "zzz"], ["wc", "-l"] ],
    #{ "stdin": "alpha\nbeta\n" }
))
[
    str.trim(r.stdout) == "0",
    r.codes[0] == 0,
    r.codes[1] == 1,
    len(r.codes) == 3
]
"#,
        4,
    );
}

#[test]
fn signal_flag_starts_clear_and_resets() {
    // Arming installs the handler and clears the flag; with no Ctrl-C sent,
    // interrupted() stays false, and reset is idempotent. (Delivering a
    // real SIGINT to the test process would abort the whole run, so the
    // graceful-shutdown loop is exercised end-to-end in examples/watch.)
    assert_all_true(
        r#"
unwrap(os.on_interrupt())
let a = os.interrupted()
os.reset_interrupt()
let b = os.interrupted()
[ a == false, b == false ]
"#,
        2,
    );
}

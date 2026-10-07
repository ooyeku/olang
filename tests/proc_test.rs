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
    // graceful-shutdown loop is exercised end-to-end in examples/tools/watch.)
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

/// Is `pid` gone within two seconds? (A killed grandchild is reparented
/// and reaped; give that a moment.)
const GONE: &str = r#"
fn gone(pid) = {
    let mut left = 40
    let mut alive = true
    while alive && left > 0 {
        alive = unwrap(os.exec("kill", ["-0", pid])).code == 0
        if alive => time.sleep(50) else => ()
        left = left - 1
    }
    !alive
}
"#;

#[cfg(unix)]
#[test]
fn a_tree_kill_ends_the_grandchildren_too() {
    // `sh -c "sleep 7 & …; sleep 7"`: the backgrounded sleep is the
    // shell's child, the test's grandchild. A plain kill leaves it; a
    // tree kill of a grouped spawn ends it.
    assert_all_true(
        &format!(
            "{GONE}{}",
            r#"
let p = unwrap(proc.spawn("sh", ["-c", "sleep 7 & echo $!; sleep 7"], #{ "group": true }))
let grandchild = str.trim(unwrap(proc.read_line(p)))
let k = proc.kill(p, #{ "tree": true })
let w = unwrap(proc.wait(p))
[ is_ok(k), w.code == -1, gone(grandchild) ]
"#
        ),
        3,
    );
}

#[cfg(unix)]
#[test]
fn a_tree_kill_needs_a_grouped_spawn() {
    assert_all_true(
        r#"
let p = unwrap(proc.spawn("sleep", ["5"]))
let refused = match proc.kill(p, #{ "tree": true }) { Ok(u) => "", Err(e) => e }
let plain = proc.kill(p)
let w = proc.wait(p)
[ str.contains(refused, "group: true"), is_ok(plain) ]
"#,
        2,
    );
}

#[test]
fn a_missing_cwd_is_named_not_the_program() {
    assert_all_true(
        r#"
let s = match proc.spawn("sh", [], #{ "cwd": "/nonexistent/olang-dir" }) { Ok(p) => "", Err(e) => e }
let pl = match proc.pipeline([["sh"]], #{ "cwd": "/nonexistent/olang-dir" }) { Ok(p) => "", Err(e) => e }
let ex = match os.exec("sh", [], #{ "cwd": "/nonexistent/olang-dir" }) { Ok(p) => "", Err(e) => e }
[ str.contains(s, "cwd '/nonexistent/olang-dir' does not exist"), !str.contains(s, "'sh'"),
  str.contains(pl, "/nonexistent/olang-dir"), str.contains(ex, "cwd '/nonexistent/olang-dir' does not exist") ]
"#,
        4,
    );
}

#[cfg(unix)]
#[test]
fn exec_with_a_timeout_kills_the_tree_and_says_so() {
    assert_all_true(
        &format!(
            "{GONE}{}",
            r#"
let t0 = time.monotonic_ms()
let r = unwrap(os.exec("sh", ["-c", "sleep 7 & echo $!; sleep 7"], #{ "timeout_ms": 300 }))
let took = time.monotonic_ms() - t0
[ r.timed_out, r.code == -1, took >= 290, took < 3000, gone(str.trim(r.stdout)) ]
"#
        ),
        5,
    );
}

#[test]
fn exec_within_its_timeout_answers_as_without_one() {
    assert_all_true(
        r#"
let r = unwrap(os.exec("sh", ["-c", "echo out; echo err >&2; exit 3"], #{ "timeout_ms": 10000 }))
let fed = unwrap(os.exec("cat", [], #{ "timeout_ms": 10000, "stdin": "fed" }))
let plain = unwrap(os.exec("sh", ["-c", "exit 0"]))
[ r.timed_out == false, r.code == 3, str.trim(r.stdout) == "out", str.trim(r.stderr) == "err",
  fed.stdout == "fed", plain.timed_out == false ]
"#,
        6,
    );
}

/// The plain (unframed) path beside the framed one: a grouped child's
/// lines, its exit code, a stream read to its end, a cancel by tree kill —
/// what a framed spawn must not take from (it once took a plain child's
/// stdout before checking `framing`, so no line or exit code arrived).
#[test]
fn a_plain_grouped_child_streams_lines_and_exits_beside_a_framed_one() {
    assert_all_true(
        r#"
let p = unwrap(proc.spawn("sh", ["-c", "echo one; echo two; exit 3"], #{ "group": true }))
let a = unwrap(proc.read_line(p))
let b = unwrap(proc.read_line(p))
let end_ = proc.read_line(p)
let code = unwrap(proc.wait(p)).code
let q = unwrap(proc.spawn("sh", ["-c", "sleep 30"], #{ "group": true }))
let killed = is_ok(proc.kill(q, #{ "tree": true }))
let gone = unwrap(proc.wait(q)).code != 0
let f = unwrap(proc.spawn("cat", [], #{ "framing": "content-length" }))
unwrap(proc.write_frame(f, "{\"a\":1}"))
let framed = unwrap(proc.read_line(f))
proc.close_stdin(f)
let fend = proc.read_line(f)
[a == "one", b == "two", is_err(end_), code == 3, killed, gone, framed == "{\"a\":1}", is_err(fend)]
"#,
        8,
    );
}

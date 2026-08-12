//! The script-enabler set: shebang lines, environment readers, stdin as
//! data, and path helpers — what makes `#!/usr/bin/env olang` scripts
//! first-class citizens.

use olang::Value;
use olang::interpreter::Interpreter;
use olang::parser::Parser;
use std::process::{Command, Stdio};

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

fn olang_bin() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

// ── shebang ────────────────────────────────────────────────────────────

#[test]
fn shebang_line_parses_and_runs() {
    assert_eq!(
        eval("#!/usr/bin/env olang\n1 + 2"),
        Value::Integer(3),
        "a leading #! line is host metadata, not syntax"
    );
    // Shebang with flags, and a file that is only a shebang.
    assert_eq!(
        eval("#!/usr/bin/env -S olang --no-ovm\n42"),
        Value::Integer(42)
    );
    assert_eq!(eval("#!/usr/bin/env olang"), Value::Unit);
}

#[test]
fn shebang_preserves_line_numbers_exactly() {
    // The shebang is masked, not stripped: an error on line 3 of the file
    // reports line 3.
    let program = Parser::new()
        .parse("#!/usr/bin/env olang\nlet a = 1\nlet b: Int = \"nope\"\n")
        .expect("parse");
    let err = Interpreter::new()
        .eval_program(program)
        .expect_err("dishonest annotation");
    let mut interp = Interpreter::new();
    // Re-run to capture the location side channel.
    let program = Parser::new()
        .parse("#!/usr/bin/env olang\nlet a = 1\nlet b: Int = \"nope\"\n")
        .expect("parse");
    let _ = interp.eval_program(program);
    let loc = interp.take_error_location().expect("location captured");
    assert_eq!(loc.line, 3, "shebang must not shift line numbers");
    assert!(err.to_string().contains("expects Int, got String"));
}

#[test]
fn hash_bang_mid_file_is_still_an_error() {
    // Only a LEADING #! is host metadata.
    assert!(
        Parser::new()
            .parse("let a = 1\n#!/usr/bin/env olang\n")
            .is_err()
    );
}

// ── env (pre-existing surface: get_env/set_env/has_env/list_env) ──────

#[test]
fn env_surface_round_trips() {
    let v = eval(
        "os.set_env(\"OLANG_SCRIPT_TEST\", \"v1\")\nunwrap(os.get_env(\"OLANG_SCRIPT_TEST\"))",
    );
    assert_eq!(v, Value::String("v1".to_string().into()));
    assert_eq!(
        eval("unwrap(os.has_env(\"OLANG_DEFINITELY_NOT_SET_ANYWHERE\"))"),
        Value::Boolean(false)
    );
}

// ── stdin ──────────────────────────────────────────────────────────────

fn run_with_stdin(src: &str, stdin: &str) -> String {
    let dir = std::env::temp_dir().join(format!("olang-script-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("stdin_case.ol");
    std::fs::write(&file, src).unwrap();
    let mut child = Command::new(olang_bin())
        .arg(&file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn olang");
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("run olang");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn stdin_reads_all_and_lines_strips_endings() {
    let out = run_with_stdin("println(to_string(len(unwrap(os.stdin()))))", "abc\ndef\n");
    assert_eq!(out.trim(), "8", "stdin() reads every byte to EOF");
    let out = run_with_stdin(
        "let ls = unwrap(os.stdin_lines())\nprintln(to_string(len(ls)) + \" \" + ls[0] + \" \" + ls[2])",
        "alpha\r\nbeta\ngamma\n",
    );
    assert_eq!(out.trim(), "3 alpha gamma", "lines split, endings stripped");
}

// ── the pipe convention ────────────────────────────────────────────────

#[test]
fn closed_stdout_ends_the_program_quietly() {
    // `olang gen.ol | head -1`: when the reader exits, olang must behave
    // like every Unix filter — terminate quietly with 141 (128+SIGPIPE),
    // not panic with "failed printing to stdout".
    let dir = std::env::temp_dir().join(format!("olang-pipe-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("gen.ol");
    std::fs::write(
        &file,
        "let mut i = 0\nwhile i < 1000000 { println(to_string(i))\n i = i + 1 }\n",
    )
    .unwrap();
    let mut child = Command::new(olang_bin())
        .arg(&file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn olang");
    // Read one line, then drop the pipe — the head -1 shape.
    {
        use std::io::BufRead as _;
        let stdout = child.stdout.take().unwrap();
        let mut reader = std::io::BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "0");
    } // reader drops here; the pipe closes
    let out = child.wait_with_output().expect("wait");
    assert_eq!(out.status.code(), Some(141), "SIGPIPE convention");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains("panicked") && !err.contains("failed printing"),
        "quiet exit, no panic: {}",
        err
    );
}

// ── path helpers ───────────────────────────────────────────────────────

#[test]
fn path_helpers_join_and_decompose() {
    assert_eq!(
        eval("fs.join([\"a\", \"b\", \"c.txt\"])"),
        Value::String("a/b/c.txt".to_string().into())
    );
    // An absolute segment restarts the path — standard join semantics.
    assert_eq!(
        eval("fs.join([\"a\", \"/etc\", \"hosts\"])"),
        Value::String("/etc/hosts".to_string().into())
    );
    assert_eq!(
        eval("fs.dirname(\"/x/y/z.ol\")"),
        Value::String("/x/y".to_string().into())
    );
    assert_eq!(
        eval("fs.basename(\"/x/y/z.ol\")"),
        Value::String("z.ol".to_string().into())
    );
    assert_eq!(
        eval("fs.ext(\"/x/y/z.ol\")"),
        Value::String("ol".to_string().into())
    );
    assert_eq!(
        eval("fs.ext(\"/x/y/Makefile\")"),
        Value::String("".to_string().into())
    );
    assert_eq!(
        eval("fs.dirname(\"solo\")"),
        Value::String("".to_string().into())
    );
}

#[test]
fn abs_path_and_home_dir_resolve() {
    // abs_path works for paths that don't exist and resolves dots.
    let v = eval("unwrap(fs.abs_path(\"./definitely/../not-created.txt\"))");
    match v {
        Value::String(s) => {
            assert!(s.starts_with('/'), "absolute: {}", s);
            assert!(!s.contains(".."), "dots resolved: {}", s);
        }
        other => panic!("expected string, got {:?}", other),
    }
    // os.home_dir (pre-existing) agrees with the environment.
    let home = std::env::var("HOME").expect("HOME set in test env");
    assert_eq!(eval("unwrap(os.home_dir())"), Value::String(home.into()));
}

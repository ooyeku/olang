//! Roadmap Tier 3: the `time` module, `fs.walk`/`fs.glob`, `os.exec`
//! options, `str.fmt`, and db transactions.

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
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

// ── time ───────────────────────────────────────────────────────────────

#[test]
fn time_clocks_and_sleep() {
    let src = r#"
let t0 = time.monotonic_ms()
time.sleep(20)
let dt = time.monotonic_ms() - t0
show(time.now_ms() > 1700000000000) + "/" + show(dt >= 15)
"#;
    assert_eq!(s(eval(src)), "true/true");
}

#[test]
fn time_sleep_rejects_negatives() {
    assert!(eval_res("time.sleep(-5)").is_err());
}

// ── fs.walk / fs.glob ──────────────────────────────────────────────────

#[test]
fn walk_lists_files_recursively_and_sorted() {
    // The repo's own docs directory: known, stable, nested-free but real.
    let src = r#"
let files = unwrap(fs.walk("docs"))
show(len(files) > 5) + "/" + show(col.all(files, (f) => str.starts_with(f, "docs/")))
"#;
    assert_eq!(s(eval(src)), "true/true");
}

#[test]
fn glob_matches_star_within_a_segment() {
    let src = r#"
let hits = unwrap(fs.glob("docs/*.md"))
show(contains(hits, "docs/language.md")) + "/" + show(col.all(hits, (f) => str.ends_with(f, ".md")))
"#;
    assert_eq!(s(eval(src)), "true/true");
}

#[test]
fn glob_double_star_spans_segments() {
    let src = r#"
let hits = unwrap(fs.glob("examples/**/main.ol"))
show(len(hits) > 5) + "/" + show(contains(hits, "examples/language/regex/main.ol"))
"#;
    assert_eq!(s(eval(src)), "true/true");
}

// ── os.exec options ────────────────────────────────────────────────────

#[test]
fn exec_cwd_option_sets_the_working_directory() {
    let src = r#"
let r = unwrap(os.exec("pwd", [], #{ "cwd": "/" }))
str.trim(r.stdout)
"#;
    assert_eq!(s(eval(src)), "/");
}

#[test]
fn exec_stdin_and_env_options() {
    let src = r#"
let r = unwrap(os.exec("sh", ["-c", "cat; printf %s \"$T3\""],
    #{ "stdin": "in:", "env": #{ "T3": "env" } }))
r.stdout
"#;
    assert_eq!(s(eval(src)), "in:env");
}

#[test]
fn exec_two_argument_form_is_unchanged() {
    let src = r#"str.trim(unwrap(os.exec("echo", ["plain"])).stdout)"#;
    assert_eq!(s(eval(src)), "plain");
}

#[test]
fn exec_rejects_unknown_options() {
    // A mistyped option key is misuse: 0.64 raises, so the typo cannot be
    // swallowed by an `unwrap_or` and silently ignored.
    let program = olang::Parser::new()
        .parse(r#"os.exec("echo", [], #{ "typo": 1 })"#)
        .expect("parses");
    let err = olang::Interpreter::new()
        .eval_program(program)
        .expect_err("an unknown option is misuse");
    assert!(
        err.to_string().contains("unknown or mistyped option"),
        "got: {err}"
    );
}

// ── str.fmt ────────────────────────────────────────────────────────────

#[test]
fn fmt_fills_placeholders_in_display_form() {
    let src = r#"str.fmt("{} of {} ({})", 3, "hearts", true)"#;
    assert_eq!(s(eval(src)), "3 of hearts (true)");
}

#[test]
fn fmt_escapes_literal_braces() {
    assert_eq!(s(eval(r#"str.fmt("{{}} and {}", 1)"#)), "{} and 1");
}

#[test]
fn fmt_arity_mismatches_error() {
    assert!(eval_res(r#"str.fmt("{} {}", 1)"#).is_err());
    assert!(eval_res(r#"str.fmt("{}", 1, 2)"#).is_err());
}

// ── db transactions ────────────────────────────────────────────────────

#[test]
fn rollback_discards_and_commit_keeps() {
    let src = r#"
let conn = unwrap(db.open(":memory:"))
unwrap(db.execute(conn, "CREATE TABLE t (n INTEGER)"))
unwrap(db.begin(conn))
unwrap(db.execute(conn, "INSERT INTO t VALUES (1)"))
unwrap(db.rollback(conn))
unwrap(db.begin(conn))
unwrap(db.execute(conn, "INSERT INTO t VALUES (2)"))
unwrap(db.commit(conn))
let rows = unwrap(db.query(conn, "SELECT n FROM t"))
show(len(rows)) + "/" + show(map_get(rows[0], "n"))
"#;
    assert_eq!(s(eval(src)), "1/2");
}

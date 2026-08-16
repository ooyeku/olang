//! The error model settled in 0.65 (roadmap lane S7).
//!
//! `Result` with `?` is how expected failure travels. A runtime error is a
//! bug and stops the program — there is no construct that catches one
//! mid-expression, and `try`/`catch` (which never did, despite looking
//! like it) is gone.
//!
//! Recovery is *structural*: a spawned task and an `http.serve` handler
//! are already isolated, so a failure inside one becomes a value on the
//! far side without anything at the failure site.

use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn run(source: &str) -> Result<String, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    Interpreter::new()
        .eval_program(program)
        .map(|v| match v {
            olang::ast::Value::String(s) => s.to_string(),
            other => other.to_string(),
        })
        .map_err(|e| e.to_string())
}

fn err(source: &str) -> String {
    match run(source) {
        Ok(v) => panic!("expected an error, got {v}"),
        Err(e) => e,
    }
}

// ── try/catch is gone ─────────────────────────────────────────────────

#[test]
fn try_catch_gives_a_migration_error() {
    // A removed construct that simply falls out of the grammar produces
    // "expected a statement", which tells a reader nothing. This must name
    // the two forms that replace it.
    let e = err("let r = try { Err(\"x\") } catch (e) { 0 }\n");
    assert!(e.contains("removed in 0.65"), "{e}");
    assert!(e.contains("match"), "{e}");
    assert!(e.contains("unwrap_or"), "{e}");
}

#[test]
fn match_and_unwrap_or_say_what_catch_said() {
    // The two forms the migration error names, on the same input.
    assert_eq!(
        run(
            "fn risky(n) = if n > 0 => Ok(n * 2) else => Err(\"negative\")\n\
             to_string(match risky(21) { Ok(v) => v, Err(e) => 0 })\n"
        )
        .unwrap(),
        "42"
    );
    assert_eq!(
        run(
            "fn risky(n) = if n > 0 => Ok(n * 2) else => Err(\"negative\")\n\
             to_string(unwrap_or(risky(-1), 0))\n"
        )
        .unwrap(),
        "0"
    );
}

// ── a runtime error is not recoverable in place ───────────────────────

#[test]
fn a_runtime_error_stops_the_program() {
    // Deliberate: these are bugs, and 0.61-0.64 spent four releases making
    // bugs stop rather than degrade. There is no in-place recovery.
    let e = err("fn f() = 1 + \"not a number\"\nf()\n");
    assert!(e.contains("cannot add"), "{e}");
}

// ── recovery is structural ────────────────────────────────────────────

#[test]
fn a_task_boundary_turns_a_runtime_error_into_a_value() {
    let out = run(
        "fn risky(n) = if n > 2 => 1 + \"not a number\" else => n * 10\n\
                   let jobs = [spawn risky(1), spawn risky(9)]\n\
                   to_string(jobs |> map((j) => match task.join(j) { Err(e) => -1, v => v }))\n",
    )
    .unwrap();
    assert_eq!(out, "[10, -1]");
}

#[test]
fn one_failing_task_does_not_take_the_program_with_it() {
    let out = run("fn boom() = 1 + \"x\"\n\
                   let t = spawn boom()\n\
                   let r = match task.join(t) { Err(e) => \"handled\", v => \"?\" }\n\
                   r + \" and still running\"\n")
    .unwrap();
    assert_eq!(out, "handled and still running");
}

// ── the discarded-Result warning ──────────────────────────────────────

fn warnings(source: &str) -> Vec<String> {
    let program = Parser::new().parse(source).expect("parses");
    olang::tools::check::check_program(&program)
        .into_iter()
        .filter(|d| d.warning)
        .map(|d| d.message)
        .collect()
}

#[test]
fn a_discarded_result_is_flagged() {
    // The hole this closes: the write failed, the program carried on, and
    // nothing anywhere said so.
    let w = warnings("fs.write_file(\"/nope/x.txt\", \"data\")\nprintln(\"done\")\n");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("fs.write_file"), "{}", w[0]);
    assert!(w[0].contains("discarded"), "{}", w[0]);
}

#[test]
fn handling_the_result_any_of_the_four_ways_is_clean() {
    for source in [
        "let r = fs.write_file(\"/tmp/x\", \"d\")\nprintln(show(r))\n",
        "let _ = fs.write_file(\"/tmp/x\", \"d\")\n",
        "unwrap(fs.write_file(\"/tmp/x\", \"d\"))\n",
        "match fs.write_file(\"/tmp/x\", \"d\") { Ok(v) => 1, Err(e) => 0 }\n",
    ] {
        assert!(
            warnings(source).is_empty(),
            "{source}: {:?}",
            warnings(source)
        );
    }
}

#[test]
fn a_tail_expression_is_a_return_not_a_discard() {
    // The false positive this rule exists to avoid: a function whose body
    // *is* the fallible call returns the Result perfectly well.
    let w = warnings(
        "fn parse_body(b) = {\n    let text = if len(b) == 0 => \"{}\" else => b\n    json.parse(text)\n}\nprintln(show(parse_body(\"{}\")))\n",
    );
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn an_infallible_call_is_never_flagged() {
    // 0.64 made these return values, so there is no Result to discard.
    // This also pins that the help registry (the lint's source of truth)
    // agrees with what the functions actually return.
    for source in ["os.args()\n", "os.arch()\n", "fs.exists(\"/tmp\")\n"] {
        assert!(
            warnings(source).is_empty(),
            "{source}: {:?}",
            warnings(source)
        );
    }
}

#[test]
fn the_warning_is_advisory_and_does_not_gate() {
    // Unlike the scope and mutability rules, this is a judgement about
    // intent rather than a provable contradiction, so it must not fail a
    // build. A script that genuinely does not care still runs.
    let program = Parser::new()
        .parse("fs.write_file(\"/nope/x.txt\", \"data\")\nprintln(\"done\")\n")
        .expect("parses");
    let diagnostics = olang::tools::check::check_program(&program);
    assert!(diagnostics.iter().all(|d| d.warning), "must not gate");
}

//! Errors that teach (roadmap lane W3), plus the assert builtins that
//! closed W5's straggler.
//!
//! Each test pins a message a person actually hit: the words matter, so
//! the assertions are on message content, not just on failure.

use olang::interpreter::{Interpreter, IntuitiveErrorFormatter};
use olang::parser::Parser;

fn run(source: &str) -> Result<String, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    Interpreter::new()
        .eval_program(program)
        .map(|v| v.to_string())
        .map_err(|e| e.to_string())
}

fn err(source: &str) -> String {
    match run(source) {
        Ok(v) => panic!("expected an error, got {v}"),
        Err(e) => e,
    }
}

fn suggestion_text(source: &str) -> String {
    Parser::new()
        .suggestions_for(source)
        .into_iter()
        .map(|s| {
            format!(
                "{} | {} | {}",
                s.message,
                s.fix.unwrap_or_default(),
                s.help.unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── W5: asserts are expressions too ───────────────────────────────────

#[test]
fn assert_eq_works_in_a_match_arm() {
    // The grammar rewrites asserts only in statement position; expression
    // positions (match arms, lambda bodies) reach the builtins instead.
    // Both must raise identically.
    assert_eq!(
        run("match Ok(1) { Ok(v) => assert_eq(v, 1), Err(e) => () }").unwrap(),
        "()"
    );
    let e = err("match Ok(1) { Ok(v) => assert_eq(v, 999), Err(e) => () }");
    assert!(e.contains("Assertion failed"), "got: {e}");
}

#[test]
fn all_five_asserts_resolve_as_expressions() {
    for call in [
        "assert_eq(1, 1)",
        "assert_ne(1, 2)",
        "assert(true)",
        "assert_true(true)",
        "assert_false(false)",
    ] {
        let src = format!("let f = () => {call}\nf()");
        assert!(run(&src).is_ok(), "{call} failed as an expression");
    }
    let e = err("let f = () => assert_false(true, \"still on\")\nf()");
    assert!(e.contains("still on"), "custom message lost: {e}");
}

// ── W3: calling a non-function names the value and its type ───────────

#[test]
fn calling_an_int_names_the_binding_and_type() {
    let e = err("let x = 5\nx(1)");
    assert!(e.contains("'x' is an Int, not a function"), "got: {e}");
    assert!(e.contains("shadows it"), "shadow hint missing: {e}");
}

#[test]
fn calling_a_map_suggests_indexing() {
    let e = err("let m = #{\"a\": 1}\nm(\"a\")");
    assert!(e.contains("'m' is a Map, not a function"), "got: {e}");
    assert!(e.contains("m[...]"), "indexing hint missing: {e}");
}

#[test]
fn calling_a_string_uses_the_right_article() {
    let e = err("let s = \"hi\"\ns(2)");
    assert!(e.contains("'s' is a String, not a function"), "got: {e}");
}

// ── W3: module not found teaches the shelf ────────────────────────────

#[test]
fn module_not_found_help_names_the_shelf_workflow() {
    let error = olang::interpreter::InterpreterError::ModuleNotFound {
        module_path: "geomtry".to_string(),
        searched_paths: vec!["Standard library modules".to_string()],
        available_modules: vec!["geometry".to_string()],
        suggestions: vec!["geometry".to_string()],
    };
    let text = IntuitiveErrorFormatter::default().format_error(&error);
    assert!(text.contains("Did you mean"), "got: {text}");
    assert!(
        text.contains("otc lib list"),
        "shelf guidance missing: {text}"
    );
    assert!(
        text.contains("use lib.<name>"),
        "lib/ guidance missing: {text}"
    );
}

// ── W3: C-style braces after `if` name the arrow form ─────────────────

#[test]
fn brace_after_if_suggests_the_arrow() {
    let text = suggestion_text("let x = 5\nif x > 1 {\n    println(\"big\")\n}\n");
    assert!(
        text.contains("`if` takes an arrow before its branch"),
        "got: {text}"
    );
    assert!(
        text.contains("if x > 1 => {"),
        "concrete fix missing: {text}"
    );
}

#[test]
fn while_keeps_its_braces_without_complaint() {
    // Only `if`/`else` want the arrow; a legal while-loop parses clean.
    assert_eq!(
        run("let mut i = 0\nwhile i < 3 {\n    i = i + 1\n}\ni").unwrap(),
        "3"
    );
}

// ── W3: unclosed delimiters name the opener ───────────────────────────

#[test]
fn unclosed_paren_reports_where_it_opened() {
    let text = suggestion_text("fn f(x) = {\n    let y = (x + 1\n    y * 2\n}\nf(3)\n");
    assert!(
        text.contains("Unclosed `(` opened at line 2, column 13"),
        "got: {text}"
    );
}

#[test]
fn unclosed_bracket_reports_where_it_opened() {
    let text = suggestion_text("let xs = [1, 2, 3\nprintln(xs)\n");
    assert!(
        text.contains("Unclosed `[` opened at line 1, column 10"),
        "got: {text}"
    );
}

#[test]
fn brackets_inside_strings_do_not_count_as_openers() {
    let text = suggestion_text("let s = \"a ( in a string\"\nlet t = ) oops\n");
    assert!(!text.contains("Unclosed"), "false positive: {text}");
}

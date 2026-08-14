//! The embedded `term` package: terminal styling and structure. The
//! pure functions (color codes, style, rule, table, bar) are testable
//! natively; styling is gated on `CLICOLOR_FORCE` for exactly this
//! reason (a test's stdout is a pipe, not a TTY).

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
fn styling_gates_on_env() {
    // Force-on: color functions wrap with the right SGR codes and reset.
    // Then NO_COLOR wins even when forced-adjacent — and with neither
    // set, a piped test stdout yields plain text.
    assert_all_true(
        r##"use term
os.set_env("CLICOLOR_FORCE", "1")
os.remove_env("NO_COLOR")
let r = term.red("x")
let s = term.style("y", #{ "fg": "yellow", "bg": "blue", "bold": true })
let on = [
    str.contains(r, "\x1b[31m") && str.contains(r, "x") && str.contains(r, "\x1b[0m"),
    str.contains(s, "\x1b[33;44;1m"),
    term.color() == true
]
os.remove_env("CLICOLOR_FORCE")
// No force, not a TTY (piped) → styling off, plain passthrough.
let off = [ term.red("x") == "x", term.color() == false ]
concat(on, off)"##,
        5,
    );
}

#[test]
fn structure_rule_table_bar() {
    assert_all_true(
        r##"use term
os.remove_env("CLICOLOR_FORCE")
let t = term.table(["name", "n"], [["ada", "128"], ["evelyn", "1"]])
let lines = str.lines(t)
[
    str.length(term.rule(10)) == str.length("──────────"),
    // header line pads "name" to the width of "evelyn" (6) + gap.
    str.starts_with(lines[0], "name  "),
    str.contains(lines[1], "ada   ") && str.contains(lines[1], "128"),
    // bar: 0.5 of 20 → 10 filled blocks, 10 empty, and a percentage.
    str.contains(term.bar(0.5, 20), "50%"),
    str.contains(term.bar(1.0, 10), "100%")
]"##,
        5,
    );
}

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
    // Then, with neither set, a piped test stdout yields plain text —
    // and the structure helpers render correctly with color off.
    //
    // Everything runs in ONE program (one thread), sequentially: styling
    // gates on process-global env (`CLICOLOR_FORCE`/`NO_COLOR`), which a
    // second parallel test mutating the same vars would race against.
    assert_all_true(
        r##"use term
os.set_env("CLICOLOR_FORCE", "1")
os.remove_env("NO_COLOR")
let r = term.red("x")
let s = term.style("y", #{ "fg": "yellow", "bg": "blue", "bold": true })
os.remove_env("CLICOLOR_FORCE")
// Color off now (piped, not a TTY): plain passthrough, and the
// structure helpers produce clean aligned output.
let plain_red = term.red("x")
let colored_off = term.color()
let t = term.table(["name", "n"], [["ada", "128"], ["evelyn", "1"]])
let lines = str.lines(t)
[
    str.contains(r, "\x1b[31m") && str.contains(r, "x") && str.contains(r, "\x1b[0m"),
    str.contains(s, "\x1b[33;44;1m"),
    plain_red == "x",
    colored_off == false,
    str.length(term.rule(10)) == str.length("──────────"),
    str.starts_with(lines[0], "name  "),
    str.contains(lines[1], "ada   ") && str.contains(lines[1], "128"),
    str.contains(term.bar(0.5, 20), "50%"),
    str.contains(term.bar(1.0, 10), "100%")
]"##,
        9,
    );
}

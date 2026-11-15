//! The embedded `cli` package: declarative argument parsing. Pure logic
//! over an argv list, so it's fully testable natively — the browser and
//! the terminal only change where the argv comes from.

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

// A spec reused across cases: a bool flag, a typed flag with a default,
// and one required positional.
const SPEC: &str = r#"
let spec = #{
    "name": "greet", "about": "Greet someone",
    "flags": [
        #{ "name": "loud", "short": "l", "type": "bool" },
        #{ "name": "count", "short": "n", "type": "int", "default": 1 }
    ],
    "args": [ #{ "name": "who", "required": true } ]
}
"#;

#[test]
fn flags_positionals_and_defaults() {
    assert_all_true(
        &format!(
            r#"use cli
{SPEC}
let a = unwrap(cli.parse(spec, ["--loud", "-n", "3", "world"]))
let b = unwrap(cli.parse(spec, ["ada"]))
let c = unwrap(cli.parse(spec, ["--count=5", "eve"]))
[
    map_get(a, "loud") == true && map_get(a, "count") == 3 && map_get(a, "who") == "world",
    map_get(b, "loud") == false && map_get(b, "count") == 1 && map_get(b, "who") == "ada",
    map_get(c, "count") == 5 && map_get(c, "who") == "eve"
]"#
        ),
        3,
    );
}

#[test]
fn errors_are_precise() {
    assert_all_true(
        &format!(
            r#"use cli
{SPEC}
fn msg(r) = match r {{ Err(e) => e, Ok(a) => "OK" }}
[
    str.contains(msg(cli.parse(spec, ["--bogus", "x"])), "unknown flag: --bogus"),
    str.contains(msg(cli.parse(spec, ["-n", "notnum", "x"])), "expects an integer"),
    str.contains(msg(cli.parse(spec, [])), "missing required argument: <who>"),
    str.contains(msg(cli.parse(spec, ["--count"])), "needs a value")
]"#
        ),
        4,
    );
}

#[test]
fn help_short_circuits_and_renders() {
    assert_all_true(
        &format!(
            r#"use cli
{SPEC}
let h = unwrap(cli.parse(spec, ["-h"]))
let text = cli.help(spec)
[
    map_get(h, "help") == true,
    str.contains(text, "Usage: greet [options] <who>"),
    str.contains(text, "-n, --count") && str.contains(text, "(default: 1)"),
    str.contains(text, "-h, --help")
]"#
        ),
        4,
    );
}

#[test]
fn subcommands_dispatch() {
    assert_all_true(
        r#"use cli
let spec = #{
    "name": "notes",
    "commands": [
        #{ "name": "add", "args": [ #{ "name": "text", "required": true } ] },
        #{ "name": "list", "flags": [ #{ "name": "all", "short": "a", "type": "bool" } ] }
    ]
}
let add = unwrap(cli.parse(spec, ["add", "buy milk"]))
let list = unwrap(cli.parse(spec, ["list", "-a"]))
fn msg(r) = match r { Err(e) => e, Ok(a) => "OK" }
[
    map_get(add, "command") == "add" && map_get(add, "text") == "buy milk",
    map_get(list, "command") == "list" && map_get(list, "all") == true,
    str.contains(msg(cli.parse(spec, ["nope"])), "unknown command: nope"),
    map_get(unwrap(cli.parse(spec, [])), "help") == true
]"#,
        4,
    );
}

#[test]
fn env_fallback_fills_absent_flags() {
    // A flag with no default takes its value from the named env var when
    // the flag is absent; otherwise the key stays unset.
    assert_all_true(
        r#"use cli
let spec = #{ "flags": [ #{ "name": "token", "env": "OLANG_CLI_TEST_TOKEN" } ] }
let missing = unwrap(cli.parse(spec, []))
[ map_has_key(missing, "token") == false ]"#,
        1,
    );
}

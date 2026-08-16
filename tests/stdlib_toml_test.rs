//! `toml` — parse/stringify/validate, bridged through the same serde
//! conversions as `json` so value shapes cannot drift between the two.

use olang::Value;
use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn eval(src: &str) -> Value {
    let program = Parser::new().parse(src).expect("parse");
    Interpreter::new().eval_program(program).expect("eval")
}

#[test]
fn parse_produces_maps_lists_and_scalars() {
    let v = eval(
        "let d = unwrap(toml.parse(\"name = \\\"cfg\\\"\\nports = [1, 2]\\n[server]\\nhost = \\\"h\\\"\\ndebug = true\\nratio = 0.5\"))\nshow(map_get(d, \"name\")) + \" \" + show(map_get(d, \"ports\")) + \" \" + show(map_get(map_get(d, \"server\"), \"debug\")) + \" \" + show(map_get(map_get(d, \"server\"), \"ratio\"))",
    );
    assert_eq!(v, Value::String("cfg [1, 2] true 0.5".to_string().into()));
}

#[test]
fn stringify_round_trips_and_rejects_non_tables() {
    let v = eval(
        "let out = unwrap(toml.stringify(#{ \"server\": #{ \"port\": 7317 }, \"name\": \"x\" }))\nshow(map_get(map_get(unwrap(toml.parse(out)), \"server\"), \"port\"))",
    );
    assert_eq!(v, Value::String("7317".to_string().into()));
    // A non-table argument is misuse, so 0.64 raises rather than
    // returning an Err the caller might unwrap_or past.
    for bad in ["toml.stringify([1, 2])", "toml.stringify(42)"] {
        let program = Parser::new().parse(bad).expect("parses");
        let err = Interpreter::new()
            .eval_program(program)
            .expect_err("a non-table is misuse");
        assert!(
            err.to_string().contains("a TOML document is a table"),
            "{err}"
        );
    }
}

#[test]
fn validate_answers_without_erroring() {
    assert_eq!(eval("toml.validate(\"a = 1\")"), Value::Boolean(true));
    assert_eq!(
        eval("toml.validate(\"not [ valid\")"),
        Value::Boolean(false)
    );
    // Malformed input is a clean Err from parse, never a crash.
    assert_eq!(
        eval("is_err(toml.parse(\"= broken\"))"),
        Value::Boolean(true)
    );
}

#[test]
fn datetimes_are_plain_strings() {
    // The toml crate wraps datetimes in a private one-key object over
    // serde; the module unwraps them to the string the docs promise.
    let v = eval("show(map_get(unwrap(toml.parse(\"when = 2026-08-12T10:00:00Z\")), \"when\"))");
    assert_eq!(v, Value::String("2026-08-12T10:00:00Z".to_string().into()));
}

#[test]
fn parses_a_real_package_manifest() {
    // The repo's own manifest format is the first customer.
    let v = eval(
        "let d = unwrap(toml.parse(unwrap(fs.read_file(\"examples/packages/demo/olang.toml\"))))\nshow(map_get(map_get(d, \"package\"), \"name\"))",
    );
    assert_eq!(v, Value::String("demo".to_string().into()));
}

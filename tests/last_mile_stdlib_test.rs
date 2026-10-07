//! The 0.68 "last-mile" stdlib surface: the `Bytes` value kind, the
//! first-class `Date` value, and the HTTP client's options map.
//! (fix.md item 5; items 1–4 and 6 are covered by the loop/append,
//! precedence, effects, and conventions suites.)

use olang::interpreter::Interpreter;
use olang::parser::Parser;

fn run(source: &str) -> Result<String, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    Interpreter::new()
        .eval_program(program)
        .map(|v| match v {
            olang::ast::Value::String(ref s) => s.to_string(),
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

// ── Bytes ─────────────────────────────────────────────────────────────

#[test]
fn bytes_construct_inspect_and_convert() {
    assert_eq!(run("typeof(bytes.from_list([1, 2]))").unwrap(), "Bytes");
    assert_eq!(run("show(len(bytes.from_list([1, 2, 3])))").unwrap(), "3");
    assert_eq!(run("show(bytes.from_list([10, 20])[0])").unwrap(), "10");
    // Negative indices count from the end, like lists.
    assert_eq!(run("show(bytes.from_list([10, 20])[-1])").unwrap(), "20");
    assert_eq!(
        run("unwrap(bytes.to_string(bytes.from_string(\"hi\")))").unwrap(),
        "hi"
    );
    assert_eq!(
        run("show(bytes.to_list(bytes.slice(bytes.from_list([1,2,3,4]), 1, 3)))").unwrap(),
        "[2, 3]"
    );
    assert_eq!(
        run("show(len(bytes.concat(bytes.from_list([1]), bytes.from_list([2, 3]))))").unwrap(),
        "3"
    );
}

#[test]
fn bytes_equality_is_structural() {
    assert_eq!(
        run("show(bytes.from_list([1, 2]) == bytes.from_list([1, 2]))").unwrap(),
        "true"
    );
    assert_eq!(
        run("show(bytes.from_list([1, 2]) == bytes.from_list([2, 1]))").unwrap(),
        "false"
    );
}

#[test]
fn bytes_misuse_raises() {
    // Rule 3: a wrongly-made call is a bug and aborts.
    let e = err("bytes.from_list([1, 300])");
    assert!(e.contains("outside a byte's range"), "{e}");
    let e = err("bytes.from_list([\"a\"])");
    assert!(e.contains("must be an integer"), "{e}");
    let e = err("bytes.to_list(\"not bytes\")");
    assert!(e.contains("expected Bytes"), "{e}");
}

#[test]
fn bytes_to_string_is_a_result_not_a_raise() {
    // Rule 2: whether bytes are valid UTF-8 is a property of the data.
    assert_eq!(
        run("show(is_err(bytes.to_string(bytes.from_list([255]))))").unwrap(),
        "true"
    );
}

#[test]
fn base64_and_crypto_accept_bytes() {
    assert_eq!(
        run("base64.encode(bytes.from_string(\"hi\"))").unwrap(),
        "aGk="
    );
    assert_eq!(
        run("show(bytes.to_list(unwrap(base64.decode_bytes(\"AAH/\"))))").unwrap(),
        "[0, 1, 255]"
    );
    assert_eq!(
        run("show(crypto.sha256(bytes.from_string(\"x\")) == crypto.sha256(\"x\"))").unwrap(),
        "true"
    );
}

// ── Date ──────────────────────────────────────────────────────────────

#[test]
fn date_is_a_first_class_value() {
    assert_eq!(
        run("typeof(unwrap(dates.date(2026, 8, 7)))").unwrap(),
        "Date"
    );
    assert_eq!(
        run("show(unwrap(dates.date(2026, 8, 7)))").unwrap(),
        "2026-08-07"
    );
    assert_eq!(
        run("typeof(unwrap(dates.parse(\"2026-08-07\")))").unwrap(),
        "Date"
    );
}

#[test]
fn date_operators_work() {
    let base = "let d = unwrap(dates.date(2026, 8, 7))\n";
    assert_eq!(run(&format!("{base}show(d + 30)")).unwrap(), "2026-09-06");
    assert_eq!(run(&format!("{base}show(d - 7)")).unwrap(), "2026-07-31");
    assert_eq!(run(&format!("{base}show((d + 30) - d)")).unwrap(), "30");
    assert_eq!(run(&format!("{base}show(d < d + 1)")).unwrap(), "true");
    assert_eq!(
        run(&format!(
            "{base}show(d == unwrap(dates.parse(\"2026-08-07\")))"
        ))
        .unwrap(),
        "true"
    );
}

#[test]
fn date_functions_answer_in_kind() {
    // Date in → Date out; string in → string out (pre-0.68 behavior kept).
    assert_eq!(
        run("typeof(unwrap(dates.add_days(unwrap(dates.date(2026, 1, 1)), 1)))").unwrap(),
        "Date"
    );
    assert_eq!(
        run("show(unwrap(dates.add_days(\"2026-01-01\", 1)))").unwrap(),
        "2026-01-02"
    );
    assert_eq!(
        run("show(unwrap(dates.year(unwrap(dates.date(2026, 1, 1)))))").unwrap(),
        "2026"
    );
}

#[test]
fn day_numbers_and_the_calendars_edges() {
    // a day number is days since 1970-01-01, both ways, either side of it
    assert_eq!(
        run("show(unwrap(dates.epoch_day(\"1970-01-02\")))").unwrap(),
        "1"
    );
    assert_eq!(
        run("show(unwrap(dates.epoch_day(\"1969-12-31\")))").unwrap(),
        "-1"
    );
    assert_eq!(
        run("show(unwrap(dates.from_epoch_day(-1)))").unwrap(),
        "1969-12-31"
    );
    assert_eq!(
        run("typeof(unwrap(dates.from_epoch_day(0)))").unwrap(),
        "Date"
    );
    assert_eq!(
        run(
            "let n = unwrap(dates.epoch_day(\"2024-02-29\"))\nshow(unwrap(dates.from_epoch_day(n)))"
        )
        .unwrap(),
        "2024-02-29"
    );
    // a stamp's day is the day it is written in, whatever its offset
    assert_eq!(
        run("show(unwrap(dates.epoch_day(\"2026-10-04T23:30:00-05:00\")) == unwrap(dates.epoch_day(\"2026-10-04\")))").unwrap(),
        "true"
    );
    assert_eq!(
        run("show(unwrap(dates.epoch_day(unwrap(dates.date(1970, 1, 11)))))").unwrap(),
        "10"
    );
    // weeks start on Monday; answers in kind
    assert_eq!(
        run("show(unwrap(dates.start_of_week(\"2026-10-08\")))").unwrap(),
        "2026-10-05"
    );
    assert_eq!(
        run("show(unwrap(dates.start_of_week(\"2026-10-05\")))").unwrap(),
        "2026-10-05"
    );
    assert_eq!(
        run("show(unwrap(dates.start_of_week(\"2026-10-11\")))").unwrap(),
        "2026-10-05"
    );
    assert_eq!(
        run("typeof(unwrap(dates.start_of_week(unwrap(dates.date(2026, 10, 8)))))").unwrap(),
        "Date"
    );
    assert_eq!(
        run("show(unwrap(dates.start_of_month(\"2024-02-29\")))").unwrap(),
        "2024-02-01"
    );
    // data failures are values; misuse raises
    assert_eq!(
        run("show(is_err(dates.epoch_day(\"soon\")))").unwrap(),
        "true"
    );
    assert_eq!(
        run("show(is_err(dates.from_epoch_day(9223372036854775807)))").unwrap(),
        "true"
    );
    let e = err("dates.from_epoch_day(\"3\")");
    assert!(
        e.contains("dates.from_epoch_day") && e.contains("must be an integer"),
        "{e}"
    );
}

#[test]
fn timestamp_keeps_the_offset_and_takes_a_day() {
    assert_eq!(
        run("show(unwrap(dates.timestamp(\"1970-01-02T00:00:00+00:00\")))").unwrap(),
        "86400"
    );
    assert_eq!(
        run("show(unwrap(dates.timestamp(\"2026-10-04T20:18:08.507558+00:00\")) == unwrap(dates.timestamp(\"2026-10-04T15:18:08.507505-05:00\")))").unwrap(),
        "true"
    );
    assert_eq!(
        run("show(unwrap(dates.timestamp(\"2024-06-15T14:30:00Z\")))").unwrap(),
        "1718461800"
    );
    // without an offset, UTC (as before)
    assert_eq!(
        run("show(unwrap(dates.timestamp(\"2024-06-15T14:30:00\")))").unwrap(),
        "1718461800"
    );
    // a bare day, as text or a Date, is its midnight UTC
    assert_eq!(
        run("show(unwrap(dates.timestamp(\"1970-01-03\")))").unwrap(),
        "172800"
    );
    assert_eq!(
        run("show(unwrap(dates.timestamp(unwrap(dates.date(1970, 1, 3)))))").unwrap(),
        "172800"
    );
    // now, in any zone, is now
    assert_eq!(
        run("show(math.abs(unwrap(dates.timestamp(dates.now())) - unwrap(dates.timestamp(dates.utc_now()))) < 5)").unwrap(),
        "true"
    );
}

#[test]
fn date_string_parsing_is_uniformly_flexible() {
    // Before 0.68 `add_days` refused what `year` accepted: the strict
    // %Y-%m-%d path versus the flexible one. dates.now() output must be
    // usable everywhere a date string is.
    assert_eq!(
        run("show(is_ok(dates.add_days(dates.now(), 1)))").unwrap(),
        "true"
    );
    assert_eq!(run("show(is_ok(dates.year(dates.now())))").unwrap(), "true");
}

#[test]
fn dates_misuse_raises_instead_of_returning_err() {
    // The module violated stdlib rule 3 wholesale before 0.68: a typo'd
    // call came back as Err and `unwrap_or` swallowed it.
    let e = err("unwrap_or(dates.add_days(5, 1), \"fallback\")");
    assert!(
        e.contains("dates.add_days") && e.contains("must be a Date or a date string"),
        "{e}"
    );
    let e = err("dates.year()");
    assert!(e.contains("expects 1 argument"), "{e}");
    // A *data* failure is still a value, not a raise.
    assert_eq!(
        run("show(is_err(dates.parse(\"not a date\")))").unwrap(),
        "true"
    );
    assert_eq!(
        run("show(is_err(dates.date(2026, 13, 1)))").unwrap(),
        "true"
    );
}

// ── HTTP client options ───────────────────────────────────────────────

#[test]
fn http_option_misuse_raises_before_any_network_io() {
    // Options are validated before a request is attempted, so these are
    // testable offline — and an unknown key must raise, never be ignored
    // (a typo'd option would otherwise become a request that quietly
    // lacked its auth header).
    let e = err("http.get(\"http://localhost:1\", #{ \"bogus\": 1 })");
    assert!(e.contains("unknown option"), "{e}");
    let e = err("http.get(\"http://localhost:1\", #{ \"timeout_ms\": \"soon\" })");
    assert!(e.contains("timeout_ms"), "{e}");
    let e = err("http.get(\"http://localhost:1\", #{ \"basic\": \"user\" })");
    assert!(e.contains("basic"), "{e}");
    let e = err("http.get(\"http://localhost:1\", #{ \"bytes\": \"yes\" })");
    assert!(e.contains("\"bytes\" must be true or false"), "{e}");
    // a valid one passes validation: the request itself fails, as a value
    assert_eq!(
        run("show(is_err(http.get(\"http://127.0.0.1:1/x\", #{ \"bytes\": true, \"timeout_ms\": 500 })))").unwrap(),
        "true"
    );
    let e = err("http.get(\"http://localhost:1\", 42)");
    assert!(e.contains("options must be a map"), "{e}");
}

#[test]
fn http_network_failure_is_still_a_value() {
    // Valid options, unreachable host: Err(..), not a raise.
    assert_eq!(
        run("show(is_err(http.get(\"http://127.0.0.1:9\", #{ \"timeout_ms\": 200 })))").unwrap(),
        "true"
    );
}

//! Coverage for the `str` and `re` modules, and the stdlib return-type
//! convention: total operations return bare values, fallible operations
//! return an olang `Result`.

use olang::{Interpreter, Parser, Value};

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

fn s(v: Value) -> String {
    match v {
        Value::String(s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

// ── str: total operations return bare values ────────────────────────

#[test]
fn str_case_and_trim() {
    assert_eq!(s(eval(r#"str.to_upper("abc")"#)), "ABC");
    assert_eq!(s(eval(r#"str.to_lower("ABC")"#)), "abc");
    assert_eq!(s(eval(r#"str.trim("  hi  ")"#)), "hi");
    assert_eq!(s(eval(r#"str.capitalize("hello")"#)), "Hello");
}

#[test]
fn str_search_and_slice() {
    assert_eq!(eval(r#"str.index_of("hello", "llo")"#), Value::Integer(2));
    // Absence is Unit (0.68): the -1 sentinel was dangerous next to
    // negative indexing, and Unit is what every other lookup answers.
    assert_eq!(eval(r#"str.index_of("hello", "z")"#), Value::Unit);
    assert_eq!(eval(r#"str.last_index_of("hello", "z")"#), Value::Unit);
    assert_eq!(s(eval(r#"str.substring("hello world", 6, 11)"#)), "world");
    // substring clamps out-of-range indices rather than failing
    assert_eq!(s(eval(r#"str.substring("hi", 0, 99)"#)), "hi");
    assert_eq!(eval(r#"str.count("banana", "a")"#), Value::Integer(3));
}

#[test]
fn str_transform() {
    assert_eq!(s(eval(r#"str.replace("a-b-c", "-", "+")"#)), "a+b+c");
    assert_eq!(s(eval(r#"str.replace_first("a-b-c", "-", "+")"#)), "a+b-c");
    assert_eq!(s(eval(r#"str.repeat("ab", 3)"#)), "ababab");
    assert_eq!(s(eval(r#"str.pad_start("7", 3, "0")"#)), "007");
    assert_eq!(s(eval(r#"str.reverse("abc")"#)), "cba");
    assert_eq!(s(eval(r#"str.join(["a", "b", "c"], "-")"#)), "a-b-c");
}

#[test]
fn str_char_indexing_is_unicode() {
    // "héllo": index 1 is the accented e; length counts characters
    assert_eq!(eval(r#"str.length("héllo")"#), Value::Integer(5));
    assert_eq!(s(eval(r#"str.char_at("héllo", 1)"#)), "é");
    // out-of-range char_at yields empty string, not an error
    assert_eq!(s(eval(r#"str.char_at("hi", 9)"#)), "");
}

// ── str: fallible operations return Result ──────────────────────────

#[test]
fn str_parse_returns_result() {
    assert_eq!(eval(r#"unwrap(str.parse_int("42"))"#), Value::Integer(42));
    assert_eq!(
        eval(r#"match str.parse_int("nope") { Ok(_) => "ok", Err(_) => "err" }"#),
        Value::String("err".to_string().into())
    );
    assert_eq!(eval(r#"unwrap(str.parse_float("3.5"))"#), Value::Float(3.5));
}

// ── re: every operation returns Result (fallible on bad pattern) ────

#[test]
fn re_matching() {
    assert_eq!(
        eval(r#"unwrap(re.is_match("^\\d+$", "123"))"#),
        Value::Boolean(true)
    );
    assert_eq!(
        eval(r#"unwrap(re.is_match("^\\d+$", "12a"))"#),
        Value::Boolean(false)
    );
    assert_eq!(s(eval(r#"unwrap(re.find("\\d+", "abc123def"))"#)), "123");
}

#[test]
fn re_find_all_and_split() {
    let list = eval(r#"unwrap(re.find_all("\\d+", "a1b22c333"))"#);
    match list {
        Value::List(items) => {
            let got: Vec<String> = items.iter().cloned().map(s).collect();
            assert_eq!(got, vec!["1", "22", "333"]);
        }
        other => panic!("expected list, got {:?}", other),
    }
    assert_eq!(
        s(eval(r#"unwrap(re.replace_all("\\s+", "a  b   c", "_"))"#)),
        "a_b_c"
    );
}

#[test]
fn re_captures_groups() {
    let groups = eval(r#"unwrap(re.captures("(\\w+)@(\\w+)", "user@host"))"#);
    match groups {
        Value::List(items) => {
            let got: Vec<String> = items.iter().cloned().map(s).collect();
            assert_eq!(got, vec!["user@host", "user", "host"]);
        }
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn re_bad_pattern_is_recoverable_err() {
    // A malformed pattern is an olang Err, not a crash
    assert_eq!(
        eval(r#"match re.is_match("[unclosed", "x") { Ok(_) => "ok", Err(_) => "err" }"#),
        Value::String("err".to_string().into())
    );
    // is_valid is total: it answers the question with a bare bool
    assert_eq!(eval(r#"re.is_valid("[a-z]+")"#), Value::Boolean(true));
    assert_eq!(eval(r#"re.is_valid("[bad")"#), Value::Boolean(false));
}

// ── convention: total vs fallible across the wider stdlib ───────────

#[test]
fn stdlib_convention_holds() {
    // Total operations return bare values (no unwrap needed)
    assert!(matches!(eval(r#"crypto.sha256("x")"#), Value::String(_)));
    assert!(matches!(eval(r#"base64.encode("x")"#), Value::String(_)));
    assert!(matches!(
        eval(r#"dates.is_leap_year(2024)"#),
        Value::Boolean(_)
    ));

    // Fallible operations return Result (Ok/Err)
    assert!(matches!(eval(r#"crypto.hash_password("x")"#), Value::Ok(_)));
    assert!(matches!(eval(r#"base64.decode("aGk=")"#), Value::Ok(_)));
    assert!(matches!(
        eval(r#"dates.add_days("2026-01-01", 5)"#),
        Value::Ok(_)
    ));
    // A bad date is a recoverable Err, not a crash
    assert!(matches!(eval(r#"dates.add_days("bad", 5)"#), Value::Err(_)));
}

#[test]
fn os_args_returns_a_list() {
    // Without the CLI setting script args, os.args() falls back to the
    // process args — but it must always return Ok(list-of-strings).
    match eval(r#"os.args()"#) {
        Value::List(items) => assert!(items.iter().all(|v| matches!(v, Value::String(_)))),
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn log_line_parsing_with_captures() {
    // The parsing approach the loganalyzer example uses: a capture regex over
    // a log line yields [whole, time, level, message]. Guards the core.
    let src = r#"
let groups = unwrap(re.captures("^(\\S+ \\S+) \\[(\\w+)\\] (.+)$", "2026-08-06 09:13:15 [ERROR] db failed: timeout"))
[groups[1], groups[2], groups[3]]
"#;
    match eval(src) {
        Value::List(items) => {
            assert_eq!(s(items[0].clone()), "2026-08-06 09:13:15");
            assert_eq!(s(items[1].clone()), "ERROR");
            assert_eq!(s(items[2].clone()), "db failed: timeout");
        }
        other => panic!("expected list, got {:?}", other),
    }
    // A non-matching line yields Ok([]) — the analyzer treats that as malformed.
    let no_match = eval(r#"len(unwrap(re.captures("^(\\S+) \\[(\\w+)\\]$", "junk line")))"#);
    assert_eq!(no_match, Value::Integer(0));
}

// ── concurrency (found by dogfooding the scheduler) ──
//
// These began as four Promise regressions. 0.63 removed that API; what
// they were really guarding — that a dynamically-built list of jobs
// works, and that concurrent jobs overlap — survives, expressed against
// `spawn` + `task.join`.

#[test]
fn a_dynamically_built_list_of_tasks_joins() {
    // The original guarded `Promise.all` accepting any list expression,
    // not only a literal. The same question applies to `map(task.join)`.
    let src = r#"
fn id(n) = n
let ts = range(1, 4) |> map((n) => spawn id(n))
ts |> map(task.join)
"#;
    match eval(src) {
        Value::List(items) => assert_eq!(
            items,
            vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)].into()
        ),
        other => panic!("expected list, got {:?}", other),
    }
}

#[test]
fn joined_tasks_keep_their_order_and_overlap_in_time() {
    // Order follows the handles, not completion: the slowest task is
    // first in the list and first in the result. And the total is one
    // task's time, not the sum.
    let src = r#"
fn work(name, ms) = { time.sleep(ms); name }
let jobs = [spawn work("slow", 60), spawn work("fast", 5)]
join(jobs |> map(task.join), ",")
"#;
    let started = std::time::Instant::now();
    assert_eq!(s(eval(src)), "slow,fast");
    assert!(
        started.elapsed().as_millis() < 150,
        "tasks should overlap, took {:?}",
        started.elapsed()
    );
}

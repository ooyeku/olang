//! What untrusted input can ask of olang is bounded by errors, never by
//! the process ending (release builds abort on any panic, and a stack
//! overflow aborts): a count a builtin is given that would build more
//! than a list or a string holds (`str.repeat("x", 1 << 62)` in a meta
//! fn killed the language server on a file opened), `range`'s step at
//! i64's end (it wrapped and looped forever), and brackets nested past
//! what the parser's recursion holds.

use olang::ast::Value;
use olang::{Interpreter, Parser};

// the interpreter's own stack (src/main.rs): a test thread's 2 MiB holds
// fewer nested brackets than a run does
fn big<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("join")
}

fn run(src: &str) -> Result<Value, String> {
    let src = src.to_string();
    big(move || {
        let program = Parser::new().parse(&src).map_err(|e| e.to_string())?;
        Interpreter::new()
            .eval_program(program)
            .map_err(|e| e.to_string())
    })
}

fn parse_err(src: String) -> Option<String> {
    big(move || Parser::new().parse(&src).err().map(|e| e.to_string()))
}

#[test]
fn counts_past_what_a_list_or_string_holds_are_errors_not_aborts() {
    for src in [
        "range(1 << 62)",
        "range(-5, 1 << 62)",
        "range(0, 1 << 62, 2)",
        "col.filled(1 << 62, 0)",
        "str.repeat(\"ab\", 1 << 62)",
        "str.repeat(\"ab\", 9223372036854775807)",
        "str.pad_start(\"a\", 1 << 62, \" \")",
    ] {
        let r = run(src);
        assert!(matches!(r, Err(_) | Ok(Value::Err(_))), "{}: {:?}", src, r);
    }
    assert_eq!(run("len(range(0, 10, 3))").unwrap(), Value::Integer(4));
    assert_eq!(run("len(range(10, 0, -3))").unwrap(), Value::Integer(4));
    assert_eq!(
        run("len(str.repeat(\"ab\", 3))").unwrap(),
        Value::Integer(6)
    );
}

#[test]
fn a_step_past_the_end_of_i64_stops_the_range() {
    let r = run("range(0, 9223372036854775807, 9223372036854775806)").expect("range");
    assert_eq!(r.to_string(), "[0, 9223372036854775806]");
    let r = run("range(0, -9223372036854775807, -9223372036854775806)").expect("range");
    assert_eq!(r.to_string(), "[0, -9223372036854775806]");
}

#[test]
fn brackets_nested_past_the_limit_are_a_parse_error_not_a_stack_overflow() {
    let n = 200_000;
    for (open, close) in [("(", ")"), ("[", "]"), ("{", "}")] {
        let src = format!("let x = {}1{}\n", open.repeat(n), close.repeat(n));
        let err = parse_err(src).expect("too deep");
        assert!(err.contains("nested more than"), "{}", err);
    }
    // inside a template's ${…} too
    let src = format!("let x = `a${{{}1{}}}`\n", "(".repeat(n), ")".repeat(n));
    assert!(parse_err(src).is_some());
    // brackets in strings, raw strings, templates' text, char literals and
    // comments are not counted
    let many = "(".repeat(5000);
    let src = format!(
        "// {many}\nlet a = \"{many}\"\nlet b = r\"{many}\"\nlet c = `{many} ${{len([1, 2])}} {many}`\nlet d = '('\nlen(a) + len(b) + str.length(c) + (if d == '(' => 1 else => 0)\n"
    );
    assert_eq!(
        run(&src).expect("runs"),
        Value::Integer(5000 + 5000 + 10003 + 1)
    );
    // a real depth parses
    let src = format!("{}1{}\n", "(".repeat(200), ")".repeat(200));
    assert_eq!(run(&src).expect("runs"), Value::Integer(1));
}

#[test]
fn a_notebooks_front_matter_nested_past_the_limit_is_a_diagnostic_not_a_stack_overflow() {
    let text = format!(
        "---\ntitle: x\nparams:\n  n: {}1{}\n---\n\n```olang\n1\n```\n",
        "[".repeat(500_000),
        "]".repeat(500_000)
    );
    let nb = big(move || olang::notebook::parse(&text));
    assert!(
        nb.diagnostics
            .iter()
            .any(|d| format!("{:?}", d).contains("nested more than")),
        "{:?}",
        nb.diagnostics.iter().take(2).collect::<Vec<_>>()
    );
}

#[test]
fn a_git_dependencys_url_or_ref_that_reads_as_an_option_is_refused_before_git_runs() {
    use olang::pkg::cache::{CacheError, GitRef, fetch_git};
    for (url, r) in [
        ("--upload-pack=touch /tmp/olang-pwned;", GitRef::Default),
        ("-c", GitRef::Default),
        (
            "https://example.invalid/x.git",
            GitRef::Rev("--output=/tmp/x".into()),
        ),
        ("https://example.invalid/x.git", GitRef::Branch("-q".into())),
    ] {
        match fetch_git(url, &r, false) {
            Err(CacheError::Git(m)) => assert!(m.contains("cannot start with '-'"), "{}", m),
            Err(other) => panic!("{}: {}", url, other),
            Ok(_) => panic!("{} fetched", url),
        }
    }
}

#[test]
fn expressions_nested_without_brackets_past_the_limit_are_a_parse_error() {
    let n = 100_000;
    for src in [
        format!(
            "let a = true\nlet x = {}1{}\n",
            "if a => ".repeat(n),
            " else => 0".repeat(n)
        ),
        format!("let f = {}1\n", "(x) => ".repeat(n)),
        // the same, a level a line: a line that ends in `=>` goes on
        format!(
            "let a = true\nlet x = {}1{}\n",
            "if a =>\n".repeat(n),
            "\nelse => 0".repeat(n)
        ),
        format!("let f = {}1\n", "(x) =>\n".repeat(n)),
    ] {
        let err = parse_err(src).expect("too deep");
        assert!(err.contains("nested more than"), "{}", err);
    }
    // a real depth parses and runs, brackets to their own limit too
    let src = format!(
        "let a = true\n{}1{}\n",
        "if a => ".repeat(500),
        " else => 0".repeat(500)
    );
    assert_eq!(run(&src).expect("runs"), Value::Integer(1));
    // a long match, its arms a line each, is no nesting
    let arms: String = (0..10_000).map(|k| format!("    {k} => {k}\n")).collect();
    let src = format!("let n = 9999\nmatch n {{\n{arms}    _ => -1\n}}\n");
    assert_eq!(run(&src).expect("runs"), Value::Integer(9999));
    let src = format!("{}1{}\n", "(".repeat(1000), ")".repeat(1000));
    assert_eq!(run(&src).expect("runs"), Value::Integer(1));
}

#[test]
fn a_value_too_deep_for_toml_is_an_error_not_a_stack_overflow() {
    let r = run(
        "let mut v = #{ \"a\": 1 }\nfor i in range(20000) { v = #{ \"a\": v } }\ntoml.stringify(v)",
    )
    .expect("runs");
    assert!(
        matches!(&r, Value::Err(e) if e.to_string().contains("nests more than")),
        "{:?}",
        r.to_string().chars().take(200).collect::<String>()
    );
    let r = run("toml.stringify(#{ \"a\": #{ \"b\": [1, 2] } })").expect("runs");
    assert!(matches!(r, Value::Ok(_)), "{}", r);
}

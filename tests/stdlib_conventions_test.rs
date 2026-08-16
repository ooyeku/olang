//! The stdlib conventions settled in 0.64 (roadmap lane S6).
//!
//! Three rules decide what a function returns:
//!
//!   * cannot fail        -> returns its value
//!   * can fail, handleably -> returns `Result`
//!   * called wrongly     -> raises
//!
//! The third is what makes the second trustworthy: if misuse also came
//! back as `Err`, `unwrap_or(f(x), default)` would swallow a typo'd call
//! exactly the way it swallows a real failure.
//!
//! Also covers the seven declaration keywords freed as identifiers, and
//! `fs.join`'s variadic shape.

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

// ── rule 1: infallible operations return their value ──────────────────

#[test]
fn host_introspection_returns_values_not_results() {
    // These cannot fail, so wrapping them in Result was pure noise —
    // `unwrap(os.args())` appeared in nearly every script.
    assert_eq!(run("typeof(os.args())").unwrap(), "List");
    assert_eq!(run("typeof(os.arch())").unwrap(), "String");
    assert_eq!(run("typeof(os.os_type())").unwrap(), "String");
    assert_eq!(run("typeof(os.family())").unwrap(), "String");
    assert_eq!(run("typeof(os.pid())").unwrap(), "Int");
    assert_eq!(run("typeof(os.path_separator())").unwrap(), "String");
    assert_eq!(run("typeof(os.temp_dir())").unwrap(), "String");
    assert_eq!(run("typeof(os.username())").unwrap(), "String");
    assert_eq!(run("typeof(os.is_tty())").unwrap(), "Bool");
    assert_eq!(run("typeof(os.has_env(\"PATH\"))").unwrap(), "Bool");
    // list_env returns a struct-like record, not a bare Map.
    assert_eq!(
        run("typeof(os.list_env())").unwrap(),
        "EnvironmentVariables"
    );
    assert_eq!(run("typeof(os.interrupted())").unwrap(), "Bool");
}

#[test]
fn path_predicates_return_bools() {
    assert_eq!(run("typeof(fs.exists(\"/tmp\"))").unwrap(), "Bool");
    assert_eq!(run("typeof(fs.is_dir(\"/tmp\"))").unwrap(), "Bool");
    assert_eq!(run("typeof(fs.is_file(\"/tmp\"))").unwrap(), "Bool");
    assert_eq!(run("fs.is_dir(\"/tmp\")").unwrap(), "true");
}

// ── rule 2: genuine failure still returns Result ───────────────────────

#[test]
fn operations_that_can_fail_keep_their_result() {
    // An absent environment variable is a condition the caller handles,
    // not a bug — so it stays a Result.
    assert_eq!(
        run("show(os.get_env(\"OLANG_DEFINITELY_NOT_SET_XYZ\"))")
            .unwrap()
            .starts_with("Err(")
            .to_string(),
        "true"
    );
    assert_eq!(run("typeof(os.cwd())").unwrap(), "Result");
    assert_eq!(run("typeof(os.home_dir())").unwrap(), "Result");
    // Reading a missing file is environmental, so still a Result.
    let out = run("show(fs.read_file(\"/definitely/not/here.txt\"))").unwrap();
    assert!(out.starts_with("Err("), "{out}");
}

// ── rule 3: misuse raises ─────────────────────────────────────────────

#[test]
fn a_wrong_argument_count_raises() {
    for source in [
        "os.arch(1, 2)",
        "os.args(\"extra\")",
        "fs.exists()",
        "os.get_env()",
    ] {
        let e = err(source);
        assert!(
            e.contains("expects") || e.contains("at least"),
            "{source}: {e}"
        );
    }
}

#[test]
fn a_wrong_argument_type_raises() {
    for source in ["fs.exists(42)", "os.get_env(7)", "os.has_env(true)"] {
        let e = err(source);
        assert!(e.contains("must be"), "{source}: {e}");
    }
}

#[test]
fn misuse_cannot_be_swallowed_by_a_default() {
    // The load-bearing consequence. Before 0.64 a typo'd call came back as
    // Err, so `unwrap_or` quietly substituted the default and the bug
    // survived. Now the program stops.
    let e = err("unwrap_or(fs.exists(42), false)");
    assert!(e.contains("must be a string"), "{e}");
}

#[test]
fn error_messages_name_the_module_qualified_function() {
    // "arch expects 0 arguments" left the reader guessing which `arch`.
    let e = err("os.arch(1)");
    assert!(e.contains("os.arch"), "{e}");
}

// ── the seven freed keywords ──────────────────────────────────────────

#[test]
fn declaration_keywords_work_as_identifiers() {
    // Each of these is a plausible variable in data or statistical code,
    // and reserving them cost more than it bought.
    let out = run("let share = 3.0 / 4.0\n\
                   let error = 0.02\n\
                   let test = \"case-1\"\n\
                   let type = \"Reading\"\n\
                   let trait = \"warm\"\n\
                   let impl = 1\n\
                   let use = \"yes\"\n\
                   str.fmt(\"{} {} {} {} {} {} {}\", share, error, test, type, trait, impl, use)\n")
    .unwrap();
    assert_eq!(out, "0.75 0.02 case-1 Reading warm 1 yes");
}

#[test]
fn declaration_keywords_work_as_field_names() {
    let out = run("let row = { type: \"sensor\", error: 0.5, test: true }\n\
                   row.type + \" \" + to_string(row.error) + \" \" + to_string(row.test)\n")
    .unwrap();
    assert_eq!(out, "sensor 0.5 true");
}

#[test]
fn the_declarations_themselves_still_parse() {
    // Freeing the words must not cost the forms they introduce: the token
    // after each one disambiguates, and PEG backtracking does the rest.
    let out = run("type T = struct { v: Int }\n\
                   trait Nameable { fn nm(self) -> String }\n\
                   impl Nameable for T { fn nm(self) = \"t\" }\n\
                   share fn exported() = \"yes\"\n\
                   test \"a block\" { assert_eq(1, 1) }\n\
                   exported() + \" \" + T { v: 7 }.nm()\n")
    .unwrap();
    assert_eq!(out, "yes t");
}

#[test]
fn a_freed_keyword_and_its_declaration_coexist_in_one_file() {
    // The case that would break if the grammar committed to one reading.
    let out = run("let type = \"a value\"\n\
                   type Point = struct { x: Int }\n\
                   type + \" / \" + to_string(Point { x: 1 }.x)\n")
    .unwrap();
    assert_eq!(out, "a value / 1");
}

// ── fs.join ───────────────────────────────────────────────────────────

#[test]
fn fs_join_takes_parts_variadically_or_as_a_list() {
    // Variadic for the literal case; a list for the computed one, because
    // forcing a spread there would be a downgrade.
    assert_eq!(
        run("fs.join(\"data\", \"raw\", \"x.csv\")").unwrap(),
        "data/raw/x.csv"
    );
    assert_eq!(
        run("fs.join([\"data\", \"raw\", \"x.csv\"])").unwrap(),
        "data/raw/x.csv"
    );
    assert_eq!(
        run("let segs = [\"a\", \"b\"]\nfs.join(segs)\n").unwrap(),
        "a/b"
    );
    assert_eq!(run("fs.join(\"one\")").unwrap(), "one");
}

#[test]
fn fs_join_rejects_a_non_string_part() {
    let e = err("fs.join(\"a\", 2)");
    assert!(e.contains("every part must be a string"), "{e}");
    let e = err("fs.join()");
    assert!(e.contains("at least one"), "{e}");
}

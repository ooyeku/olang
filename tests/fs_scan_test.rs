//! `fs.scan` and `fs.search`: a checkout's tree walked as a person sees it
//! (every `.gitignore` respected, hidden entries and named folders left
//! out) and searched on worker threads — what an editor lists and
//! searches a workspace with.

use olang::{Interpreter, Parser, Value};
use std::fs;

fn eval(src: &str) -> Value {
    let parser = Parser::new();
    let program = parser.parse(src).expect("parse");
    let mut interpreter = Interpreter::new();
    interpreter.eval_program(program).expect("eval")
}

fn s(v: Value) -> String {
    match v {
        Value::String(ref s) => s.to_string(),
        other => panic!("expected string, got {:?}", other),
    }
}

/// A small checkout: two projects, a nested one, ignored and hidden
/// folders, a binary file.
fn tree() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let w = |rel: &str, text: &[u8]| {
        let p = d.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    };
    w("app/olang.toml", b"[package]\nname = \"app\"\n");
    w(
        "app/main.ol",
        b"use lib.util\nshare fn main() = println(util.twice(2))\n",
    );
    w(
        "app/lib/util.ol",
        b"share fn twice(n) = n * 2\nlet Twice = 0\n",
    );
    w("app/target/out.ol", b"fn built() = 1\n");
    w(
        "app/examples/demo/olang.toml",
        b"[package]\nname = \"demo\"\n",
    );
    w("app/examples/demo/main.ol", b"fn demo() = \"twice\"\n");
    w("lib2/.gitignore", b"generated/\n*.log\n");
    w("lib2/index.ol", b"share fn helper() = 1\n");
    w("lib2/generated/big.ol", b"fn gen() = 1\n");
    w("lib2/run.log", b"twice\n");
    w("lib2/node_modules/x.ol", b"fn x() = 1\n");
    w(".hidden/secret.ol", b"fn secret() = 1\n");
    w("loose/script.ol", b"println(\"\xc3\xa9 twice \xc3\xa9\")\n");
    w("loose/data.bin.ol", b"\x00\x01twice");
    w("notes.md", b"# twice\n");
    d
}

fn root_of(d: &tempfile::TempDir) -> String {
    d.path().to_str().unwrap().to_string()
}

#[test]
fn scan_lists_what_a_checkout_keeps_relative_and_sorted() {
    let d = tree();
    let r = root_of(&d);
    let got = s(eval(&format!(
        r#"show(unwrap(fs.scan("{}", #{{ "skip": ["target", "node_modules"] }})))"#,
        r
    )));
    assert_eq!(
        got,
        r#"["app/examples/demo/main.ol", "app/examples/demo/olang.toml", "app/lib/util.ol", "app/main.ol", "app/olang.toml", "lib2/index.ol", "loose/data.bin.ol", "loose/script.ol", "notes.md"]"#
    );
}

#[test]
fn scan_keeps_extensions_hidden_entries_and_ignored_files_on_request() {
    let d = tree();
    let r = root_of(&d);
    let ols = s(eval(&format!(
        r#"show(unwrap(fs.scan("{}", #{{ "exts": ["toml"] }})))"#,
        r
    )));
    assert_eq!(ols, r#"["app/examples/demo/olang.toml", "app/olang.toml"]"#);
    let hidden = eval(&format!(
        r#"contains(unwrap(fs.scan("{}", #{{ "hidden": true }})), ".hidden/secret.ol")"#,
        r
    ));
    assert_eq!(hidden, Value::Boolean(true));
    let ignored = eval(&format!(
        r#"contains(unwrap(fs.scan("{}", #{{ "gitignore": false }})), "lib2/generated/big.ol")"#,
        r
    ));
    assert_eq!(ignored, Value::Boolean(true));
    let respected = eval(&format!(
        r#"contains(unwrap(fs.scan("{}")), "lib2/generated/big.ol")"#,
        r
    ));
    assert_eq!(respected, Value::Boolean(false));
    assert_eq!(
        eval(&format!(
            r#"len(unwrap(fs.scan("{}", #{{ "limit": 2 }})))"#,
            r
        )),
        Value::Integer(2)
    );
    assert_eq!(
        eval(&format!(
            r#"unwrap(fs.scan("{}", #{{ "max_depth": 1 }}))"#,
            r
        )),
        eval(r#"["notes.md"]"#)
    );
}

/// `within` and `own`: only some folders read, by the whole walk's rules
/// (a parent's .gitignore, the skipped and hidden folders on the way), so
/// a watch's changed folders are listed as a whole scan lists them.
#[test]
fn scan_within_some_folders_lists_them_as_the_whole_walk_would() {
    let d = tree();
    let r = root_of(&d);
    let within = |opts: &str| {
        s(eval(&format!(
            r#"show(unwrap(fs.scan("{}", {})))"#,
            r, opts
        )))
    };
    assert_eq!(
        within(r#"#{ "skip": ["target"], "within": ["app/lib", "loose"] }"#),
        r#"["app/lib/util.ol", "loose/data.bin.ol", "loose/script.ol"]"#
    );
    // the folder's own files only; the root's own too
    assert_eq!(
        within(r#"#{ "own": ["app", ""] }"#),
        r#"["app/main.ol", "app/olang.toml", "notes.md"]"#
    );
    // a folder a parent's .gitignore leaves out stays out, as do the
    // skipped and the hidden
    assert_eq!(within(r#"#{ "within": ["lib2/generated"] }"#), "[]");
    assert_eq!(
        within(r#"#{ "skip": ["node_modules"], "within": ["lib2/node_modules"] }"#),
        "[]"
    );
    assert_eq!(within(r#"#{ "within": [".hidden"] }"#), "[]");
    assert_eq!(within(r#"#{ "own": ["lib2/generated"] }"#), "[]");
    // a folder that is not there: nothing
    assert_eq!(within(r#"#{ "within": ["nowhere"] }"#), "[]");
}

#[test]
fn scan_refuses_what_is_not_a_folder_and_options_it_does_not_know() {
    assert_eq!(
        eval(r#"is_err(fs.scan("/no/such/folder/here"))"#),
        Value::Boolean(true)
    );
    let d = tree();
    let msg = s(eval(&format!(
        r#"match attempt(() => fs.scan("{}", #{{ "deep": true }})) {{ Ok(x) => "", Err(e) => e }}"#,
        root_of(&d)
    )));
    assert!(msg.contains("unknown option 'deep'"), "{}", msg);
}

#[test]
fn search_finds_a_literal_ignoring_case_by_default_with_columns_in_characters() {
    let d = tree();
    let r = root_of(&d);
    let got = s(eval(&format!(
        r#"show(map(unwrap(fs.search("{}", "TWICE", #{{ "skip": ["target"] }})), (h) => map_get(h, "path") + ":" + to_string(map_get(h, "line")) + ":" + to_string(map_get(h, "column")) + "-" + to_string(map_get(h, "end_column"))))"#,
        r
    )));
    // the binary file is not searched; the é before the match counts one
    assert_eq!(
        got,
        r#"["app/examples/demo/main.ol:1:14-19", "app/lib/util.ol:1:10-15", "app/lib/util.ol:2:5-10", "app/main.ol:2:32-37", "loose/script.ol:1:12-17", "notes.md:1:3-8"]"#
    );
    let text = s(eval(&format!(
        r#"map_get(unwrap(fs.search("{}", "twice", #{{ "files": ["app/lib/util.ol"] }}))[0], "text")"#,
        r
    )));
    assert_eq!(text, "share fn twice(n) = n * 2");
}

/// A case-blind search reads a file whole before its lines: a data file
/// of megabytes holding no match costs a pass over its bytes, and one
/// holding a match in any case (or the Kelvin sign's K) is still found.
#[test]
fn a_case_blind_search_passes_a_large_file_holding_no_match_and_finds_one_in_any_case() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().to_string_lossy().to_string();
    let row = "Entity,Year,CO2,Energy per Capita\n";
    std::fs::write(d.path().join("big.csv"), row.repeat(200_000)).unwrap();
    std::fs::write(d.path().join("mixed.txt"), "a ValUE here\nnone\n").unwrap();
    std::fs::write(d.path().join("kelvin.txt"), "300 \u{212A}elvin\n").unwrap();
    let t0 = std::time::Instant::now();
    let got = s(eval(&format!(
        r#"show(map(unwrap(fs.search("{r}", "value")), (h) => map_get(h, "path") + ":" + to_string(map_get(h, "line"))))"#
    )));
    assert_eq!(got, r#"["mixed.txt:1"]"#);
    assert!(t0.elapsed().as_millis() < 2000, "{:?}", t0.elapsed());
    let got = s(eval(&format!(
        r#"show(map(unwrap(fs.search("{r}", "kelvin")), (h) => map_get(h, "path")))"#
    )));
    assert_eq!(got, r#"["kelvin.txt"]"#);
}

#[test]
fn search_matches_case_words_and_regular_expressions_and_stops_at_its_limit() {
    let d = tree();
    let r = root_of(&d);
    let cased = eval(&format!(
        r#"len(unwrap(fs.search("{}", "Twice", #{{ "case": true }})))"#,
        r
    ));
    assert_eq!(cased, Value::Integer(1));
    let words = eval(&format!(
        r#"len(unwrap(fs.search("{}", "twice", #{{ "word": true, "files": ["app/main.ol", "app/lib/util.ol"] }})))"#,
        r
    ));
    assert_eq!(words, Value::Integer(3));
    let decls = s(eval(&format!(
        r#"show(map(unwrap(fs.search("{}", "^(?:share\\s+)?fn\\s+\\w+", #{{ "regex": true, "files": ["app/lib/util.ol", "app/main.ol", "lib2/index.ol"] }})), (h) => map_get(h, "path") + ":" + to_string(map_get(h, "line"))))"#,
        r
    )));
    assert_eq!(
        decls,
        r#"["app/lib/util.ol:1", "app/main.ol:2", "lib2/index.ol:1"]"#
    );
    assert_eq!(
        eval(&format!(
            r#"len(unwrap(fs.search("{}", "twice", #{{ "limit": 2 }})))"#,
            r
        )),
        Value::Integer(2)
    );
    assert_eq!(
        eval(&format!(
            r#"is_err(fs.search("{}", "(", #{{ "regex": true }}))"#,
            r
        )),
        Value::Boolean(true)
    );
    assert_eq!(
        eval(&format!(r#"unwrap(fs.search("{}", ""))"#, r)),
        eval("[]")
    );
}

#[test]
fn search_says_what_each_match_becomes_with_a_replacement_groups_expanded() {
    let d = tree();
    let r = root_of(&d);
    // a literal: the template as it is
    let lit = s(eval(&format!(
        r#"show(map(unwrap(fs.search("{}", "twice", #{{ "replace": "double", "files": ["app/lib/util.ol"] }})), (h) => map_get(h, "with")))"#,
        r
    )));
    assert_eq!(lit, r#"["double", "double"]"#);
    // a regular expression: its groups, by number and by name, and $$
    let rx = s(eval(&format!(
        r#"show(map(unwrap(fs.search("{}", "fn (?P<name>\\w+)\\((\\w*)\\)", #{{ "regex": true, "replace": "fn ${{name}}_v2($2) $$", "files": ["app/lib/util.ol"] }})), (h) => map_get(h, "with")))"#,
        r
    )));
    assert_eq!(rx, r#"["fn twice_v2(n) $"]"#);
    // no replacement asked: no `with`
    assert_eq!(
        eval(&format!(
            r#"map_get(unwrap(fs.search("{}", "twice"))[0], "with")"#,
            r
        )),
        Value::Unit
    );
    assert_eq!(
        eval(&format!(
            r#"is_err(fs.search("{}", "twice", #{{ "replace": 3 }}))"#,
            r
        )),
        Value::Boolean(true)
    );
}

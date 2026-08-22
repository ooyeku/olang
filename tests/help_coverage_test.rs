//! The REPL's `:help` must cover every module a program can call.
//! Regression for `:help meta.parse` returning "No help found" — the
//! `meta` module shipped without help entries. The module list below is
//! the checklist: a new module lands with at least one help entry, or
//! this test names it.

use olang::help::HelpSystem;

#[test]
fn meta_parse_has_a_full_help_entry() {
    let help = HelpSystem::new();
    let doc = help
        .find_function_by_name("meta.parse")
        .expect(":help meta.parse must resolve");
    assert!(doc.syntax.contains("meta.parse(source)"));
    assert!(!doc.examples.is_empty(), "meta.parse help carries examples");
}

#[test]
fn every_callable_module_has_help_entries() {
    let help = HelpSystem::new();
    // Every module namespace a program can call: native modules, the data
    // stack, and the embedded olang modules and packages.
    let modules = [
        "str", "col", "math", "json", "toml", "csv", "re", "dates", "time", "random", "crypto",
        "base64", "bytes", "fs", "os", "http", "db", "chan", "proc", "testing", "meta", "dom",
        "ods", "stats", "plot", "colx", "mathx", "cli", "term", "ui", "viz", "dash",
    ];
    let mut missing = Vec::new();
    for m in modules {
        if help.functions_in_module(m).is_empty() {
            missing.push(m);
        }
    }
    assert!(
        missing.is_empty(),
        "modules with zero :help entries: {missing:?}"
    );
}

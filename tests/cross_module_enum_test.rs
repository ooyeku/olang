//! An enum's variant constructors cross the module boundary. Importing the
//! enum type (or the variants by name, or `*`) makes the variants
//! constructible in the importing module — not only matchable — surfaced by
//! dogfooding a template engine whose AST lives in a shared module.

use olang::{Interpreter, Parser};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_xmod_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A package whose `lib/nodes.ol` shares an enum type. Returns the app dir.
fn scaffold(ws: &Path) -> PathBuf {
    fs::write(
        ws.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::create_dir_all(ws.join("lib")).unwrap();
    fs::write(
        ws.join("lib/nodes.ol"),
        "share type Node = enum { Text(String), Var(String) }\nshare fn mk_text(s) = Text(s)\n",
    )
    .unwrap();
    ws.to_path_buf()
}

fn run(app: &Path, source: &str) -> Result<String, String> {
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    interp.set_current_file(&app.join("main.ol"));
    match interp.eval_program(program).map_err(|e| e.to_string())? {
        olang::Value::String(s) => Ok(s.to_string()),
        other => Err(format!("expected string, got {:?}", other)),
    }
}

#[test]
fn importing_the_type_brings_its_variant_constructors() {
    let ws = workspace("type");
    let app = scaffold(&ws);
    // `Var` is constructed here, in the importer, though only `Node` is named.
    let out = run(
        &app,
        "use lib.nodes { Node }\nmatch Var(\"x\") { Text(s) => \"t:\" + s, Var(p) => \"v:\" + p }",
    );
    assert_eq!(out.as_deref(), Ok("v:x"));
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn variants_can_be_imported_by_name() {
    let ws = workspace("byname");
    let app = scaffold(&ws);
    let out = run(
        &app,
        "use lib.nodes { Text }\nmatch Text(\"hi\") { Text(s) => \"t:\" + s, Var(p) => \"v\" }",
    );
    assert_eq!(out.as_deref(), Ok("t:hi"));
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn a_shared_function_returning_a_variant_still_matches() {
    let ws = workspace("fn");
    let app = scaffold(&ws);
    let out = run(
        &app,
        "use lib.nodes { mk_text }\nmatch mk_text(\"yo\") { Text(s) => \"t:\" + s, Var(p) => \"v\" }",
    );
    assert_eq!(out.as_deref(), Ok("t:yo"));
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn wildcard_import_brings_variants_too() {
    let ws = workspace("wild");
    let app = scaffold(&ws);
    let out = run(
        &app,
        "use lib.nodes { * }\nmatch Var(\"w\") { Text(s) => \"t\", Var(p) => \"v:\" + p }",
    );
    assert_eq!(out.as_deref(), Ok("v:w"));
    let _ = fs::remove_dir_all(&ws);
}

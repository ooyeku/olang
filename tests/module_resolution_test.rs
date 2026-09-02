//! Module resolution across package boundaries (roadmap W9): a module
//! inside a dependency resolves its own `use lib.x` against its package,
//! and the module cache is keyed by the resolved file rather than the
//! dotted name — so two packages that each have a `lib.util` never hand
//! one another's module out, whatever the load order. Both shapes were
//! observed building open-track against the web SDK: a consumer hit
//! "Cannot find module 'lib.state'" unless the SDK's index.ol imported
//! its modules in a lucky order, and a wrapper function ran another
//! package's same-named import.

use olang::pkg::{InstallOptions, install};
use olang::{Interpreter, Parser};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_modres_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

fn run(app: &Path, source: &str) -> Result<String, String> {
    let map = install(app, &InstallOptions::default())
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    let program = Parser::new().parse(source).map_err(|e| e.to_string())?;
    let mut interp = Interpreter::new();
    interp.enable_bytecode_tier(1, false);
    interp.set_current_file(&app.join("main.ol"));
    interp.set_dependency_map(map);
    match interp.eval_program(program).map_err(|e| e.to_string())? {
        olang::Value::String(s) => Ok(s.to_string()),
        other => Err(format!("expected string, got {:?}", other)),
    }
}

/// A dependency whose index imports only `lib.a`, where `lib/a.ol` needs
/// `lib.b` — and a consumer with its own, unrelated `lib/b.ol`.
fn scaffold_nested(ws: &Path) -> PathBuf {
    write(
        &ws.join("sdk/olang.toml"),
        "[package]\nname = \"sdk\"\nversion = \"1.0.0\"\n",
    );
    write(&ws.join("sdk/index.ol"), "share use lib.a { render }\n");
    write(
        &ws.join("sdk/lib/a.ol"),
        "use lib.b { state_name }\nshare fn render() = \"render:\" + state_name()\n",
    );
    write(
        &ws.join("sdk/lib/b.ol"),
        "share fn state_name() = \"sdk-state\"\n",
    );
    write(
        &ws.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nsdk = { path = \"../sdk\" }\n",
    );
    // The consumer's lib/b.ol exports the same name with a different body.
    write(
        &ws.join("app/lib/b.ol"),
        "share fn state_name() = \"app-state\"\n",
    );
    ws.join("app")
}

#[test]
fn a_dependency_resolves_its_own_internal_imports() {
    let ws = workspace("nested");
    let app = scaffold_nested(&ws);
    // Before the fix this either failed with "Cannot find module
    // 'lib.b'" or — worse — silently bound the consumer's lib/b.ol.
    let out = run(&app, "use sdk { render }\nrender()");
    assert_eq!(out.as_deref(), Ok("render:sdk-state"));
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn the_consumers_same_named_module_is_untouched_by_the_dependency() {
    let ws = workspace("both");
    let app = scaffold_nested(&ws);
    // Both `lib.b`s are live in one program; each importer gets its own.
    let out = run(
        &app,
        "use sdk { render }\nuse lib.b { state_name }\nrender() + \"/\" + state_name()",
    );
    assert_eq!(out.as_deref(), Ok("render:sdk-state/app-state"));
    // And in the other load order.
    let out = run(
        &app,
        "use lib.b { state_name }\nuse sdk { render }\nstate_name() + \"/\" + render()",
    );
    assert_eq!(out.as_deref(), Ok("app-state/render:sdk-state"));
    let _ = fs::remove_dir_all(&ws);
}

/// The open-track shape: two packages export a generic name (`check`);
/// a wrapper module imports one of them by name; the application imports
/// the wrapper whole and the *other* package's `check` by name.
#[test]
fn an_imported_name_is_scoped_to_the_module_that_imported_it() {
    let ws = workspace("scoped");
    for (pkg, body) in [
        ("rules", "\"rules-check\""),
        ("validate", "\"validate-check\""),
    ] {
        write(
            &ws.join(format!("{pkg}/olang.toml")),
            &format!("[package]\nname = \"{pkg}\"\nversion = \"1.0.0\"\n"),
        );
        write(
            &ws.join(format!("{pkg}/index.ol")),
            "share use lib.util { check }\n",
        );
        write(
            &ws.join(format!("{pkg}/lib/util.ol")),
            &format!("share fn check(x) = {body} + \":\" + x\n"),
        );
    }
    write(
        &ws.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\n\
         rules = { path = \"../rules\" }\nvalidate = { path = \"../validate\" }\n",
    );
    write(
        &ws.join("app/lib/wrapper.ol"),
        "use rules { check }\nshare fn check_rule(src) = check(src)\n",
    );
    // The consumer's own lib/util.ol shares the dotted name both packages
    // use internally.
    write(
        &ws.join("app/lib/util.ol"),
        "share fn check(x) = \"app-check:\" + x\n",
    );
    let app = ws.join("app");
    let out = run(
        &app,
        "use lib.wrapper\nuse validate { check }\nuse lib.util { check as util_check }\n\
         wrapper.check_rule(\"a\") + \" \" + check(\"b\") + \" \" + util_check(\"c\")",
    );
    assert_eq!(
        out.as_deref(),
        Ok("rules-check:a validate-check:b app-check:c")
    );
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn circular_imports_are_still_refused() {
    let ws = workspace("cycle");
    write(
        &ws.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    );
    write(&ws.join("app/lib/x.ol"), "use lib.y\nshare fn fx() = 1\n");
    write(&ws.join("app/lib/y.ol"), "use lib.x\nshare fn fy() = 2\n");
    let err = run(&ws.join("app"), "use lib.x\n\"unreachable\"").unwrap_err();
    assert!(err.to_lowercase().contains("circular"), "{err}");
    let _ = fs::remove_dir_all(&ws);
}

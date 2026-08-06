//! End-to-end package manager tests over path dependencies: install resolves
//! the manifest, writes a lockfile with checksums, and the interpreter
//! resolves `use <dep>` through the dependency map across the package
//! boundary.

use olang::pkg::{install, InstallOptions};
use olang::{Interpreter, Parser};
use std::fs;
use std::path::PathBuf;

/// A throwaway workspace under the temp dir, unique per test.
fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_pkg_it_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// Build a project that depends, by path, on a sibling library package.
fn scaffold(ws: &std::path::Path) -> PathBuf {
    // library package
    write(
        &ws.join("mathlib/olang.toml"),
        "[package]\nname = \"mathlib\"\nversion = \"1.0.0\"\n",
    );
    write(
        &ws.join("mathlib/index.ol"),
        "share fn square(x) = x * x\nshare fn cube(x) = x * x * x\n",
    );
    // application package
    write(
        &ws.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmathlib = { path = \"../mathlib\" }\n",
    );
    ws.join("app")
}

fn run_program(app: &std::path::Path, source: &str) -> olang::Value {
    let map = install(app, &InstallOptions::default())
        .expect("install")
        .into_iter()
        .collect();
    let program = Parser::new().parse(source).expect("parse");
    let mut interp = Interpreter::new();
    interp.set_current_file(&app.join("main.ol"));
    interp.set_dependency_map(map);
    interp.eval_program(program).expect("eval")
}

#[test]
fn install_writes_a_lockfile_with_a_checksum() {
    let ws = workspace("lock");
    let app = scaffold(&ws);

    let map = install(&app, &InstallOptions::default()).expect("install");
    assert!(map.contains_key("mathlib"), "dependency map has mathlib");

    let lock_text = fs::read_to_string(app.join("olang.lock")).expect("lock written");
    assert!(lock_text.contains("mathlib"), "lock names the dependency");
    assert!(lock_text.contains("checksum"), "lock records a checksum");
    assert!(lock_text.contains("path"), "lock records the path source");

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn use_resolves_across_the_package_boundary() {
    let ws = workspace("use");
    let app = scaffold(&ws);

    let result = run_program(&app, "use mathlib { square, cube }\nsquare(5) + cube(3)");
    assert_eq!(result, olang::Value::Integer(25 + 27));

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn selective_import_only_binds_named_items() {
    let ws = workspace("selective");
    let app = scaffold(&ws);

    // Only `square` is imported; referencing `cube` must fail.
    let map = install(&app, &InstallOptions::default())
        .unwrap()
        .into_iter()
        .collect();
    let program = Parser::new()
        .parse("use mathlib { square }\ncube(2)")
        .unwrap();
    let mut interp = Interpreter::new();
    interp.set_current_file(&app.join("main.ol"));
    interp.set_dependency_map(map);
    assert!(
        interp.eval_program(program).is_err(),
        "cube was not imported"
    );

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn find_root_walks_up_to_the_manifest() {
    let ws = workspace("root");
    let app = scaffold(&ws);
    // A nested file resolves to the app package root, not the workspace.
    let nested = app.join("deep/nested/file.ol");
    write(&nested, "1");
    let root = olang::pkg::manifest::Manifest::find_root(&nested).expect("finds root");
    assert_eq!(root, app);

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn frozen_install_rejects_a_stale_lock() {
    let ws = workspace("frozen");
    let app = scaffold(&ws);

    // Seed a lockfile that doesn't match reality.
    write(
        &app.join("olang.lock"),
        "version = 1\n\n[package.ghost]\ndependencies = []\n\n[package.ghost.source]\nkind = \"path\"\npath = \"../ghost\"\n",
    );
    let frozen = InstallOptions {
        frozen: true,
        registry: None,
    };
    assert!(
        install(&app, &frozen).is_err(),
        "frozen install must reject a lock that would change"
    );

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn shared_functions_can_call_private_helpers() {
    // A package's public function calling a package-private helper declared
    // *later* in the file must resolve — the export closes over the whole
    // module, not just what preceded its declaration.
    let ws = workspace("private");
    write(
        &ws.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n",
    );
    write(
        &ws.join("lib/index.ol"),
        // `public` calls `helper`, which is private and defined afterwards.
        "share fn public(x) = helper(x) + 1\nfn helper(x) = x * 10\n",
    );
    write(
        &ws.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib = { path = \"../lib\" }\n",
    );
    let app = ws.join("app");

    let map = install(&app, &InstallOptions::default())
        .unwrap()
        .into_iter()
        .collect();
    let program = Parser::new()
        .parse("use lib { public }\npublic(4)")
        .unwrap();
    let mut interp = Interpreter::new();
    interp.set_current_file(&app.join("main.ol"));
    interp.set_dependency_map(map);
    assert_eq!(
        interp.eval_program(program).unwrap(),
        olang::Value::Integer(41)
    );

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn a_package_is_referable_by_its_own_name() {
    // Regression: running inside a package and doing `use <that package>`
    // must resolve to its own root module. The binaries add the package's
    // own name -> its root to the dependency map; this verifies that a map
    // entry pointing a name at the package directory makes `use name` work.
    let ws = workspace("selfref");
    write(
        &ws.join("geometry/olang.toml"),
        "[package]\nname = \"geometry\"\nversion = \"1.0.0\"\n",
    );
    write(
        &ws.join("geometry/index.ol"),
        "share fn circle(r) = { kind: \"circle\", r: r }\nshare fn area(s) = 3.0 * s.r * s.r\n",
    );
    let geo = ws.join("geometry");

    // Simulate the binary's self-entry: geometry -> its own directory.
    let mut map = std::collections::HashMap::new();
    map.insert("geometry".to_string(), geo.clone());

    let program = Parser::new()
        .parse("use geometry { circle, area }\narea(circle(2.0))")
        .unwrap();
    let mut interp = Interpreter::new();
    interp.set_current_file(&geo.join("olang.toml"));
    interp.set_dependency_map(map);
    assert_eq!(
        interp.eval_program(program).unwrap(),
        olang::Value::Float(12.0)
    );

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn a_package_loaded_by_path_resolves_from_anywhere() {
    // Regression: a package made available by an arbitrary path (as `:pkg
    // load <path>` does) is usable without being in its directory. The
    // resolution layer only needs the name -> directory mapping.
    let ws = workspace("bypath");
    write(
        &ws.join("faraway/lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n",
    );
    write(&ws.join("faraway/lib/index.ol"), "share fn answer() = 42\n");

    // The current file is somewhere unrelated; only the map connects them.
    let elsewhere = ws.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let mut map = std::collections::HashMap::new();
    map.insert("lib".to_string(), ws.join("faraway/lib"));

    let program = Parser::new().parse("use lib { answer }\nanswer()").unwrap();
    let mut interp = Interpreter::new();
    interp.set_current_file(&elsewhere.join("scratch.ol"));
    interp.set_dependency_map(map);
    assert_eq!(
        interp.eval_program(program).unwrap(),
        olang::Value::Integer(42)
    );

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn wildcard_import_binds_all_exports() {
    // `use pkg { * }` imports everything the package shares, unlike a
    // selective `use pkg { a, b }`.
    let ws = workspace("wildcard");
    let app = scaffold(&ws); // mathlib exports square and cube

    let map = install(&app, &InstallOptions::default())
        .unwrap()
        .into_iter()
        .collect();
    // Only square is named explicitly nowhere — the wildcard must bring both.
    let program = Parser::new()
        .parse("use mathlib { * }\nsquare(2) + cube(2)")
        .unwrap();
    let mut interp = Interpreter::new();
    interp.set_current_file(&app.join("main.ol"));
    interp.set_dependency_map(map);
    assert_eq!(
        interp.eval_program(program).unwrap(),
        olang::Value::Integer(4 + 8)
    );

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn bare_use_imports_all_exports() {
    // `use pkg` with no braces is sugar for `use pkg { * }`.
    let ws = workspace("bareuse");
    let app = scaffold(&ws); // mathlib exports square and cube
    let map = install(&app, &InstallOptions::default())
        .unwrap()
        .into_iter()
        .collect();
    let program = Parser::new()
        .parse("use mathlib\nsquare(3) + cube(2)")
        .unwrap();
    let mut interp = Interpreter::new();
    interp.set_current_file(&app.join("main.ol"));
    interp.set_dependency_map(map);
    assert_eq!(
        interp.eval_program(program).unwrap(),
        olang::Value::Integer(9 + 8)
    );
    let _ = fs::remove_dir_all(&ws);
}

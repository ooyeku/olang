//! End-to-end package manager tests over path dependencies: install resolves
//! the manifest, writes a lockfile with checksums, and the interpreter
//! resolves `use <dep>` through the dependency map across the package
//! boundary.

use olang::pkg::{InstallOptions, install};
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
        refresh: false,
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

/// Make a tiny local git repo usable as a git dependency; returns its path
/// (usable as a file:// URL for git) after committing `index.ol`.
fn git_lib(ws: &std::path::Path, source: &str) -> PathBuf {
    let repo = ws.join("gitlib");
    write(
        &repo.join("olang.toml"),
        "[package]\nname = \"gitlib\"\nversion = \"1.0.0\"\n",
    );
    write(&repo.join("index.ol"), source);
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {:?}: {:?}", args, out);
    };
    git(&["init", "--quiet", "--initial-branch=main"]);
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "v1"]);
    repo
}

#[test]
fn install_replays_the_lock_but_update_moves_the_pin() {
    let ws = workspace("replay");
    let repo = git_lib(&ws, "share fn answer() = 1\n");
    let app = ws.join("app");
    write(
        &app.join("olang.toml"),
        &format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ngitlib = {{ git = \"{}\" }}\n",
            repo.display()
        ),
    );

    // First install pins the current commit.
    let map = install(&app, &InstallOptions::default()).expect("first install");
    let v1_dir = map.get("gitlib").unwrap().clone();
    let lock_v1 = fs::read_to_string(app.join("olang.lock")).unwrap();
    assert!(
        fs::read_to_string(v1_dir.join("index.ol"))
            .unwrap()
            .contains("= 1")
    );

    // The upstream moves on.
    write(&ws.join("gitlib/index.ol"), "share fn answer() = 2\n");
    let commit = std::process::Command::new("git")
        .current_dir(&repo)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .args(["commit", "--quiet", "-am", "v2"])
        .output()
        .unwrap();
    assert!(commit.status.success());

    // A plain install replays the pin: same checkout, lock untouched.
    let map = install(&app, &InstallOptions::default()).expect("replay install");
    assert_eq!(map.get("gitlib").unwrap(), &v1_dir);
    assert_eq!(
        fs::read_to_string(app.join("olang.lock")).unwrap(),
        lock_v1,
        "replayed install must not rewrite the lock"
    );

    // An explicit update re-resolves to the new head and rewrites the lock.
    let refresh = InstallOptions {
        refresh: true,
        ..Default::default()
    };
    let map = install(&app, &refresh).expect("update");
    let v2_dir = map.get("gitlib").unwrap().clone();
    assert_ne!(v2_dir, v1_dir, "update must move to the new commit");
    assert!(
        fs::read_to_string(v2_dir.join("index.ol"))
            .unwrap()
            .contains("= 2")
    );
    assert_ne!(fs::read_to_string(app.join("olang.lock")).unwrap(), lock_v1);

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn changing_the_requested_git_ref_re_resolves_without_update() {
    let ws = workspace("refchange");
    let repo = git_lib(&ws, "share fn answer() = 1\n");
    // Tag the first commit, then advance main.
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {:?}: {:?}", args, out);
    };
    git(&["tag", "v1"]);
    write(&ws.join("gitlib/index.ol"), "share fn answer() = 2\n");
    git(&["commit", "--quiet", "-am", "v2"]);
    git(&["tag", "v2"]);

    let app = ws.join("app");
    let manifest_with = |tag: &str| {
        format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ngitlib = {{ git = \"{}\", tag = \"{}\" }}\n",
            repo.display(),
            tag
        )
    };
    write(&app.join("olang.toml"), &manifest_with("v1"));
    let map = install(&app, &InstallOptions::default()).expect("install v1");
    let v1_dir = map.get("gitlib").unwrap().clone();

    // Editing the manifest's tag must re-resolve on a plain install — the
    // lock covers v1, not v2.
    write(&app.join("olang.toml"), &manifest_with("v2"));
    let map = install(&app, &InstallOptions::default()).expect("install v2");
    assert_ne!(map.get("gitlib").unwrap(), &v1_dir);
    assert!(
        fs::read_to_string(map.get("gitlib").unwrap().join("index.ol"))
            .unwrap()
            .contains("= 2")
    );

    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn install_fails_on_a_path_dependency_that_does_not_exist() {
    // A path dependency pointing nowhere must be a resolve error, not a
    // silent lock of an empty checksum that fails opaquely at run time.
    let ws = workspace("missing_path_dep");
    let app = ws.join("app");
    write(
        &app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmylib = { path = \"../no_such_lib\" }\n",
    );
    let err = install(&app, &InstallOptions::default()).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("mylib"), "should name the dependency: {msg}");
    assert!(msg.contains("does not exist"), "should explain: {msg}");
    assert!(
        !app.join("olang.lock").exists(),
        "no lockfile should be written for an unresolvable dependency"
    );
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn publishing_the_same_version_twice_is_rejected() {
    use olang::pkg::registry::{Registry, Release};
    let ws = workspace("pubdup");
    let reg = Registry::at(ws.join("registry"));
    let rel = |rev: &str| Release {
        version: semver::Version::new(1, 0, 0),
        git: "somewhere".into(),
        rev: rev.into(),
        checksum: None,
        dependencies: Default::default(),
    };
    reg.publish("thing", rel("aaaa"), false)
        .expect("first publish");
    // Same version again: append-only, refused — and the index keeps the
    // original release untouched.
    let err = reg.publish("thing", rel("bbbb"), false).unwrap_err();
    assert!(
        err.to_string().contains("append-only"),
        "unexpected error: {err}"
    );
    assert_eq!(reg.entry("thing").unwrap().releases[0].rev, "aaaa");
    // An explicit overwrite is a deliberate act and allowed.
    reg.publish("thing", rel("bbbb"), true)
        .expect("forced publish");
    assert_eq!(reg.entry("thing").unwrap().releases[0].rev, "bbbb");
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn install_rejects_a_tampered_registry_release() {
    use olang::pkg::registry::{Registry, Release};
    let ws = workspace("tampered_release");
    let repo = git_lib(&ws, "share fn answer() = 41\n");
    let rev = {
        let out = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let reg_dir = ws.join("registry");
    let reg = Registry::at(&reg_dir);
    let release = |checksum: Option<String>| Release {
        version: semver::Version::new(1, 0, 0),
        git: repo.display().to_string(),
        rev: rev.clone(),
        checksum,
        dependencies: Default::default(),
    };
    // The index claims a checksum the source does not have.
    reg.publish("gitlib", release(Some("deadbeef".into())), false)
        .unwrap();

    let app = ws.join("app");
    write(
        &app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ngitlib = \"^1.0\"\n",
    );
    let opts = InstallOptions {
        registry: Some(reg_dir.clone()),
        ..Default::default()
    };
    let err = install(&app, &opts).expect_err("tampered release must not install");
    assert!(
        err.to_string().contains("checksum mismatch"),
        "unexpected error: {err}"
    );

    // With the true checksum published, the same install succeeds.
    let real = olang::pkg::cache::checksum_dir(&repo).unwrap();
    reg.publish("gitlib", release(Some(real)), true).unwrap();
    install(&app, &opts).expect("install with the correct checksum");
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn verify_flags_edited_locked_content() {
    use olang::pkg::{VerifyStatus, verify};
    let ws = workspace("verify_drift");
    let app = scaffold(&ws);
    install(&app, &InstallOptions::default()).expect("install");

    // Fresh from install, everything matches its recorded checksum.
    let report = verify(&app, &InstallOptions::default()).expect("verify");
    assert!(!report.is_empty());
    assert!(
        report.iter().all(|(_, _, s)| *s == VerifyStatus::Ok),
        "expected all Ok, got {report:?}"
    );

    // Edit the dependency's source: verify must see the drift.
    write(
        &ws.join("mathlib/index.ol"),
        "share fn square(x) = x + 1\nshare fn cube(x) = x\n",
    );
    let report = verify(&app, &InstallOptions::default()).expect("verify after edit");
    assert!(
        report
            .iter()
            .any(|(n, _, s)| n == "mathlib" && matches!(s, VerifyStatus::Mismatch { .. })),
        "expected mathlib mismatch, got {report:?}"
    );
    let _ = fs::remove_dir_all(&ws);
}

/// An *application* package — `main.ol` plus a `lib/`, no public root
/// module — cannot be imported by its own name, and the error must say so
/// and name what it does export.
///
/// Regression: `use <app-pkg>` reported the generic "Module 'x' not found"
/// with a "system-level error" hint, because the resolver swallowed the
/// dependency-specific diagnosis and fell through to the global search.
/// The reader was sent looking for a missing file rather than told the
/// package has no root module.
#[test]
fn importing_an_application_package_by_name_explains_itself() {
    let ws = workspace("app_pkg_root");
    let app = ws.join("fleet");
    write(
        &app.join("olang.toml"),
        "[package]\nname = \"fleet\"\nversion = \"0.1.0\"\n",
    );
    write(&app.join("main.ol"), "println(\"entry point\")\n");
    write(
        &app.join("lib/stats.ol"),
        "share fn mean(xs) = average(xs)\n",
    );
    install(&app, &InstallOptions::default()).expect("install");

    let mut interp = Interpreter::new();
    interp.set_current_file(&app.join("olang.toml"));
    interp.set_dependency_map(std::collections::HashMap::from([(
        "fleet".to_string(),
        app.clone(),
    )]));

    let program = Parser::new().parse("use fleet\n").expect("parses");
    let err = interp
        .eval_program(program)
        .expect_err("an app package has no root module")
        .to_string();
    assert!(err.contains("has no root module"), "{err}");
    assert!(
        err.contains("index.ol"),
        "should name what it looked for: {err}"
    );
    assert!(
        err.contains("fleet.lib.stats"),
        "should name what IS importable: {err}"
    );

    // And the submodule form works, which is the whole point of saying so.
    let mut interp = Interpreter::new();
    interp.set_current_file(&app.join("olang.toml"));
    interp.set_dependency_map(std::collections::HashMap::from([(
        "fleet".to_string(),
        app.clone(),
    )]));
    let program = Parser::new()
        .parse("use fleet.lib.stats { mean }\nmean([2.0, 4.0])\n")
        .expect("parses");
    assert_eq!(
        format!("{}", interp.eval_program(program).expect("resolves")),
        "3.0"
    );

    let _ = fs::remove_dir_all(&ws);
}

/// The mirror case: a library package with an `index.ol` still resolves by
/// name, so the stricter error path did not break ordinary imports.
#[test]
fn importing_a_library_package_by_name_still_works() {
    let ws = workspace("lib_pkg_root");
    let lib = ws.join("shapes");
    write(
        &lib.join("olang.toml"),
        "[package]\nname = \"shapes\"\nversion = \"0.1.0\"\n",
    );
    write(&lib.join("index.ol"), "share fn area(w, h) = w * h\n");
    install(&lib, &InstallOptions::default()).expect("install");

    let mut interp = Interpreter::new();
    interp.set_current_file(&lib.join("olang.toml"));
    interp.set_dependency_map(std::collections::HashMap::from([(
        "shapes".to_string(),
        lib.clone(),
    )]));
    let program = Parser::new()
        .parse("use shapes { area }\narea(3, 4)\n")
        .expect("parses");
    assert_eq!(
        format!("{}", interp.eval_program(program).expect("resolves")),
        "12"
    );

    let _ = fs::remove_dir_all(&ws);
}

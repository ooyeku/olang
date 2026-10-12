//! `[package] lock = "never"`: a project (a test fixture) whose
//! `olang.lock` no incidental install may create or rewrite. A run and
//! `olang test` resolve as usual — replaying a lock that covers the
//! manifest, resolving in memory otherwise — and leave the lock (or its
//! absence) as it was. An explicit install still writes it; a project
//! without the flag keeps the old behaviour.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// app (depends on ../lib by path) + lib; `policy` goes into app's
/// `[package]` as `lock = "<policy>"`.
fn project(tag: &str, policy: Option<&str>) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_lock_never_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("app")).unwrap();
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(
        dir.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(dir.join("lib/index.ol"), "share fn twice(n) = n * 2\n").unwrap();
    let lock_line = policy
        .map(|p| format!("lock = \"{}\"\n", p))
        .unwrap_or_default();
    fs::write(
        dir.join("app/olang.toml"),
        format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\n{}\n[dependencies.lib]\npath = \"../lib\"\n", lock_line),
    )
    .unwrap();
    fs::write(dir.join("app/main.ol"), "use lib\nprintln(lib.twice(21))\n").unwrap();
    fs::write(
        dir.join("app/twice_test.ol"),
        "use lib\ntest \"twice doubles\" {\n    assert_eq(lib.twice(4), 8)\n}\n",
    )
    .unwrap();
    dir
}

/// A lock that does not cover the manifest: it names a dependency the
/// manifest has dropped, so any install that may write rewrites it.
const STALE: &str = "version = 1\n\n[package.ghost]\ndependencies = []\n\n[package.ghost.source]\nkind = \"path\"\npath = \"../ghost\"\n";

fn olang(app: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(app)
        .env_remove("OLANG_LOCK_READONLY")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "olang {:?} failed:\n{}\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn run_and_test(app: &Path) {
    let run = olang(app, &["main.ol"]);
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "42");
    let test = olang(app, &["test", "."]);
    assert!(
        String::from_utf8_lossy(&test.stdout).contains("1 passed"),
        "{}",
        String::from_utf8_lossy(&test.stdout)
    );
}

#[test]
fn never_without_a_lock_writes_none() {
    let dir = project("none", Some("never"));
    let app = dir.join("app");
    run_and_test(&app);
    assert!(
        !app.join("olang.lock").exists(),
        "a lock = \"never\" project got an olang.lock"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn never_leaves_a_stale_lock_byte_for_byte() {
    let dir = project("stale", Some("never"));
    let app = dir.join("app");
    fs::write(app.join("olang.lock"), STALE).unwrap();
    run_and_test(&app);
    assert_eq!(
        fs::read_to_string(app.join("olang.lock")).unwrap(),
        STALE,
        "a lock = \"never\" project's lock was rewritten"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn an_explicit_install_still_writes_a_never_lock() {
    let dir = project("explicit", Some("never"));
    let app = dir.join("app");
    let incidental = olang::pkg::install(&app, &olang::pkg::InstallOptions::default()).unwrap();
    assert!(incidental.contains_key("lib"));
    assert!(!app.join("olang.lock").exists());
    let opts = olang::pkg::InstallOptions {
        explicit: true,
        ..Default::default()
    };
    olang::pkg::install(&app, &opts).unwrap();
    assert!(
        fs::read_to_string(app.join("olang.lock"))
            .unwrap()
            .contains("[package.lib]")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn without_the_flag_runs_still_write_and_refresh_the_lock() {
    for (tag, policy) in [("default", None), ("auto", Some("auto"))] {
        let dir = project(tag, policy);
        let app = dir.join("app");
        olang(&app, &["main.ol"]);
        assert!(
            app.join("olang.lock").exists(),
            "{tag}: a run writes olang.lock"
        );
        fs::write(app.join("olang.lock"), STALE).unwrap();
        olang(&app, &["test", "."]);
        let lock = fs::read_to_string(app.join("olang.lock")).unwrap();
        assert_ne!(lock, STALE, "{tag}: olang test refreshes a stale lock");
        assert!(
            lock.contains("[package.lib]") && !lock.contains("ghost"),
            "{tag}: {lock}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

#[test]
fn check_warns_on_an_unknown_lock_value() {
    let dir = project("unknown", Some("nevr"));
    let app = dir.join("app");
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["check", "main.ol"])
        .current_dir(&app)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("lock = \"nevr\" is not a lock policy"),
        "{err}"
    );
    assert!(
        out.status.success(),
        "a warning does not fail the check: {err}"
    );
    let known = project("known", Some("never"));
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["check", "main.ol"])
        .current_dir(known.join("app"))
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stderr).contains("lock policy"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&known);
}

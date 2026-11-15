//! `[package] olang` names the oldest olang a project runs on: an older
//! olang warns as it runs the project, and `olang check` refuses a value
//! that is not a version.

use std::process::Command;

fn project(pin: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("olang.toml"),
        format!("[package]\nname = \"pinned\"\nversion = \"0.1.0\"\nolang = \"{pin}\"\nlock = \"never\"\n"),
    )
    .unwrap();
    std::fs::write(dir.path().join("main.ol"), "println(\"ran\")\n").unwrap();
    dir
}

fn olang(dir: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", home.path())
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn an_older_olang_warns_and_runs() {
    let dir = project("99.0");
    let (ok, out, err) = olang(dir.path(), &["main.ol"]);
    assert!(ok, "{err}");
    assert_eq!(out.trim(), "ran");
    assert!(
        err.contains("needs olang 99.0 or newer") && err.contains(olang::VERSION),
        "{err}"
    );
}

#[test]
fn a_satisfied_pin_says_nothing() {
    for pin in ["0.88", "0", "0.1.2"] {
        let dir = project(pin);
        let (ok, out, err) = olang(dir.path(), &["main.ol"]);
        assert!(ok, "{err}");
        assert_eq!(out.trim(), "ran");
        assert!(!err.contains("warning"), "{pin}: {err}");
    }
}

#[test]
fn check_refuses_a_pin_that_is_not_a_version() {
    for pin in ["banana", "^0.88", ">=0.87", "0.88.1.2"] {
        let dir = project(pin);
        let (ok, _, err) = olang(dir.path(), &["check", "."]);
        assert!(!ok, "{pin}: {err}");
        assert!(err.contains("is not a version"), "{pin}: {err}");
    }
    let dir = project("0.88");
    let (ok, _, err) = olang(dir.path(), &["check", "."]);
    assert!(ok, "{err}");
}

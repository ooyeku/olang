//! A package's internals are its own (OL-448): from outside a package
//! only its root module resolves, plus the modules its olang.toml lists
//! under `[package] public`. The run refuses any other with the fix, and
//! `olang check` reports the same thing before the program runs. The
//! package itself still reaches all of its modules by its own name.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// `geo` (index.ol re-exporting `area`, lib/area.ol sharing a helper too)
/// and `app`, which depends on it.
fn workspace(sub: &str, public: &str) -> (PathBuf, PathBuf, PathBuf) {
    let ws = std::env::temp_dir().join(format!("olang_privacy_{}_{}", sub, std::process::id()));
    let _ = fs::remove_dir_all(&ws);
    let (geo, app) = (ws.join("geo"), ws.join("app"));
    write(
        &geo.join("olang.toml"),
        &format!("[package]\nname = \"geo\"\nversion = \"0.1.0\"\n{public}"),
    );
    write(
        &geo.join("lib/area.ol"),
        "share fn area(w, h) = w * h\nshare fn internal_helper(x) = x + 1\n",
    );
    write(&geo.join("index.ol"), "share use lib.area { area }\n");
    write(
        &geo.join("tests/area_test.ol"),
        "use geo.lib.area { internal_helper }\ntest \"inside\" { assert_eq(internal_helper(1), 2) }\n",
    );
    write(
        &app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.geo]\npath = \"../geo\"\n",
    );
    (ws, geo, app)
}

fn run(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(olang())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run olang");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

#[test]
fn an_internal_module_is_refused_from_outside_its_package() {
    let (ws, geo, app) = workspace("refused", "");
    write(
        &app.join("main.ol"),
        "use geo.lib.area { internal_helper }\nprintln(internal_helper(1))\n",
    );
    let (ok, text) = run(&app, &["main.ol"]);
    assert!(!ok, "the run is refused:\n{text}");
    assert!(
        text.contains("'geo.lib.area' is internal to package 'geo'"),
        "{text}"
    );
    assert!(
        text.contains("[package] public"),
        "the fix is named:\n{text}"
    );

    // olang check says it, at the use
    let (ok, text) = run(&app, &["check", "main.ol"]);
    assert!(!ok, "check fails:\n{text}");
    assert!(text.contains("internal to package 'geo'"), "{text}");

    // the root module is public
    write(
        &app.join("main.ol"),
        "use geo { area }\nprintln(area(3, 4))\n",
    );
    let (ok, text) = run(&app, &["main.ol"]);
    assert!(ok && text.contains("12"), "{text}");
    let (ok, text) = run(&app, &["check", "main.ol"]);
    assert!(ok, "{text}");

    // the package reaches its own internals by its name
    let (ok, text) = run(&geo, &["test", "tests"]);
    assert!(ok, "the package's own test imports its internals:\n{text}");
    let _ = fs::remove_dir_all(&ws);
}

#[test]
fn a_module_the_manifest_lists_public_is_importable() {
    for public in ["public = [\"lib.area\"]\n", "public = [\"lib.*\"]\n"] {
        let (ws, _geo, app) = workspace("listed", public);
        write(
            &app.join("main.ol"),
            "use geo.lib.area { internal_helper }\nprintln(internal_helper(1))\n",
        );
        let (ok, text) = run(&app, &["main.ol"]);
        assert!(ok && text.trim() == "2", "{public}: {text}");
        let (ok, text) = run(&app, &["check", "main.ol"]);
        assert!(ok, "{public}: {text}");
        let _ = fs::remove_dir_all(&ws);
    }
}

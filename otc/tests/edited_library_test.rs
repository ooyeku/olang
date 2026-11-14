//! An edited path or shelf library re-pins the lock at the next `otc
//! install`, which says so; `otc install --frozen` fails on the edit
//! (OL-367). Before, both said "olang.lock unchanged" and the lock kept a
//! checksum of contents that were no longer there.
//!
//! Each test has a home and a shelf of its own under the temp directory.

use std::path::{Path, PathBuf};
use std::process::Command;

fn otc(dir: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_otc"))
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", home)
        .env("OLANG_SHELF", home.join("shelf.toml"))
        .env_remove("OLANG_LOCK_READONLY")
        .output()
        .expect("spawn otc");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn sandbox(tag: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!("otc_edited_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("home")).unwrap();
    base
}

fn checksum(lock: &str) -> String {
    lock.lines()
        .find_map(|l| l.trim().strip_prefix("checksum = "))
        .expect("the lock records a checksum")
        .to_string()
}

/// `geo` (a library) and `app2`, which depends on it as `how` says
/// (`shelf`: `otc add geo` after `otc lib add`; `path`: by its path).
fn project(base: &Path, how: &str) -> (PathBuf, PathBuf) {
    let home = base.join("home");
    let (ok, out) = otc(base, &home, &["new", "geo", "--lib"]);
    assert!(ok, "{out}");
    let (ok, out) = otc(base, &home, &["new", "app2"]);
    assert!(ok, "{out}");
    let app = base.join("app2");
    let target = if how == "shelf" {
        let (ok, out) = otc(base, &home, &["lib", "add", "./geo"]);
        assert!(ok, "{out}");
        "geo"
    } else {
        "../geo"
    };
    let (ok, out) = otc(&app, &home, &["add", target]);
    assert!(ok, "{out}");
    (app, home)
}

fn edited_library_re_pins(how: &str) {
    let base = sandbox(how);
    let (app, home) = project(&base, how);
    let before = checksum(&std::fs::read_to_string(app.join("olang.lock")).unwrap());
    let greet = std::fs::read_dir(base.join("geo/lib"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "ol"))
        .expect("the library has a module");
    let text = std::fs::read_to_string(&greet).unwrap();
    std::fs::write(&greet, format!("{text}// edit\n")).unwrap();

    // CI's gate fails on the edit, and leaves the lock as it was.
    let (ok, out) = otc(&app, &home, &["install", "--frozen"]);
    assert!(!ok, "--frozen must fail on an edited library: {out}");
    assert!(out.contains("lockfile is out of date"), "{out}");
    let lock = std::fs::read_to_string(app.join("olang.lock")).unwrap();
    assert_eq!(checksum(&lock), before);

    // A plain install says so and re-pins.
    let (ok, out) = otc(&app, &home, &["install"]);
    assert!(ok, "{out}");
    assert!(out.contains("library 'geo' changed"), "{out}");
    assert!(out.contains("Wrote olang.lock"), "{out}");
    let after = checksum(&std::fs::read_to_string(app.join("olang.lock")).unwrap());
    assert_ne!(after, before, "the lock re-pins the edited library");

    // And from there the lock is true again.
    let (ok, out) = otc(&app, &home, &["install", "--frozen"]);
    assert!(ok, "{out}");
    assert!(out.contains("olang.lock unchanged"), "{out}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn an_edited_shelf_library_re_pins_and_fails_frozen() {
    edited_library_re_pins("shelf");
}

#[test]
fn an_edited_path_library_re_pins_and_fails_frozen() {
    edited_library_re_pins("path");
}

//! olang.lock and shelf.toml carry a format version; a file a newer olang
//! wrote is refused by name, never replayed in part or rewritten without
//! what it holds.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("otc-fmtv-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

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

#[test]
fn a_newer_lock_is_refused_and_left_as_it_was() {
    let home = scratch("lock-home");
    let app = scratch("lock-app");
    std::fs::write(
        app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let newer = "version = 7\n[future]\nkept = true\n";
    std::fs::write(app.join("olang.lock"), newer).unwrap();
    let (ok, out) = otc(&app, &home, &["install"]);
    assert!(!ok, "{out}");
    assert!(out.contains("olang.lock is format version 7"), "{out}");
    assert_eq!(
        std::fs::read_to_string(app.join("olang.lock")).unwrap(),
        newer
    );
    let _ = std::fs::remove_dir_all(&app);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_shelf_says_its_version_and_a_newer_one_is_refused() {
    let home = scratch("shelf-home");
    let lib = scratch("shelf-lib");
    std::fs::write(lib.join("index.ol"), "share fn one() = 1\n").unwrap();
    std::fs::write(
        lib.join("olang.toml"),
        "[package]\nname = \"one\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    let (ok, out) = otc(&lib, &home, &["lib", "add", "."]);
    assert!(ok, "{out}");
    let shelf = std::fs::read_to_string(home.join("shelf.toml")).unwrap();
    assert!(shelf.starts_with("version = 1\n"), "{shelf}");
    let newer = "version = 3\n[libraries]\n";
    std::fs::write(home.join("shelf.toml"), newer).unwrap();
    let (ok, out) = otc(&lib, &home, &["lib", "list"]);
    assert!(!ok, "{out}");
    assert!(out.contains("format version 3"), "{out}");
    assert_eq!(
        std::fs::read_to_string(home.join("shelf.toml")).unwrap(),
        newer
    );
    let _ = std::fs::remove_dir_all(&lib);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_lists_answer_as_data() {
    let home = scratch("lists-home");
    let app = scratch("lists-app");
    std::fs::create_dir_all(app.join("lib")).unwrap();
    std::fs::write(app.join("lib/index.ol"), "share fn one() = 1\n").unwrap();
    std::fs::write(
        app.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        app.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nlib = { path = \"lib\" }\n",
    )
    .unwrap();
    let (ok, out) = otc(&app, &home, &["list", "--format", "json"]);
    assert!(ok, "{out}");
    let d: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(d["format"], 1);
    assert_eq!(d["kind"], "deps");
    assert_eq!(d["dependencies"][0]["name"], "lib");
    assert_eq!(d["dependencies"][0]["source"], "path");
    assert!(
        d["dependencies"][0]["dir"]
            .as_str()
            .unwrap()
            .starts_with('/')
    );
    let (ok, out) = otc(&app, &home, &["lib", "list", "--format", "json"]);
    assert!(ok, "{out}");
    let d: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(d["kind"], "shelf");
    assert!(d["libraries"].is_array());
    let (ok, out) = otc(&app, &home, &["toolchain", "list", "--format", "json"]);
    assert!(ok, "{out}");
    let d: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(d["kind"], "toolchains");
    let (ok, _) = otc(&app, &home, &["list", "--format", "yaml"]);
    assert!(!ok);
    let _ = std::fs::remove_dir_all(&app);
    let _ = std::fs::remove_dir_all(&home);
}

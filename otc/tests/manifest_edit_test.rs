//! `otc add` and `otc remove` edit olang.toml in place: its comments, its
//! order and the keys olang does not know stay as they were.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("otc-edit-{}-{}", name, std::process::id()));
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
fn add_and_remove_keep_the_manifests_comments_and_unknown_keys() {
    let home = scratch("home");
    let root = scratch("root");
    let lib = root.join("lib");
    let app = root.join("app");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        lib.join("olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(lib.join("index.ol"), "share fn one() = 1\n").unwrap();
    let before = "# the app\n[package]\nname = \"app\"   # its name\nversion = \"0.1.0\"\n\n[dependencies]\n# none yet\n\n[extra]\nkept = true\n";
    std::fs::write(app.join("olang.toml"), before).unwrap();

    let (ok, out) = otc(&app, &home, &["add", "../lib"]);
    assert!(ok, "{out}");
    let added = std::fs::read_to_string(app.join("olang.toml")).unwrap();
    assert!(added.starts_with("# the app\n[package]\nname = \"app\"   # its name\n"), "{added}");
    assert!(added.contains("lib = { path = \"../lib\" }"), "{added}");
    assert!(added.contains("# none yet") && added.contains("[extra]\nkept = true"), "{added}");

    let (ok, out) = otc(&app, &home, &["remove", "lib"]);
    assert!(ok, "{out}");
    let removed = std::fs::read_to_string(app.join("olang.toml")).unwrap();
    assert!(!removed.contains("lib = "), "{removed}");
    assert!(removed.contains("# the app") && removed.contains("[extra]\nkept = true"), "{removed}");
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&home);
}

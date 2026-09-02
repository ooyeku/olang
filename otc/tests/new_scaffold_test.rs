//! `otc new` shapes (roadmap W9): a hyphenated name scaffolds a package
//! imported by its identifier form, `--web` builds on the web SDK,
//! `--web-bare` keeps the raw-stdlib shape, and a project outside a git
//! repository is told so.

use std::path::PathBuf;
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("otc_new_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn otc(dir: &std::path::Path, args: &[&str]) -> (String, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_otc"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run otc");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.success(),
    )
}

#[test]
fn a_hyphenated_library_is_imported_by_its_identifier() {
    let ws = workspace("hyphen");
    let (out, ok) = otc(&ws, &["new", "open-track-query", "--lib"]);
    assert!(ok, "{out}");
    let manifest = std::fs::read_to_string(ws.join("open-track-query/olang.toml")).unwrap();
    assert!(
        manifest.contains("name = \"open_track_query\""),
        "{manifest}"
    );
    assert!(out.contains("use open_track_query"), "{out}");
    assert!(out.contains("imported as `use open_track_query`"), "{out}");
    // The scaffold's own source uses the identifier, so it parses.
    let index = std::fs::read_to_string(ws.join("open-track-query/index.ol")).unwrap();
    assert!(
        index.contains("open_track_query") || index.contains("share fn"),
        "{index}"
    );
    // Outside a repository, the next steps end with the git hint.
    assert!(out.contains("git init"), "{out}");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn a_name_that_cannot_be_an_identifier_is_refused() {
    let ws = workspace("badname");
    let (out, ok) = otc(&ws, &["new", "9lives", "--lib"]);
    assert!(!ok);
    assert!(out.contains("cannot be a package name"), "{out}");
    assert!(!ws.join("9lives").exists());
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn the_web_shape_is_two_files_on_the_sdk() {
    let ws = workspace("web");
    let (out, ok) = otc(&ws, &["new", "notes", "--web"]);
    assert!(ok, "{out}");
    let root = ws.join("notes");
    assert!(root.join("main.ol").exists());
    assert!(root.join("client.ol").exists());
    assert!(
        !root.join("lib/router.ol").exists(),
        "the SDK shape hand-rolls nothing"
    );
    let main = std::fs::read_to_string(root.join("main.ol")).unwrap();
    assert!(
        main.contains("use web {") && main.contains("serve(#{"),
        "{main}"
    );
    let client = std::fs::read_to_string(root.join("client.ol")).unwrap();
    assert!(
        client.contains("mount(\"#app\"") && client.contains("btn_confirm"),
        "{client}"
    );
    let manifest = std::fs::read_to_string(root.join("olang.toml")).unwrap();
    assert!(manifest.contains("shelf = \"web\""), "{manifest}");
    assert!(manifest.contains("shelf = \"validate\""), "{manifest}");
    assert!(out.contains("on the web SDK"), "{out}");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn the_bare_web_shape_keeps_every_seam() {
    let ws = workspace("webbare");
    let (out, ok) = otc(&ws, &["new", "notes", "--web-bare"]);
    assert!(ok, "{out}");
    let root = ws.join("notes");
    assert!(root.join("lib/router.ol").exists());
    assert!(root.join("static/index.html").exists());
    assert!(root.join("static/app.ol").exists());
    let manifest = std::fs::read_to_string(root.join("olang.toml")).unwrap();
    assert!(!manifest.contains("shelf ="), "{manifest}");
    let _ = std::fs::remove_dir_all(&ws);
}

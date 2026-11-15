//! A diagnostic's `file:line:col` header counts characters, as
//! `olang check --format json` and the language server do (OL-371). It
//! counted bytes, so a line with `é` or `🎉` before the error put the header
//! to the right of it.

use std::process::Command;

fn olang(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run olang");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn workspace(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_cols_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn check_text_and_json_agree_on_the_column() {
    let ws = workspace("check");
    std::fs::write(
        ws.join("n1.ol"),
        "fn add(a, b) = a + b\nlet x = 1\nprintln(\"héllo wörld 🎉\"); let y = undefined_thing + 1\n",
    )
    .unwrap();
    // `undefined_thing` starts at character 35 of line 3 (byte 40)
    let json = olang(&ws, &["check", "--format", "json", "n1.ol"]);
    assert!(json.contains("\"column\":35"), "{json}");
    let text = olang(&ws, &["check", "n1.ol"]);
    assert!(text.contains("n1.ol:3:35"), "{text}");
    assert!(!text.contains("n1.ol:3:40"), "{text}");
}

#[test]
fn a_runtime_errors_header_counts_characters() {
    let ws = workspace("run");
    std::fs::write(ws.join("r.ol"), "let s = \"é🎉\"; println(gone_name)\n").unwrap();
    // the statement `println(...)` starts at character 15 (byte 19) and
    // `gone_name` at character 23 (byte 27): the header names one of the two
    // characters, never a byte
    let out = olang(&ws, &["r.ol"]);
    assert!(
        out.contains("r.ol:1:15") || out.contains("r.ol:1:23"),
        "{out}"
    );
}

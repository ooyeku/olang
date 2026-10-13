//! Every JSON output carries `format` and `kind`, and its paths are
//! absolute; `--format json` on `inspect` and `caps`; the bench baseline
//! is versioned; `-o/--output` and `--format` with the older spellings
//! kept as aliases.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_json_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("olang.toml"),
        "[package]\nname = \"j\"\nversion = \"0.1.0\"\n\n[capabilities]\nnet = false\nfs = \"read\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("a.ol"), "/// one\nfn one() = 1\nprintln(one())\n").unwrap();
    dir
}

fn olang(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", dir.join(".home"))
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn doc(text: &str) -> serde_json::Value {
    serde_json::from_str(text.lines().next().unwrap_or(""))
        .unwrap_or_else(|e| panic!("{e}: {text}"))
}

#[test]
fn caps_and_inspect_answer_as_data() {
    let ws = workspace("caps");
    let (out, code) = olang(&ws, &["caps", "--format", "json"]);
    assert_eq!(code, 0, "{out}");
    let d = doc(&out);
    assert_eq!(d["format"], 1);
    assert_eq!(d["kind"], "caps");
    assert_eq!(d["grant"]["fs"], "read");
    assert_eq!(d["grant"]["net"], false);
    assert_eq!(d["declared"], true);
    assert!(d["manifest"].as_str().unwrap().starts_with('/'), "{out}");
    let (out, code) = olang(&ws, &["build", "a.ol", "-o", "abin"]);
    assert_eq!(code, 0, "{out}");
    let (out, code) = olang(&ws, &["inspect", "abin", "--format", "json", "--verify"]);
    assert_eq!(code, 0, "{out}");
    let d = doc(&out);
    assert_eq!(d["kind"], "inspect");
    assert_eq!(d["verified"], true);
    assert_eq!(d["checks"]["sha256"], true);
    assert_eq!(d["capabilities"]["net"], false);
    assert!(d["binary"].as_str().unwrap().starts_with('/'));
    let (out, _) = olang(&ws, &["inspect", "abin", "--caps", "--format", "json"]);
    assert_eq!(doc(&out)["grant"]["fs"], "read");
    let (_, code) = olang(&ws, &["caps", "--format", "yaml"]);
    assert_eq!(code, 2);
}

#[test]
fn check_writes_absolute_paths_and_tier_check_its_own_format() {
    let ws = workspace("check");
    let (out, _) = olang(&ws, &["check", "--format", "json", "a.ol"]);
    let d = doc(&out);
    assert!(
        d["files"][0]["file"].as_str().unwrap().starts_with('/'),
        "{out}"
    );
    let (out, _) = olang(&ws, &["check", "--tier", "--format", "json", "a.ol"]);
    let d = doc(&out);
    assert_eq!(d["kind"], "tier-check");
    assert_eq!(d["format"], 1);
}

#[test]
fn the_bench_events_and_baseline_are_versioned() {
    let ws = workspace("bench");
    let (out, code) = olang(
        &ws,
        &[
            "bench",
            "a.ol",
            "--runs",
            "1",
            "--format",
            "json",
            "--save",
            "base.json",
        ],
    );
    assert_eq!(code, 0, "{out}");
    let start = doc(&out);
    assert_eq!(start["event"], "start");
    assert_eq!(start["format"], 1);
    assert_eq!(start["kind"], "bench");
    assert!(
        start["files"][0].as_str().unwrap().starts_with('/'),
        "{out}"
    );
    let base: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ws.join("base.json")).unwrap()).unwrap();
    assert_eq!(base["format"], 1);
    assert_eq!(base["kind"], "bench-baseline");
    // a baseline from before the field reads as format 1; a newer one is refused
    std::fs::write(
        ws.join("old.json"),
        "{\"benchmarks\":{\"a\":{\"median_s\":1.0}}}",
    )
    .unwrap();
    let (out, code) = olang(
        &ws,
        &["bench", "a.ol", "--runs", "1", "--against", "old.json"],
    );
    assert_eq!(code, 0, "{out}");
    std::fs::write(
        ws.join("new.json"),
        "{\"format\":9,\"benchmarks\":{\"a\":{\"median_s\":1.0}}}",
    )
    .unwrap();
    let (_, code) = olang(
        &ws,
        &["bench", "a.ol", "--runs", "1", "--against", "new.json"],
    );
    assert_eq!(code, 2);
    let (help, _) = olang(&ws, &["bench", "--help"]);
    for flag in ["--in-task", "--format", "--profile", "--save", "--against"] {
        assert!(help.contains(flag), "{flag}: {help}");
    }
}

#[test]
fn output_and_format_flags_keep_their_old_spellings() {
    let ws = workspace("flags");
    let (out, code) = olang(
        &ws,
        &["doc", "a.ol", "--format", "markdown", "-o", "api.md"],
    );
    assert_eq!(code, 0, "{out}");
    assert!(
        std::fs::read_to_string(ws.join("api.md"))
            .unwrap()
            .contains("one")
    );
    let (out, code) = olang(&ws, &["doc", "a.ol", "--markdown", "-o", "api2.md"]);
    assert_eq!(code, 0, "{out}");
    assert!(ws.join("api2.md").exists());
    for flag in ["--output", "--out", "-o"] {
        let path = format!("p{}.json", flag.len());
        let (out, code) = olang(&ws, &["profile", "a.ol", "--format", "json", flag, &path]);
        assert_eq!(code, 0, "{flag}: {out}");
        let d: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ws.join(&path)).unwrap()).unwrap();
        assert_eq!(d["kind"], "profile");
    }
}

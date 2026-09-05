//! The rest of the board (roadmap W9/W10/W11 leftovers): snapshot tests,
//! `olang test --watch`'s flag, `olang check --fix`, the `vec` module and
//! `ods.to_matrix`, `ods.date_part`/`split`/`split_at`, imported functions
//! inside meta fn bodies, one "Runtime error:" prefix, and a compiled
//! frame's error placed in its own file.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w12_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn olang(dir: &Path, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .env_remove("OLANG_UPDATE_SNAPSHOTS")
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn snapshots_record_then_compare_then_refresh() {
    let ws = workspace("snap");
    write(
        &ws,
        "s.ol",
        "fn card(n) = \"<div class=\\\"card\\\">\" + to_string(n) + \"</div>\"\n\
         test \"the card\" { testing.snapshot(\"card\", card(1)) }\n",
    );
    // First run records.
    let (out, err, rc) = olang(&ws, &["test", "s.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    let snap = ws.join("__snapshots__/card.snap");
    // The display form of a string: quoted, its own quotes as written.
    assert_eq!(
        std::fs::read_to_string(&snap).unwrap(),
        "\"<div class=\"card\">1</div>\""
    );
    // Same output passes.
    let (out, _, rc) = olang(&ws, &["test", "s.ol"]);
    assert_eq!(rc, 0, "{out}");
    // A change fails, naming both forms.
    write(
        &ws,
        "s.ol",
        "fn card(n) = \"<div class=\\\"card\\\">\" + to_string(n + 1) + \"</div>\"\n\
         test \"the card\" { testing.snapshot(\"card\", card(1)) }\n",
    );
    let (out, err, rc) = olang(&ws, &["test", "s.ol"]);
    assert_ne!(rc, 0, "{out}{err}");
    let all = out + &err;
    assert!(all.contains("snapshot 'card' changed"), "{all}");
    assert!(all.contains("OLANG_UPDATE_SNAPSHOTS"), "{all}");
    // Refreshing accepts the new form.
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["test", "s.ol"])
        .current_dir(&ws)
        .env("OLANG_UPDATE_SNAPSHOTS", "1")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(std::fs::read_to_string(&snap).unwrap().contains(">2<"));
}

#[test]
fn test_watch_is_a_flag_and_check_fix_rewrites_template_escapes() {
    let ws = workspace("fix");
    let (out, _, _) = olang(&ws, &["test", "--help"]);
    assert!(out.contains("--watch"), "{out}");
    write(
        &ws,
        "t.ol",
        "let name = \"x\"\nprintln(`a\\nb ${name}\\tc`)\nprintln(`plain ${name}`)\n",
    );
    let (out, err, _) = olang(&ws, &["check", "--fix", "t.ol"]);
    let all = out + &err;
    assert!(all.contains("fixed 2 template escapes"), "{all}");
    let fixed = std::fs::read_to_string(ws.join("t.ol")).unwrap();
    assert!(
        fixed.contains("`a${\"\\n\"}b ${name}${\"\\t\"}c`"),
        "{fixed}"
    );
    // The rewrite means what the warning said: real control characters.
    let (out, err, rc) = olang(&ws, &["run", "t.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "a\nb x\tc\nplain x\n");
    // Nothing left to warn about.
    let (out, err, _) = olang(&ws, &["check", "t.ol"]);
    assert!(
        !(out + &err).contains("backtick template"),
        "warning remains"
    );
}

#[test]
fn vec_and_to_matrix_and_the_ods_splits() {
    let ws = workspace("vec");
    write(
        &ws,
        "v.ol",
        "println(to_string(vec.dot([1.0, 2.0], [3.0, 4.0])))\n\
         println(to_string(vec.add([1, 2], [0.5, 0.5])))\n\
         println(to_string(vec.scale([1.0, 2.0], -2.0)))\n\
         println(to_string(vec.norm([3.0, 4.0])))\n\
         println(to_string(vec.mean([1, 2, 3])))\n\
         let f = ods.frame([[\"a\", [1, 2, 3, 4]], [\"b\", [2.0, 4.0, 6.0, 8.0]], [\"d\", [\"2026-01-03\", \"2026-01-09\", \"2026-02-01\", \"2026-03-15\"]]])\n\
         let m = ods.to_matrix(f, [\"a\", \"b\"])\n\
         println(to_string(vec.dot(m[0], m[1])))\n\
         println(to_string(ods.to_list(ods.date_part(f[\"d\"], \"month\"))))\n\
         random.seed(3)\n\
         let parts = ods.split(f, 0.5)\n\
         println(to_string(ods.n_rows(parts[0]) + ods.n_rows(parts[1])) + \" \" + to_string(ods.n_rows(parts[0])))\n\
         let chrono = ods.split_at(f, \"d\", 0.75)\n\
         println(to_string(ods.to_list(chrono[0][\"d\"])))\n\
         println(to_string(ods.to_list(chrono[1][\"d\"])))\n\
         match vec.dot([1.0], [1.0, 2.0]) { v => println(\"unreachable\") }",
    );
    let (out, err, rc) = olang(&ws, &["run", "v.ol"]);
    assert_ne!(rc, 0, "{out}{err}");
    assert_eq!(
        out,
        "11.0\n[1.5, 2.5]\n[-2.0, -4.0]\n5.0\n2.0\n60.0\n[\"2026-01\", \"2026-01\", \"2026-02\", \"2026-03\"]\n4 2\n[\"2026-01-03\", \"2026-01-09\", \"2026-02-01\"]\n[\"2026-03-15\"]\n"
    );
    assert!(err.contains("vec.dot: lengths differ (1 and 2)"), "{err}");
}

#[test]
fn a_meta_fn_body_may_call_an_imported_share_fn() {
    let ws = workspace("metaimport");
    write(&ws, "lib/rules.ol", "share fn shout(s) = str.to_upper(s)\n");
    write(
        &ws,
        "m.ol",
        "use lib.rules { shout }\n\
         meta fn loud(e) = `\"${shout(unwrap(meta.eval(e)))}\"`\n\
         println(@loud(\"hi\"))\n\
         println(shout(\"bye\"))\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "m.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert_eq!(out, "HI\nBYE\n");
}

#[test]
fn a_runtime_error_carries_one_prefix_and_a_compiled_frame_names_its_file() {
    let ws = workspace("prefix");
    write(
        &ws,
        "lib/deep.ol",
        "share fn hot(n) = {\n    let mut acc = 0\n    for i in range(0, n) { acc = acc + i }\n    acc\n}\n\
         share fn boom(n) = hot(n) + map_get(5, \"k\")\n",
    );
    write(
        &ws,
        "p.ol",
        "use lib.deep { boom, hot }\n\
         let mut warm = 0\n\
         for i in range(0, 50) { warm = warm + hot(10) }\n\
         let t = spawn { boom(3) }\n\
         match task.join(t) { Err(e) => println(e), v => println(\"no\") }\n\
         boom(3)\n",
    );
    let (out, err, rc) = olang(&ws, &["run", "p.ol"]);
    assert_ne!(rc, 0, "{out}{err}");
    // The joined task's error text carries the prefix once.
    let first = out.lines().next().unwrap_or("");
    assert_eq!(
        first
            .matches("Runtime error:")
            .count()
            .max(first.matches("Type error:").count()),
        1,
        "{first}"
    );
    assert!(!err.contains("Runtime error: Runtime error:"), "{err}");
    // The failing statement is placed in the dependency's file.
    assert!(err.contains("deep.ol"), "{err}");
}

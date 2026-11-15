//! `// allow: <class>[, <class>…]` (docs/tooling.md "Allowing one line"):
//! at the end of a line, or on a line of its own above it (comment lines
//! between), it silences that line's advisories of the classes named —
//! in `olang check`'s text and JSON, a notebook's cells, `--rules` rules
//! (by a rule's name), `--tier` (on the `fn` line). An error is never
//! silenced; an allow wins over `[check] promote`; a name that is no
//! class is a warning of its own. (The server's agreement is in
//! tests/lsp_check_agreement_test.rs; `olang fmt` keeping the comments
//! in place is here.)

use std::path::{Path, PathBuf};
use std::process::Command;

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang-allow-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
    }
    dir.canonicalize().unwrap()
}

fn toml(extra: &str) -> String {
    format!("[package]\nname = \"t\"\nversion = \"0.1.0\"\nauthors = []\n{extra}")
}

/// `olang <args>` in `dir`: its exit code and its output, both streams.
fn olang(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run olang");
    (
        out.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// `olang check --format json FILE`: (exit, [(line, code, severity)]).
fn json(dir: &Path, file: &str) -> (i32, Vec<(u64, String, String)>, serde_json::Value) {
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["check", "--format", "json", file])
        .current_dir(dir)
        .output()
        .expect("run olang check --format json");
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let ds = doc["files"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["line"].as_u64().unwrap(),
                d["code"].as_str().unwrap_or("").to_string(),
                d["severity"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    (out.status.code().unwrap_or(-1), ds, doc)
}

const MAIN: &str = "\
fn f(x) = {
    x == 3 // allow: no_effect
    // allow: no_effect
    // the label a reader sees
    \"label\"
    x * 2

    // allow: no_effect

    \"apart\"
    x
}
let cell = 2 // allow: shadow, no_effect
let keep = 1 // allow: all
keep
keep // allow: no_efect
println(f(1) + cell)
";

#[test]
fn trailing_above_multiple_all_and_unknown() {
    let dir = project("basic", &[("olang.toml", &toml("")), ("main.ol", MAIN)]);
    let (code, ds, doc) = json(&dir, "main.ol");
    assert_eq!(code, 0, "{doc}");
    let lines = |c: &str| -> Vec<u64> { ds.iter().filter(|d| d.1 == c).map(|d| d.0).collect() };
    // line 2 trailing, line 5 above (a comment line between): silenced;
    // line 6 not allowed; line 10 below a blank line: not allowed;
    // line 15 has no allow; line 16's names no class
    assert_eq!(lines("no_effect"), vec![6, 10, 15, 16], "{doc}");
    assert!(lines("shadow").is_empty(), "{doc}");
    assert_eq!(lines("allow"), vec![16], "{doc}");
    let unknown = doc["files"][0]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "allow")
        .unwrap();
    assert_eq!(unknown["severity"], "warning");
    assert_eq!(unknown["column"], 16, "at the name: {unknown}");
    assert!(
        unknown["message"]
            .as_str()
            .unwrap()
            .contains("did you mean `no_effect`"),
        "{unknown}"
    );
    // the text form says the same
    let (code, text) = olang(&dir, &["check", "main.ol"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("1 file clean, 5 warnings"), "{text}");
    assert!(text.contains("`// allow:` names `no_efect`"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_error_is_never_silenced() {
    let dir = project(
        "errors",
        &[
            ("olang.toml", &toml("")),
            (
                "main.ol",
                "fn f(x: Int) -> Int = x + 1\nlet e = nowhere(1) // allow: all\nlet fixed = 1\n// allow: all\nfixed = 2\nprintln(f(\"a\")) // allow: all\nprintln(e)\n",
            ),
        ],
    );
    let (code, ds, doc) = json(&dir, "main.ol");
    assert_eq!(code, 1, "{doc}");
    let errors: Vec<u64> = ds.iter().filter(|d| d.2 == "error").map(|d| d.0).collect();
    assert_eq!(errors, vec![2, 5, 6], "{doc}");
    let (code, text) = olang(&dir, &["check", "main.ol"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("3 problems found"), "{text}");
    // a line that does not parse is a syntax error, allow or not
    std::fs::write(dir.join("bad.ol"), "let y = // allow: all\n").unwrap();
    let (code, ds, _) = json(&dir, "bad.ol");
    assert_eq!(code, 1);
    assert_eq!(ds[0].1, "syntax");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_allow_wins_over_promote() {
    let dir = project(
        "promote",
        &[
            (
                "olang.toml",
                &toml("[check]\npromote = [\"no_effect\", \"shadow\"]\n"),
            ),
            (
                "main.ol",
                "let x = 1\nx // allow: no_effect\nlet cell = 3 // allow: all\nprintln(x + cell)\n",
            ),
        ],
    );
    let (code, text) = olang(&dir, &["check", "main.ol"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("1 file clean"), "{text}");
    // the same line without its allow: an error, as promoted
    std::fs::write(dir.join("main.ol"), "let x = 1\nx\nprintln(x)\n").unwrap();
    let (code, text) = olang(&dir, &["check", "main.ol"]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("1 problem found"), "{text}");
    // an unknown name's warning is never promoted, even by `all`
    std::fs::write(
        dir.join("olang.toml"),
        toml("[check]\npromote = [\"all\"]\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("main.ol"),
        "let x = 1 // allow: nothing_such\nprintln(x)\n",
    )
    .unwrap();
    let (code, ds, doc) = json(&dir, "main.ol");
    assert_eq!(code, 0, "{doc}");
    assert_eq!(
        ds,
        vec![(1, "allow".to_string(), "warning".to_string())],
        "{doc}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_notebook_cell() {
    let nb = "# Cells\n\n```olang\nlet a = 2\na // allow: no_effect\n// allow: no_effect\na + 1\na * 3\na\n```\n\nProse.\n\n```olang\nlet b = a\nb // allow: no_effect\nb + 1\n```\n";
    let dir = project("notebook", &[("olang.toml", &toml("")), ("cells.olmd", nb)]);
    let (code, ds, doc) = json(&dir, "cells.olmd");
    assert_eq!(code, 0, "{doc}");
    let lines: Vec<u64> = ds
        .iter()
        .filter(|d| d.1 == "no_effect")
        .map(|d| d.0)
        .collect();
    // lines 5 and 7 allowed, 9 and 17 are cells' values, 8 is reported
    assert_eq!(lines, vec![8], "{doc}");
    let (code, text) = olang(&dir, &["check", "cells.olmd"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("1 file clean, 1 warning"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_rule_is_allowed_by_its_name() {
    let rules = "share fn rule_no_print(nodes) = map(filter(nodes, (n) => map_get(n, \"kind\") == \"call\" && map_get(n, \"target\") == \"println\"), (n) => #{ \"message\": \"no println\", \"line\": map_get(n, \"line\") })\n";
    let dir = project(
        "rules",
        &[
            ("olang.toml", &toml("")),
            ("rules.ol", rules),
            (
                "main.ol",
                "println(1) // allow: rule_no_print\n// allow: rule_no_print\nprintln(2)\nprintln(3) // allow: all\nprintln(4)\n",
            ),
        ],
    );
    let (code, text) = olang(&dir, &["check", "--rules", "rules.ol", "main.ol"]);
    assert_eq!(code, 1, "{text}");
    let found: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("[rule_no_print]"))
        .collect();
    // `all` is the checker's classes: a rule is allowed only by its name
    assert_eq!(found.len(), 2, "{text}");
    assert!(
        found[0].contains("main.ol:4:") && found[1].contains("main.ol:5:"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tier_refusal_on_an_allowed_fn_line() {
    let main = "\
// allow: tier
fn doubled_later(n) = {
    let t = spawn (n * 2)
    unwrap(task.join(t))
}

// studio: native
fn pinned_later(n) = { // allow: tier
    let t = spawn (n * 2)
    unwrap(task.join(t))
}

fn other_later(n) = {
    let t = spawn (n * 2)
    unwrap(task.join(t))
}
println(doubled_later(1) + pinned_later(1) + other_later(1))
";
    let dir = project(
        "tier",
        &[
            ("olang.toml", &toml("[check]\npromote = [\"tier\"]\n")),
            ("main.ol", main),
        ],
    );
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["check", "--tier", "--format", "json", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let fns = doc["files"][0]["functions"].as_array().unwrap();
    let row = |n: &str| fns.iter().find(|f| f["name"] == n).unwrap().clone();
    assert_eq!(row("doubled_later")["verdict"], "refused");
    assert_eq!(row("doubled_later")["allowed"], true, "{doc}");
    assert_eq!(row("doubled_later")["error"], false, "{doc}");
    // a pin says it must compile: the allow cannot unsay it
    assert_eq!(row("pinned_later")["allowed"], false, "{doc}");
    assert_eq!(row("pinned_later")["error"], true, "{doc}");
    assert_eq!(row("other_later")["error"], true, "{doc}");
    assert_eq!(doc["refused"], 2, "{doc}");
    assert_eq!(doc["errors"], 2, "{doc}");
    let (_, text) = olang(&dir, &["check", "--tier", "main.ol"]);
    assert!(!text.contains("`doubled_later`"), "{text}");
    assert!(text.contains("`other_later`"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_keeps_allow_comments_in_place() {
    let src = "fn f(x) = {\n    x == 3 // allow: no_effect\n    // allow: no_effect\n    // why\n    \"label\"\n    x\n}\n\"logo.png\" // allow: no_effect\nprintln(f(1))\n";
    let dir = project("fmt", &[("olang.toml", &toml("")), ("main.ol", src)]);
    let (code, text) = olang(&dir, &["fmt", "main.ol"]);
    assert_eq!(code, 0, "{text}");
    let after = std::fs::read_to_string(dir.join("main.ol")).unwrap();
    assert_eq!(after, src, "fmt moved an allow");
    let (code, ds, doc) = json(&dir, "main.ol");
    assert_eq!(code, 0);
    assert!(ds.is_empty(), "{doc}");
    let _ = std::fs::remove_dir_all(&dir);
}

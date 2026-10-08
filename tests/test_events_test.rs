//! `olang test --format json` over the real binary (docs/tooling.md): the
//! runner's events as line-delimited JSON — the run, each block started
//! and its outcome, a failing `assert_eq`'s expected and actual values and
//! the assertion's place, an error's frames, a pixel snapshot that
//! differs, a block `--only` left out, a file's end, the finish — with
//! several paths in one run, with `--verify-tiers` (a divergence named),
//! and the human output unchanged without the flag.

use serde_json::{Value as J, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang-test-events-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("olang.toml"), "[package]\nname = \"t\"\nversion = \"0.1.0\"\nauthors = []\n").unwrap();
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    dir.canonicalize().unwrap()
}

fn olang(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_olang"));
    c.args(args).current_dir(dir).env_remove("OLANG_VERIFY_TIERS");
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("run olang");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The events of a run's output, and the lines that are not events.
fn events(out: &str) -> (Vec<J>, Vec<String>) {
    let mut evs = Vec::new();
    let mut other = Vec::new();
    for line in out.lines() {
        match line.find("{\"").and_then(|at| serde_json::from_str::<J>(&line[at..]).ok()) {
            Some(j) if j.get("event").is_some() => evs.push(j),
            _ => other.push(line.to_string()),
        }
    }
    (evs, other)
}

fn of<'a>(evs: &'a [J], event: &str, name: &str) -> &'a J {
    evs.iter()
        .find(|e| e["event"] == json!(event) && e["name"] == json!(name))
        .unwrap_or_else(|| panic!("no {event} event for {name}: {evs:?}"))
}

const CALC: &str = r#"share fn total(xs) = fold(xs, 0, (a, x) => a + x)

share fn cents(s) = {
    let parts = str.split(s, ".")
    to_int(parts[0]) * 100 + to_int(parts[1])
}

share fn checkout(prices) = total(map(prices, (p) => cents(p)))
"#;

const TESTS: &str = r#"use lib.calc { total, checkout }

test "adds" {
    assert_eq(total([1, 2, 3]), 6)
}

test "off by one" {
    let got = total([10, 20, 12])
    assert_eq(got, 41)
}

test "a receipt" {
    assert_eq("tea 3\ncake 4", "tea 3\ncake 5")
}

test "a record" {
    assert_eq(#{ "n": 3, "names": ["a", "b"] }, #{ "n": 3, "names": ["a", "c"] })
}

test "bad prices" {
    println("checking out")
    let c = checkout(["3.50", "two"])
}

test "pixels" {
    let _s = testing.snapshot_failed(#{ "name": "card", "baseline": "./__snapshots__/card.png", "actual": "./__snapshots__/card.actual.png",
                                         "differing": 50, "total": 19800, "worst": 231 })
    assert(false, "the snapshot \"card\" changed")
}
"#;

#[test]
fn a_run_says_each_block_and_its_outcome() {
    let dir = project("run", &[("lib/calc.ol", CALC), ("tests/calc_test.ol", TESTS)]);
    let (code, out, err) = olang(&dir, &["test", "--format", "json", "tests/calc_test.ol"], &[]);
    assert_eq!(code, 1, "{err}");
    let (evs, printed) = events(&out);
    let file = dir.join("tests/calc_test.ol").display().to_string();
    assert_eq!(evs[0]["event"], json!("run"));
    assert_eq!(evs[0]["format"], json!(1));
    assert_eq!(evs[0]["files"], json!([file]));
    assert_eq!(evs[0]["count"], json!(6));
    // each block: started, then its outcome, with its line (1-based)
    assert_eq!(of(&evs, "test", "adds")["line"], json!(3));
    assert_eq!(of(&evs, "passed", "adds")["file"], json!(file));
    assert!(of(&evs, "passed", "adds")["ms"].as_f64().is_some());
    // assert_eq: the values, and where the assertion is
    let off = of(&evs, "failed", "off by one");
    assert_eq!(off["kind"], json!("assert_eq"));
    assert_eq!(off["expected"]["show"], json!("41"));
    assert_eq!(off["actual"]["show"], json!("42"));
    assert_eq!(off["actual"]["type"], json!("Int"));
    assert_eq!(off["at"]["line"], json!(9));
    assert_eq!(off["at"]["file"], json!(file));
    // a string keeps its text; a structure its value and a form a line an entry
    let receipt = of(&evs, "failed", "a receipt");
    assert_eq!(receipt["expected"]["string"], json!("tea 3\ncake 5"));
    assert_eq!(receipt["actual"]["string"], json!("tea 3\ncake 4"));
    let record = of(&evs, "failed", "a record");
    assert_eq!(record["expected"]["value"], json!({ "n": 3, "names": ["a", "c"] }));
    assert_eq!(record["actual"]["type"], json!("Map"));
    // an error: its message, its place in the module, the frames above it
    let bad = of(&evs, "failed", "bad prices");
    assert_eq!(bad["kind"], json!("error"));
    assert!(bad["message"].as_str().unwrap().contains("two"), "{bad}");
    assert_eq!(bad["at"]["file"], json!(dir.join("lib/calc.ol").display().to_string()));
    assert_eq!(bad["at"]["line"], json!(5));
    let frames: Vec<&str> = bad["frames"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(frames.first(), Some(&"cents"), "{bad}");
    assert!(frames.contains(&"checkout"), "{bad}");
    assert_eq!(bad["frames"][0]["line"], json!(3));
    // what the program prints stays output, between the events
    assert!(printed.iter().any(|l| l == "checking out"), "{printed:?}");
    // a pixel snapshot: its own event, and the block's failure carries it
    let snap = evs.iter().find(|e| e["event"] == json!("snapshot")).expect("a snapshot event");
    assert_eq!(snap["name"], json!("pixels"));
    assert_eq!(snap["snapshot"]["differing"], json!(50));
    assert_eq!(snap["snapshot"]["baseline"], json!(dir.join("tests/__snapshots__/card.png").display().to_string()));
    let pixels = of(&evs, "failed", "pixels");
    assert_eq!(pixels["kind"], json!("pixels"));
    assert_eq!(pixels["snapshots"][0]["name"], json!("card"));
    // the file's end, then the totals
    let f = evs.iter().find(|e| e["event"] == json!("file")).unwrap();
    assert_eq!((f["passed"].clone(), f["failed"].clone()), (json!(1), json!(5)));
    let last = evs.last().unwrap();
    assert_eq!(last["event"], json!("finished"));
    assert_eq!((last["passed"].clone(), last["failed"].clone(), last["code"].clone()), (json!(1), json!(5), json!(1)));
}

#[test]
fn only_narrows_and_says_what_it_left_out_and_several_paths_run_once_each() {
    let dir = project("only", &[("lib/calc.ol", CALC), ("tests/calc_test.ol", TESTS)]);
    let (code, out, _) = olang(&dir, &["test", "--format", "json", "--only", "adds", "tests/calc_test.ol"], &[]);
    assert_eq!(code, 0);
    let (evs, _) = events(&out);
    assert_eq!(evs[0]["count"], json!(1));
    assert_eq!(evs.iter().filter(|e| e["event"] == json!("skipped")).count(), 5);
    of(&evs, "passed", "adds");
    // a file and its tests in one run: the module has no blocks, the tests
    // run once
    let (_, out2, _) = olang(&dir, &["test", "--format", "json", "lib/calc.ol", "tests/calc_test.ol", "tests/calc_test.ol"], &[]);
    let (evs2, _) = events(&out2);
    assert_eq!(evs2[0]["files"].as_array().unwrap().len(), 1);
    assert_eq!(evs2.iter().filter(|e| e["event"] == json!("test")).count(), 6);
}

#[test]
fn verify_tiers_names_a_divergence_and_ends_the_run() {
    // OLANG_VERIFY_SELFTEST makes every verified native call disagree
    // with itself: the path a real divergence takes
    let src = "fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)\n\ntest \"fib\" {\n    assert_eq(fib(15), 610)\n}\n";
    let dir = project("verify", &[("fib_test.ol", src)]);
    let (code, out, err) = olang(&dir, &["--verify-tiers", "1", "test", "--format", "json", "fib_test.ol"], &[("OLANG_VERIFY_SELFTEST", "1")]);
    assert_eq!(code, 102, "{out}{err}");
    let (evs, _) = events(&out);
    assert_eq!(evs[0]["verify_tiers"], json!(1.0));
    let d = evs.iter().find(|e| e["event"] == json!("divergence")).expect("a divergence event");
    assert!(d["function"].as_str().unwrap().contains("fib"), "{d}");
    assert_eq!(d["name"], json!("fib"));
    assert!(d["native"].is_string() && d["vm"].is_string(), "{d}");
    let last = evs.last().unwrap();
    assert_eq!(last["event"], json!("finished"));
    assert_eq!(last["aborted"], json!("divergence"));
    // and without the self-test, verification passes
    let (code, out, _) = olang(&dir, &["--verify-tiers", "1", "test", "--format", "json", "fib_test.ol"], &[]);
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_parse_error_and_a_file_error_end_their_file() {
    let dir = project("errors", &[("a_test.ol", "test \"x\" {\n  assert_eq(1, \n"), ("b_test.ol", "let z = [1][5]\n\ntest \"y\" {\n    assert_eq(1, 1)\n}\n")]);
    let (code, out, _) = olang(&dir, &["test", "--format", "json"], &[]);
    assert_eq!(code, 1);
    let (evs, _) = events(&out);
    let files: Vec<&J> = evs.iter().filter(|e| e["event"] == json!("file")).collect();
    let parse = files.iter().find(|f| f["file"].as_str().unwrap().ends_with("a_test.ol")).unwrap();
    assert_eq!(parse["error"]["kind"], json!("parse"));
    let file_err = files.iter().find(|f| f["file"].as_str().unwrap().ends_with("b_test.ol")).unwrap();
    assert_eq!(file_err["error"]["kind"], json!("error"));
    assert_eq!(file_err["error"]["line"], json!(1));
}

#[test]
fn the_human_output_is_unchanged() {
    let dir = project("human", &[("lib/calc.ol", CALC), ("tests/calc_test.ol", TESTS)]);
    let (code, out, _) = olang(&dir, &["test", "tests/calc_test.ol"], &[("NO_COLOR", "1")]);
    assert_eq!(code, 1);
    assert!(!out.contains("{\"event\""), "{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "tests/calc_test.ol", "{out}");
    // what the blocks print comes first: the report follows the file's run
    assert_eq!(lines[1], "checking out", "{out}");
    assert_eq!(lines[2], "  ✓ adds", "{out}");
    assert_eq!(lines[3], "  ✗ off by one", "{out}");
    assert_eq!(lines[4], "      Runtime error: Assertion failed: Integer(42) != Integer(41)", "{out}");
    assert!(out.contains("  1 passed, 5 failed  (1 test file, "), "{out}");
}

#[test]
fn paths_are_said_as_they_were_given() {
    // a project reached through a symlink: the events name its files by
    // the link, as the editor that asked knows them
    let dir = project("alias", &[("lib/calc.ol", CALC), ("tests/calc_test.ol", "use lib.calc { total }\n\ntest \"t\" {\n    assert_eq(total([1]), 2)\n}\n")]);
    let link = std::env::temp_dir().join(format!("olang-test-events-link-{}", std::process::id()));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    let target = link.join("tests/calc_test.ol");
    let (_, out, _) = olang(&dir, &["test", "--format", "json", target.to_str().unwrap()], &[]);
    let (evs, _) = events(&out);
    let given = target.display().to_string();
    assert_eq!(evs[0]["files"], json!([given]));
    assert_eq!(of(&evs, "failed", "t")["file"], json!(given));
    assert_eq!(of(&evs, "failed", "t")["at"]["file"], json!(given));
    let _ = std::fs::remove_file(&link);
}

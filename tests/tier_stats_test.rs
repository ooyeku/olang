//! The tier map's outputs over the real binary (docs/tooling.md): `olang
//! check --tier --format json` (every function with its verdict, reason,
//! reason code and pin; a pinned function the tier refuses fails the
//! check), `--ovm-stats=json:PATH` (calls and time per tier, deopts,
//! refusals, a compiled function whose calls fell back to the
//! tree-walker, a pinned function that fell back, the guard's exit
//! status), and the REPL's `stats`.

use serde_json::{Value as J, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn project(name: &str, main: &str, toml_extra: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang-tier-stats-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("olang.toml"),
        format!("[package]\nname = \"t\"\nversion = \"0.1.0\"\nauthors = []\n{toml_extra}"),
    )
    .unwrap();
    std::fs::write(dir.join("main.ol"), main).unwrap();
    dir.canonicalize().unwrap()
}

const MAIN: &str = r#"fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)

fn doubled_later(n) = {
    let t = spawn (n * 2)
    task.join(t)
}

fn apply(f, x) = f(x)

fn width(m, s) = m.length(s)

// studio: native
fn tally(ch, n) = n + 1

let ch = chan.new()
let mut total = 0
for i in range(0, 300) {
    total = total + fib(10)
    total = total + apply(len, [i, i])
    total = total + width(str, "ab")
    total = total + tally(ch, i)
}
println(total + doubled_later(1))
"#;

fn olang(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_olang"));
    c.args(args).current_dir(dir);
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

fn row<'a>(fns: &'a J, name: &str) -> &'a J {
    fns.as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == json!(name))
        .unwrap_or_else(|| panic!("no row for {name} in {fns}"))
}

#[test]
fn check_tier_json_lists_every_function_with_its_verdict_and_pin() {
    let dir = project("check", MAIN, "[check]\nnative = [\"fib\"]\n");
    let (code, out, err) = olang(&dir, &["check", "--tier", "--format", "json", "main.ol"], &[]);
    assert_eq!(code, 0, "{err}");
    let j: J = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out}"));
    assert_eq!(j["kind"], json!("tier-check"));
    let fns = &j["files"][0]["functions"];
    assert_eq!(row(fns, "fib")["verdict"], json!("bytecode"));
    assert_eq!(row(fns, "fib")["line"], json!(1));
    assert_eq!(row(fns, "fib")["pinned"], json!("olang.toml"));
    assert_eq!(row(fns, "tally")["pinned"], json!("comment"));
    let refused = row(fns, "doubled_later");
    assert_eq!(refused["verdict"], json!("refused"));
    assert_eq!(refused["code"], json!("task"));
    assert_eq!(refused["docs"], json!("docs/tooling.md#tier-task"));
    // the surprise is invisible here: `apply` compiles
    assert_eq!(row(fns, "apply")["verdict"], json!("bytecode"));
}

#[test]
fn a_pinned_function_the_tier_refuses_fails_the_check() {
    let dir = project("pinned", MAIN, "[check]\nnative = [\"doubled_*\"]\n");
    let (code, out, _) = olang(&dir, &["check", "--tier", "main.ol"], &[]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("is pinned native ([check] native) but stays on the tree-walker"), "{out}");
    let (code, out, _) = olang(&dir, &["check", "--tier", "--format", "json", "main.ol"], &[]);
    assert_eq!(code, 1);
    let j: J = serde_json::from_str(&out).unwrap();
    assert_eq!(row(&j["files"][0]["functions"], "doubled_later")["error"], json!(true));
}

#[test]
fn ovm_stats_json_says_where_each_function_ran_and_why() {
    let dir = project("run", MAIN, "");
    let stats = dir.join("stats.json");
    let arg = format!("--ovm-stats=json:{}", stats.display());
    let (code, out, err) = olang(&dir, &[&arg, "main.ol"], &[]);
    assert_eq!(code, 0, "{err}");
    assert!(!out.trim().is_empty(), "{out}");
    let j: J = serde_json::from_str(&std::fs::read_to_string(&stats).unwrap()).unwrap();
    assert_eq!(j["kind"], json!("ovm-stats"));
    let fns = &j["functions"];
    // native: every call counted once, on native code — the 300 the VM
    // made and the 52,800 native code made of itself (fib(10) calls fib
    // 176 times below it), which push no frame and are counted at their
    // call sites
    let fib = row(fns, "fib");
    assert_eq!(fib["tier"], json!("native"), "{fib}");
    assert_eq!(fib["calls"]["native"], json!(300 * 177));
    assert_eq!(fib["calls"]["interpreter"], json!(0));
    assert_eq!(fib["line"], json!(1));
    // a builtin passed as a value crosses the boundary: compiled, no fallback
    let apply = row(fns, "apply");
    assert_ne!(apply["tier"], json!("interpreter"), "{apply}");
    assert!(apply["fallbacks"].is_null(), "{apply}");
    // compiles, but a module passed as a value keeps every call on the
    // tree-walker: a fallback, with its reason
    let width = row(fns, "width");
    assert_eq!(width["tier"], json!("interpreter"), "{width}");
    assert_eq!(width["fallbacks"]["count"], json!(300));
    assert!(width["fallbacks"]["reason"].as_str().unwrap().contains("cannot convert"));
    // a channel argument: bytecode, each native attempt declined
    let tally = row(fns, "tally");
    assert_eq!(tally["tier"], json!("bytecode"), "{tally}");
    assert_eq!(tally["deopts"], json!(300));
    assert_eq!(tally["pinned"], json!("comment"));
    assert!(tally["violation"].as_str().unwrap().contains("stayed on bytecode"));
    assert_eq!(j["violations"].as_array().unwrap().len(), 1);
    assert!(err.contains("tier guard:") && err.contains("`tally`"), "{err}");
    // refused at compile time, with its code
    assert_eq!(row(fns, "doubled_later")["refused"]["code"], json!("task"));
}

#[test]
fn the_guard_fails_a_run_whose_pinned_function_fell_back() {
    let dir = project("guard", MAIN, "");
    let stats = dir.join("s.json");
    let (code, _, _) = olang(
        &dir,
        &["main.ol"],
        &[("OLANG_OVM_STATS", &format!("json:{}", stats.display())), ("OLANG_TIER_GUARD", "1")],
    );
    assert_eq!(code, 3);
    assert!(stats.exists());
    // without the guard the run succeeds
    let (code, _, _) = olang(&dir, &["main.ol"], &[("OLANG_OVM_STATS", &format!("json:{}", stats.display()))]);
    assert_eq!(code, 0);
}

#[test]
fn the_repl_answers_its_evaluations_statistics() {
    let dir = project("repl", MAIN, "");
    let mut child = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["repl", "--serve"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut recv = || {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str::<J>(&line).unwrap()
    };
    let hello = recv();
    assert!(hello["ops"].as_array().unwrap().contains(&json!("stats")));
    let file = dir.join("main.ol");
    writeln!(stdin, "{}", json!({"op": "eval", "id": 1, "code": "fib(15) + width(str, \"a\") + apply(len, [1])", "file": file, "line": 24})).unwrap();
    stdin.flush().unwrap();
    let r = recv();
    assert_eq!(r["ok"], json!(true), "{r}");
    writeln!(stdin, "{}", json!({"op": "stats", "id": 2, "reset": true})).unwrap();
    stdin.flush().unwrap();
    let s = recv();
    assert_eq!(s["id"], json!(2));
    let fns = &s["stats"]["functions"];
    assert_eq!(row(fns, "fib")["tier"], json!("native"), "{fns}");
    assert_eq!(row(fns, "width")["tier"], json!("interpreter"));
    // reset: the next answer starts from nothing
    writeln!(stdin, "{}", json!({"op": "stats", "id": 3})).unwrap();
    stdin.flush().unwrap();
    let s2 = recv();
    assert!(s2["stats"]["functions"].as_array().unwrap().iter().all(|f| f["calls"]["native"] == json!(0)), "{s2}");
    let _ = writeln!(stdin, "{}", json!({"op": "shutdown", "id": 4}));
    let _ = child.wait();
}

#[test]
fn a_directory_keeps_each_process_its_own_file() {
    // `json:DIR/`: a run writes `<pid>.json` there, and tells its children
    // the same directory — `olang bench`'s runs each leave theirs, where a
    // single path kept only the last child's
    let dir = project("dir", MAIN, "");
    let out = dir.join("stats");
    let arg = format!("--ovm-stats=json:{}/", out.display());
    let (code, _, err) = olang(&dir, &[&arg, "bench", "--runs", "1", "main.ol"], &[]);
    assert_eq!(code, 0, "{err}");
    let files: Vec<PathBuf> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().path()).collect();
    // the bench itself, its warm-up and its one timed run
    assert!(files.len() >= 3, "{files:?}");
    let with_fib = files
        .iter()
        .filter(|f| {
            let j: J = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            j["functions"].as_array().unwrap().iter().any(|r| r["name"] == json!("fib"))
        })
        .count();
    assert!(with_fib >= 2, "each child's run kept: {files:?}");
}

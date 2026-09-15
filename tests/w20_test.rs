//! Roadmap W20 — toward 0.85.0: typed lets on the VM, range consumers
//! on the VM, union aliases, the manifest's check promotion, the
//! project doctor, and a timeline that follows spawned tasks.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_w20_{}_{}", std::process::id(), tag));
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
        .output()
        .expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

const MODES: [&[&str]; 2] = [&["run"], &["--no-ovm", "run"]];

fn in_both_tiers(dir: &Path, file: &str, expect: &str) {
    for mode in MODES {
        let mut args: Vec<&str> = mode.to_vec();
        args.push(file);
        let (out, err, rc) = olang(dir, &args);
        assert_eq!(rc, 0, "{mode:?}: {out}{err}");
        assert_eq!(out, expect, "{mode:?}");
    }
}

#[test]
fn a_typed_let_runs_on_the_vm_with_the_interpreters_message() {
    let ws = workspace("typedlet");
    write(
        &ws,
        "t.ol",
        "type Task = { title: String }\n\
         fn hot(x) = {\n    let n: Int = x\n    let t: Task = #{ \"title\": \"a\" }\n    n + str.length(map_get(t, \"title\"))\n}\n\
         let mut acc = 0\n\
         for i in range(0, 1000) { acc = acc + hot(i) }\n\
         println(to_string(acc))\n\
         println(show(attempt(() => hot(\"a\"))))\n\
         fn typed_list(xs) = { let ys: [Int] = xs; len(ys) }\n\
         println(show(attempt(() => typed_list(3))))\n\
         println(to_string(typed_list([1, 2])))\n",
    );
    in_both_tiers(
        &ws,
        "t.ol",
        "500500\nErr(\"let binding 'n' expects Int, got String\")\nErr(\"let binding 'ys' expects List, got Int\")\n2\n",
    );
    // The function is not refused to the interpreter any more.
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(["run", "t.ol"])
        .env("OLANG_TIER_VERBOSE", "1")
        .current_dir(&ws)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !err.contains("let bindings with type annotations run interpreted"),
        "{err}"
    );
}

#[test]
fn folds_and_sums_over_ranges_need_no_list() {
    let ws = workspace("ranges");
    write(
        &ws,
        "r.ol",
        "let n = 2000000\n\
         println(to_string(fold(0..n, 0, (acc, i) => acc + i)))\n\
         println(to_string(sum(0..n)) + \" \" + to_string(sum(1..=10)) + \" \" + to_string(sum(5..5)))\n\
         println(to_string(fold(1..=4, 1, (a, i) => a * i)) + \" \" + to_string(reduce(0..0, 5, (a, i) => a + i)))\n\
         fn infn(m) = fold(0..m, 0, (a, i) => a + i * 2)\n\
         println(to_string(infn(100)))\n",
    );
    in_both_tiers(
        &ws,
        "r.ol",
        "1999999000000\n1999999000000 55 0\n24 5\n9900\n",
    );
}

#[test]
fn a_union_declaration_is_an_alias_of_the_union_annotation() {
    let ws = workspace("union");
    write(
        &ws,
        "u.ol",
        "type Id = Int | String\n\
         fn show_id(i: Id) = to_string(i)\n\
         println(show_id(3) + show_id(\"x\"))\n\
         println(show(attempt(() => show_id(1.5))))\n\
         let nodes = unwrap(meta.parse(\"type Id = Int | String\"))\n\
         println(map_get(head(nodes), \"definition\") + \" \" + map_get(map_get(head(nodes), \"type\"), \"form\"))\n",
    );
    // The parser reads `Int | String` as a union annotation behind an
    // alias: one declaration kind, the union form inside it.
    in_both_tiers(
        &ws,
        "u.ol",
        "3x\nErr(\"parameter 'i' of show_id expects Int | String, got Float\")\nalias union\n",
    );
}

#[test]
fn the_manifest_promotes_warning_classes_to_errors() {
    let ws = workspace("promote");
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n\n[check]\npromote = [\"exhaustiveness\", \"shape\"]\n",
    );
    write(
        &ws,
        "m.ol",
        "type Shape = enum { Circle(Float), Rect(Float, Float) }\n\
         fn area(s: Shape) = match s { Circle(r) => 1.0 }\n\
         type Task = { title: String }\n\
         fn t(r: Task) = map_get(r, \"titel\")\n",
    );
    let (out, err, rc) = olang(&ws, &["check", "m.ol"]);
    let all = out + &err;
    assert_ne!(rc, 0, "{all}");
    assert!(
        all.contains("promoted to an error by [check] promote"),
        "{all}"
    );
    assert!(all.contains("2 problems found"), "{all}");
    // Without the block the same findings are advisories.
    write(
        &ws,
        "olang.toml",
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\n",
    );
    let (out, err, rc) = olang(&ws, &["check", "m.ol"]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!((out + &err).contains("2 warnings"));
}

#[test]
fn the_project_doctor_names_what_is_off() {
    let ws = workspace("doctor");
    write(
        &ws,
        "app/olang.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nshuttle = { path = \"../vendor/shuttle\" }\nghost = { path = \"../nowhere\" }\n",
    );
    write(
        &ws,
        "vendor/shuttle/olang.toml",
        "[package]\nname = \"shuttle\"\nversion = \"0.1.0\"\n",
    );
    write(&ws, "app/static/olang_playground.wasm", "stale");
    let (out, err, rc) = olang(&ws.join("app"), &["doctor"]);
    assert_eq!(rc, 1, "{out}{err}");
    assert!(out.contains("✓ dependency shuttle"), "{out}");
    assert!(out.contains("✗ dependency ghost"), "{out}");
    assert!(out.contains("✓ no olang.lock yet"), "{out}");
    assert!(
        out.contains("✗ static/olang_playground.wasm is a copy"),
        "{out}"
    );
    assert!(out.contains("browser runtime"), "{out}");
    // A clean project checks out.
    let (out, _, rc) = olang(&ws.join("vendor/shuttle"), &["doctor"]);
    if out.contains("browser runtime embedded") {
        assert_eq!(rc, 0, "{out}");
        assert!(out.contains("everything checks out"), "{out}");
    }
}

#[test]
fn a_spawned_tasks_effects_record_and_replay_on_their_own_stream() {
    let ws = workspace("timeline");
    write(
        &ws,
        "w.ol",
        "let inbox = chan.new()\n\
         let t = spawn {\n\
             let conn = unwrap(db.open(\"worker.db\"))\n\
             unwrap(db.execute(conn, \"CREATE TABLE IF NOT EXISTS t (v INTEGER)\"))\n\
             let d = random.randint(1, 1000000)\n\
             unwrap(db.execute(conn, \"INSERT INTO t (v) VALUES (?)\", [d]))\n\
             chan.send(inbox, len(unwrap(db.query(conn, \"SELECT v FROM t\"))))\n\
             d\n\
         }\n\
         let n = unwrap(chan.recv(inbox))\n\
         println(\"rows \" + to_string(n) + \" drew \" + to_string(task.join(t)) + \" main \" + to_string(random.randint(1, 1000000)))\n",
    );
    let (recorded, err, rc) = olang(&ws, &["--record", "run.olt", "w.ol"]);
    assert_eq!(rc, 0, "{recorded}{err}");
    assert!(
        ws.join("worker.db").exists(),
        "the worker wrote its database while recording"
    );
    std::fs::remove_file(ws.join("worker.db")).unwrap();
    let (replayed, err, rc) = olang(&ws, &["replay", "run.olt"]);
    assert_eq!(rc, 0, "{replayed}{err}");
    assert_eq!(replayed, recorded);
    assert!(err.contains("clean"), "{err}");
    // The worker's effects were served from its stream, not repeated.
    assert!(
        !ws.join("worker.db").exists(),
        "replay re-ran the worker's database writes"
    );
    let trace: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(ws.join("run.olt")).unwrap()).unwrap();
    let threads: Vec<u64> = trace["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["thread"].as_u64().unwrap_or(0))
        .collect();
    assert!(
        threads.contains(&0) && threads.iter().any(|t| *t != 0),
        "{threads:?}"
    );
}

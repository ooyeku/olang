//! The sampled heap profile (src/heap_profile.rs): what holds live memory
//! by the site that allocated it (OL-203), what grew between two
//! snapshots (OL-610), and allocations by function (OL-324).
//!
//! The figures are estimates from samples, so each check allows the
//! sampling error: at one sample per 64 KiB, a site of 8 MB is about 128
//! samples, within a third of its size far more often than these tests
//! run.

use std::process::Command;

fn dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("olang_heap_profile_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn olang(args: &[&str], envs: &[(&str, &str)]) -> (String, String, bool) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_olang"));
    c.args(args).env("OLANG_COMPILE_CACHE", "0");
    for (k, v) in envs {
        c.env(k, v);
    }
    let out = c.output().expect("run olang");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.success(),
    )
}

fn run(name: &str, source: &str, envs: &[(&str, &str)]) -> String {
    let file = dir().join(name);
    std::fs::write(&file, source).unwrap();
    let (out, err, ok) = olang(&[file.to_str().unwrap()], envs);
    assert!(ok, "{out}\n{err}");
    out
}

fn field(out: &str, key: &str) -> i64 {
    let line = out
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{key} ")))
        .unwrap_or_else(|| panic!("no `{key}` line in:\n{out}"));
    line.trim().parse().unwrap_or_else(|_| panic!("`{key}` is not a number: {line}"))
}

fn near(estimate: i64, actual: i64, what: &str) {
    let lo = actual * 2 / 3;
    let hi = actual * 3 / 2;
    assert!(
        estimate >= lo && estimate <= hi,
        "{what}: estimated {estimate}, measured {actual} (allowed {lo}..{hi})"
    );
}

/// A program that keeps what one function makes and throws away what
/// another makes: the kept site holds about what `values` grew by, the
/// thrown-away one allocated much and holds next to nothing.
const KEEP_AND_CHURN: &str = r#"
fn keep(n) = map(range(n), (i) => "kept string number " + to_string(i * 7919))
fn churn(n) = {
    let mut total = 0
    for r in 0..n {
        let xs = map(range(2000), (i) => "temporary " + to_string(i + r))
        total = total + len(xs)
    }
    total
}
runtime.heap_start(#{ "rate": 65536 })
let before = runtime.memory().values
let kept = keep(100000)
let grew = runtime.memory().values - before
println("churned " + to_string(churn(40)))
let snap = runtime.heap()
let site = (f) => first(filter(snap.sites, (s) => s.function == f && s.scope == "values")) ?? #{ "bytes": 0, "allocated": 0, "count": 0 }
// a function's own sites and its lambdas' (`map`'s callback makes the strings)
let sum = (f, k) => fold(filter(snap.sites, (s) => s.function == f || s.function == "<lambda in " + f + ">"), 0, (a, s) => a + s[k])
println("grew " + to_string(grew))
println("keep_bytes " + to_string(sum("keep", "bytes")))
println("keep_count " + to_string(sum("keep", "count")))
println("churn_bytes " + to_string(sum("churn", "bytes")))
println("churn_allocated " + to_string(sum("churn", "allocated")))
println("rate " + to_string(snap.rate))
println("samples " + to_string(snap.samples))
println("kept " + to_string(len(kept)))
println("line " + to_string(site("keep").line ?? 0))
"#;

#[test]
fn live_bytes_are_named_by_the_function_that_allocated_them() {
    for tier in [&[][..], &["--no-ovm"][..]] {
        let file = dir().join(format!("keep{}.ol", tier.len()));
        std::fs::write(&file, KEEP_AND_CHURN).unwrap();
        let mut args: Vec<&str> = tier.to_vec();
        args.push(file.to_str().unwrap());
        let (out, err, ok) = olang(&args, &[]);
        assert!(ok, "{out}\n{err}");
        let grew = field(&out, "grew");
        near(field(&out, "keep_bytes"), grew, "keep's live bytes");
        // 100,000 strings and the list: at least that many blocks
        let count = field(&out, "keep_count");
        assert!(count > 60_000 && count < 400_000, "keep's blocks: {count}\n{out}");
        // churn made ~80,000 strings and kept none of them
        assert!(
            field(&out, "churn_allocated") > 2 * 1024 * 1024,
            "churn's allocations went unseen:\n{out}"
        );
        assert!(
            field(&out, "churn_bytes") < grew / 10,
            "churn holds what it threw away:\n{out}"
        );
        assert_eq!(field(&out, "rate"), 65536);
        assert_eq!(field(&out, "line"), 2, "keep is declared on line 2:\n{out}");
    }
}

#[test]
fn a_diff_names_what_grew_between_two_snapshots() {
    let out = run(
        "grow.ol",
        r#"
let cache = cell(#{})
fn remember(i) = cell.set(cache, map_set(cell.take(cache), "k" + to_string(i), [i, i * 2, "v" + to_string(i)]))
fn steady(n) = len(map(range(n), (i) => to_string(i)))
runtime.heap_start(#{ "rate": 65536 })
let a = runtime.heap()
for i in 0..60000 { remember(i); if i % 100 == 0 => steady(500) else => 0 }
let b = runtime.heap()
let d = runtime.heap_diff(a, b)
let top = first(d.sites)
println("top " + top.function)
println("grew " + to_string(top.grew))
println("values_grew " + to_string(d.values_grew))
println("steady " + to_string(fold(filter(d.sites, (s) => s.function == "steady"), 0, (acc, s) => acc + s.grew)))
"#,
        &[],
    );
    assert!(out.contains("top remember"), "{out}");
    let values = field(&out, "values_grew");
    assert!(values > 4 * 1024 * 1024, "{out}");
    // the one site may not be all of it (the map's table is remember's,
    // its entries' strings too), but it is most of what grew
    assert!(field(&out, "grew") > values / 3, "{out}");
    assert!(field(&out, "steady").abs() < values / 10, "{out}");
}

#[test]
fn what_runs_before_sampling_starts_is_not_seen_and_stop_forgets() {
    let out = run(
        "stop.ol",
        r#"
let early = map(range(50000), (i) => "early " + to_string(i))
runtime.heap_start(#{ "rate": 65536 })
println("again " + show(runtime.heap_start()))
println("sampled " + to_string(runtime.heap().sampled))
println("stop " + show(runtime.heap_stop()))
println("stop2 " + show(runtime.heap_stop()))
let after = runtime.heap()
println("on " + show(after.on))
println("left " + to_string(after.samples))
println("early " + to_string(len(early)))
"#,
        &[],
    );
    assert!(out.contains("again Err("), "{out}");
    // the 50,000 early strings are ~3 MB, none sampled
    assert!(field(&out, "sampled") < 1024 * 1024, "{out}");
    assert!(out.contains("stop Ok(())"), "{out}");
    assert!(out.contains("stop2 Err("), "{out}");
    assert!(out.contains("on false"), "{out}");
    assert_eq!(field(&out, "left"), 0, "{out}");
}

#[test]
fn olang_memory_run_show_and_diff() {
    let d = dir();
    let src = d.join("cli.ol");
    std::fs::write(
        &src,
        r#"
fn build_index(n) = map(range(n), (i) => #{ "id": i, "name": "entry " + to_string(i) })
let a = runtime.heap_write(os.args()[1] + "/a.json")
let idx = build_index(30000)
let b = runtime.heap_write(os.args()[1] + "/b.json")
println(len(idx))
"#,
    )
    .unwrap();
    let dirs = d.to_str().unwrap();
    let (out, err, ok) = olang(
        &["memory", "run", "--rate", "64k", src.to_str().unwrap(), dirs],
        &[],
    );
    assert!(ok, "{out}\n{err}");
    assert!(out.contains("build_index"), "{out}");
    assert!(out.contains("cli.ol:2"), "the site's place:\n{out}");

    let (out, err, ok) = olang(
        &["memory", "diff", &format!("{dirs}/a.json"), &format!("{dirs}/b.json")],
        &[],
    );
    assert!(ok, "{out}\n{err}");
    let first_site = out
        .lines()
        .skip_while(|l| !l.trim_start().starts_with("grew"))
        .nth(1)
        .unwrap_or("");
    assert!(first_site.contains("build_index"), "{out}");

    let (out, _, ok) = olang(&["memory", "show", &format!("{dirs}/b.json")], &[]);
    assert!(ok && out.contains("build_index"), "{out}");

    let (out, err, ok) = olang(&["memory", "diff", &format!("{dirs}/a.json")], &[]);
    assert!(!ok && (out + &err).contains("olang memory"), "a lone file is not a folder");
}

#[test]
fn snapshots_are_written_on_a_clock_and_a_folder_diffs() {
    let d = dir().join("every");
    let _ = std::fs::remove_dir_all(&d);
    let src = dir().join("clock.ol");
    std::fs::write(
        &src,
        r#"
let held = cell([])
fn grow(i) = cell.set(held, cell.take(held) + [map(range(2000), (k) => "g" + to_string(k + i))])
for i in 0..40 { grow(i); time.sleep(25) }
println(len(cell.get(held)))
"#,
    )
    .unwrap();
    let ds = d.to_str().unwrap();
    let (out, err, ok) = olang(
        &[src.to_str().unwrap()],
        &[
            ("OLANG_HEAP", "64k"),
            ("OLANG_HEAP_DIR", ds),
            ("OLANG_HEAP_EVERY", "0.2"),
        ],
    );
    assert!(ok, "{out}\n{err}");
    let n = std::fs::read_dir(&d)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".json")
        })
        .count();
    assert!(n >= 2, "snapshots written: {n}");
    let (out, err, ok) = olang(&["memory", "diff", ds], &[]);
    assert!(ok, "{out}\n{err}");
    assert!(out.contains("grow"), "{out}");
}

#[test]
fn a_json_profile_says_what_each_function_allocates() {
    let src = dir().join("prof.ol");
    std::fs::write(
        &src,
        r#"
fn allocates(n) = len(map(range(n), (i) => "x" + to_string(i)))
fn computes(n) = { let mut a = 0; let mut i = 0; while i < n { a = a + i * i; i = i + 1 }; a }
let mut t = 0
for r in 0..30 { t = t + allocates(20000) + computes(20000) }
println(t)
"#,
    )
    .unwrap();
    let (out, err, ok) = olang(
        &["profile", "--format", "json", src.to_str().unwrap()],
        &[],
    );
    assert!(ok, "{err}");
    let json_line = out.lines().rfind(|l| l.starts_with('{')).unwrap();
    let doc: serde_json::Value = serde_json::from_str(json_line).unwrap();
    assert!(doc["alloc_b"].as_u64().unwrap() > 10 * 1024 * 1024, "{}", doc["alloc_b"]);
    assert!(doc["allocs_n"].as_u64().unwrap() > 600_000);
    let frames = doc["frames"].as_array().unwrap();
    let row = |name: &str| {
        doc["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| frames[r["frame"].as_u64().unwrap() as usize]["name"] == name)
            .cloned()
    };
    // the lambda inside `map` makes the strings; `allocates` is its
    // caller, so either may carry them, and `computes` next to none
    let lam = row("<lambda in allocates>")
        .and_then(|r| r["alloc_b"].as_u64())
        .unwrap_or(0);
    let own = row("allocates").and_then(|r| r["alloc_b"].as_u64()).unwrap_or(0);
    let map_b = row("map").and_then(|r| r["alloc_b"].as_u64()).unwrap_or(0);
    assert!(lam + own + map_b > 5 * 1024 * 1024, "{}", doc["functions"]);
    if let Some(c) = row("computes") {
        assert!(c["alloc_b"].as_u64().unwrap_or(0) < (lam + own + map_b) / 10);
    }
}

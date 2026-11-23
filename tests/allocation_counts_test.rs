//! Allocations made and bytes asked for are counted in every build, on
//! each thread's own counter (OL-324): `runtime.memory().allocations`
//! and `allocated`, and a profile's `allocs_n` and `alloc_b`.

use std::process::Command;

fn olang(args: &[&str], source: &str, name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("olang_alloc_counts_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, source).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .args(args)
        .arg(&file)
        .env("OLANG_COMPILE_CACHE", "0")
        .output()
        .expect("run olang");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn field(out: &str, key: &str) -> i64 {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{key} ")))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or_else(|| panic!("no `{key}` in:\n{out}"))
}

const COUNT: &str = r#"
let m0 = runtime.memory()
let xs = map(range(50000), (i) => "s" + to_string(i))
let m1 = runtime.memory()
println("n " + to_string(m1.allocations - m0.allocations))
println("b " + to_string(m1.allocated - m0.allocated))
println("len " + to_string(len(xs)))
"#;

#[test]
fn allocations_are_counted_exactly_in_every_build() {
    for tier in [&[][..], &["--no-ovm"][..]] {
        let out = olang(tier, COUNT, &format!("count{}.ol", tier.len()));
        // a string each at the least, and their bytes
        assert!(field(&out, "n") >= 50_000, "{out}");
        assert!(field(&out, "b") >= 50_000 * 6, "{out}");
    }
}

#[test]
fn a_json_profile_counts_the_run_s_allocations() {
    let out = olang(
        &["profile", "--format", "json"],
        "let xs = map(range(30000), (i) => [i, to_string(i)])\nprintln(len(xs))\n",
        "prof.ol",
    );
    let doc: serde_json::Value =
        serde_json::from_str(out.lines().rfind(|l| l.starts_with('{')).unwrap()).unwrap();
    assert!(
        doc["allocs_n"].as_u64().unwrap() >= 60_000,
        "{}",
        doc["allocs_n"]
    );
    assert!(
        doc["alloc_b"].as_u64().unwrap() > 1_000_000,
        "{}",
        doc["alloc_b"]
    );
    // each point of the series carries the count as it stood then: the
    // last one is taken before the run's end, so it is not held to the total
    let series = doc["series"].as_array().unwrap();
    let at = |p: &serde_json::Value| p["allocations"].as_u64().unwrap();
    assert!(at(series.last().unwrap()) > 0);
    assert!(at(series.last().unwrap()) >= at(&series[0]));
}

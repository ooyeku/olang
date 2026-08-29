//! Hot-loop promotion (interpreter/loop_promo.rs): a top-level loop
//! that outlives the threshold runs its remainder on the tier in one
//! boundary crossing. Every case here is a differential: the tiered
//! binary and `--no-ovm` (the interpreter oracle) must print the same
//! bytes — values, error messages, and error spans alike.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> String {
    let dir = std::env::temp_dir().join("olang_loop_promo_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join(format!("t{:x}.ol", md5ish(source)));
    std::fs::write(&path, source).expect("write");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    if no_ovm {
        cmd.arg("--no-ovm");
    }
    let out = cmd.arg("run").arg(&path).output().expect("run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn md5ish(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn assert_tiers_agree(source: &str) {
    let tiered = run(source, false);
    let oracle = run(source, true);
    assert_eq!(tiered, oracle, "tiers diverged on:\n{source}");
}

#[test]
fn promoted_loops_compute_what_the_interpreter_computes() {
    // Accumulation over a function call chain — the shape that motivated
    // the mechanism (300k boundary crossings became one).
    assert_tiers_agree(
        "fn scale(x) = x * 3\n\
         fn shift(x) = x + 7\n\
         fn score(x) = shift(scale(x))\n\
         let mut acc = 0\n\
         for i in 1..30000 { acc = (acc + score(i)) % 1000003 }\n\
         println(acc)",
    );
}

#[test]
fn break_value_and_continue_survive_promotion() {
    assert_tiers_agree(
        "let found = for i in 1..100000 { if i * i > 5000000 => break i }\n\
         println(found)",
    );
    assert_tiers_agree(
        "let mut evens = 0\n\
         for i in 1..50000 { if i % 2 == 1 => continue\n evens = evens + 1 }\n\
         println(evens)",
    );
}

#[test]
fn multiple_live_outs_write_back() {
    assert_tiers_agree(
        "let mut a = 0\n\
         let mut b = 1\n\
         for i in 1..5000 { let t = a + b\n a = b\n b = t % 999983 }\n\
         println(`${a} ${b}`)",
    );
}

#[test]
fn while_loops_promote_too() {
    assert_tiers_agree(
        "let mut x = 1000000\n\
         let mut steps = 0\n\
         while x != 1 {\n\
             x = if x % 2 == 0 => x / 2 else => 3 * x + 1\n\
             steps = steps + 1\n\
         }\n\
         println(steps)",
    );
}

#[test]
fn errors_inside_the_promoted_remainder_match_the_oracle() {
    // The failure fires well past the promotion threshold; message AND
    // span must match the pure interpreter's report.
    assert_tiers_agree(
        "let xs = [1, 2, 3]\n\
         let mut s = 0\n\
         for i in 0..2000 {\n\
             s = s + xs[if i < 900 => 0 else => 7]\n\
         }\n\
         println(s)",
    );
}

#[test]
fn inclusive_max_range_terminates() {
    // The old RangeIter existed to keep `i + 1` from overflowing at
    // i64::MAX; the counter rewrite must keep that guarantee.
    assert_tiers_agree(
        "let mut c = 0\n\
         for i in 9223372036854775805..=9223372036854775807 { c = c + 1 }\n\
         println(c)",
    );
}

#[test]
fn loops_the_analyzer_must_refuse_stay_interpreted_and_correct() {
    // `?` crosses the function boundary — promotion must refuse, and the
    // loop must still run right.
    assert_tiers_agree(
        "fn find() = {\n\
             let mut s = 0\n\
             for i in 1..2000 {\n\
                 let v = if i == 1500 => Err(\"stop\") else => Ok(i)\n\
                 s = s + v?\n\
             }\n\
             Ok(s)\n\
         }\n\
         println(find())",
    );
    // Effects: println inside a promoted loop stays ordered and complete.
    assert_tiers_agree(
        "let mut n = 0\n\
         for i in 1..600 { n = n + 1\n if i % 250 == 0 => println(`tick ${i}`) }\n\
         println(n)",
    );
}

#[test]
fn promotion_actually_happens() {
    // Not just correct — engaged: the stats line proves the loop crossed
    // once instead of once per iteration.
    let out = Command::new(env!("CARGO_BIN_EXE_olang"))
        .env("OLANG_TIER_STATS", "1")
        .arg("run")
        .arg({
            let dir = std::env::temp_dir().join("olang_loop_promo_tests");
            std::fs::create_dir_all(&dir).expect("mkdir");
            let p = dir.join("engaged.ol");
            std::fs::write(
                &p,
                "fn f(x) = x * 2 + 1\n\
                 let mut s = 0\n\
                 for i in 1..100000 { s = s + f(i) }\n\
                 println(s)",
            )
            .expect("write");
            p
        })
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&out.stderr);
    let stats = err
        .lines()
        .find(|l| l.starts_with("tier-stats:"))
        .unwrap_or_else(|| panic!("no tier-stats line in: {err}"));
    let calls: u64 = stats
        .split("bytecode_calls=")
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .expect("bytecode_calls");
    assert!(
        calls < 1000,
        "expected one promoted crossing (plus the peeled prefix), got {calls} boundary calls: {stats}"
    );
}

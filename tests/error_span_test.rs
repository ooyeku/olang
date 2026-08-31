//! Error spans through promoted code (roadmap W8): the whole report —
//! message, span, and call stack — must be byte-identical between the
//! tiered binary and `--no-ovm`, for errors raised from every
//! acceleration shape: JIT-compiled calls, OSR'd loop regions, fused
//! list/map writes, promoted top-level loops, and bridge crossings.
//! The incident behind the row (a division by zero deep in a
//! tree-builder reported at an unrelated top-level line) predates the
//! OSR discard-and-retry rework; these cases pin the property that
//! killed it.

use std::process::Command;

fn run(source: &str, no_ovm: bool) -> String {
    let dir = std::env::temp_dir().join("olang_error_span_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    let path = dir.join(format!("t{:x}.ol", h.finish()));
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

/// The full report — including the rendered span line numbers and the
/// call stack — must match byte for byte.
fn assert_reports_identical(source: &str) {
    let tiered = run(source, false);
    let oracle = run(source, true);
    assert_eq!(tiered, oracle, "error report diverged on:\n{source}");
    assert!(
        tiered.contains("error occurred here"),
        "no span rendered at all:\n{tiered}"
    );
}

#[test]
fn division_by_zero_inside_a_jitted_call_chain() {
    assert_reports_identical(
        "fn inner(a, b) = {\n\
             let x = a * 2\n\
             let y = x / b\n\
             y\n\
         }\n\
         fn outer(n) = {\n\
             let mut t = 0\n\
             for i in 0..n { t = t + inner(i, 1) }\n\
             t + inner(1, 0)\n\
         }\n\
         println(outer(3000))",
    );
}

#[test]
fn error_after_a_loop_promotes_mid_function() {
    // The loop runs past the promotion threshold before the failing
    // iteration, so the error is raised from the promoted remainder.
    assert_reports_identical(
        "fn train(n) = {\n\
             let mut t = 0\n\
             let mut i = 0\n\
             while i < n {\n\
                 t = t + i\n\
                 if i == 900 => { t = t / (i - i) }\n\
                 i = i + 1\n\
             }\n\
             t\n\
         }\n\
         println(train(2000))",
    );
}

#[test]
fn float_division_by_zero_inside_an_osr_region() {
    // Float arithmetic compiles natively; the zero divisor must deopt
    // and re-raise the interpreter's trap with the original span.
    assert_reports_identical(
        "fn gini(cols, nodes) = {\n\
             let mut best = 0.0\n\
             let mut node = 0\n\
             while node < nodes {\n\
                 let mut f = 0\n\
                 while f < 3 {\n\
                     let mut s = 0.0\n\
                     for r in 0..200 { s = s + cols[f][r] }\n\
                     let denom = if node == 700 => 0.0 else => 1.0\n\
                     best = s / denom\n\
                     f = f + 1\n\
                 }\n\
                 node = node + 1\n\
             }\n\
             best\n\
         }\n\
         let cols = map(0..3, (f) => map(0..200, (r) => to_float(r)))\n\
         println(gini(cols, 900))",
    );
}

#[test]
fn out_of_bounds_writes_in_hot_fused_loops() {
    assert_reports_identical(
        "fn fill(n) = {\n\
             let mut xs = map(0..100, (i) => to_float(i))\n\
             let mut i = 0\n\
             while i < n {\n\
                 xs = col.set(xs, i % 100, 1.0)\n\
                 i = i + 1\n\
             }\n\
             col.set(xs, 500, 2.0)\n\
         }\n\
         println(len(fill(2000)))",
    );
}

#[test]
fn errors_in_promoted_top_level_loops() {
    assert_reports_identical(
        "fn risky(i) = if i == 1500 => i / (i - i) else => i\n\
         let mut t = 0\n\
         for i in 0..3000 {\n\
             t = t + risky(i)\n\
         }\n\
         println(t)",
    );
}

#[test]
fn errors_crossing_the_bridge_in_hot_higher_order_calls() {
    assert_reports_identical(
        "fn transform(v) = {\n\
             let a = v + 1\n\
             let b = a / (v - 2500)\n\
             b\n\
         }\n\
         let out = map(map(0..3000, (i) => i), (x) => transform(x))\n\
         println(len(out))",
    );
}

#[test]
fn deep_stacks_name_every_frame_identically() {
    assert_reports_identical(
        "fn level3(x) = x / (x - x)\n\
         fn level2(x) = level3(x + 1)\n\
         fn level1(x) = level2(x * 2)\n\
         fn drive(n) = {\n\
             let mut t = 0\n\
             for i in 0..n { t = t + level1(1) * 0 + 1 }\n\
             level1(t - t)\n\
         }\n\
         println(drive(2000))",
    );
}

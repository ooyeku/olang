//! On-stack replacement (Campaign 7, T3): a hot loop inside a function
//! the whole-function JIT refuses (the println in each fixture forces
//! that) leaves the VM mid-frame and finishes natively. These tests
//! drive the real binary and read the OLANG_OSR_DEBUG markers: the
//! contract is "the loop entered natively AND the answer is exactly the
//! interpreter's" — the speed class is the mechanism, equality is the
//! promise.

use std::path::PathBuf;
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn write_script(name: &str, source: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_osr_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    path
}

fn run(script: &PathBuf, envs: &[(&str, &str)]) -> (String, String) {
    let mut cmd = Command::new(olang());
    cmd.arg("run").arg(script);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_hot_loop_enters_natively_and_agrees() {
    // The println blocks whole-function JIT; the loop runs 200k
    // iterations — far past the back-edge threshold.
    let script = write_script(
        "hot_loop.ol",
        "fn crunch(n) = {\n\
             println(\"start\")\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < n { acc = (acc + i * 3) % 1000000007 i = i + 1 }\n\
             acc\n\
         }\n\
         println(crunch(200000))\n",
    );
    let (on_out, on_err) = run(&script, &[("OLANG_OSR_DEBUG", "1")]);
    let (off_out, _) = run(&script, &[("OLANG_OSR_OFF", "1")]);
    assert!(
        on_err.contains("loop entered natively"),
        "OSR must engage: {}",
        on_err
    );
    assert_eq!(on_out, off_out, "OSR must not change the answer");
    // 3 * (0 + 1 + ... + 199999) mod 1e9+7
    assert!(on_out.contains("999699587"), "the sum itself: {}", on_out);
}

#[test]
fn several_accumulators_come_back_through_the_tuple() {
    let script = write_script(
        "tuple_out.ol",
        "fn accs(n) = {\n\
             println(\"start\")\n\
             let mut a = 0\n\
             let mut b = 1\n\
             let mut i = 0\n\
             while i < n { a = (a + i) % 999983 b = (b + a) % 999983 i = i + 1 }\n\
             [a, b]\n\
         }\n\
         println(accs(100000))\n",
    );
    let (on_out, on_err) = run(&script, &[("OLANG_OSR_DEBUG", "1")]);
    let (off_out, _) = run(&script, &[("OLANG_OSR_OFF", "1")]);
    assert!(on_err.contains("loop entered natively"), "{}", on_err);
    assert_eq!(on_out, off_out);
}

#[test]
fn a_loop_over_a_moved_list_converts_and_agrees() {
    // The list crosses the tier boundary as an AstList handle (T2); the
    // OSR entry materializes it once and iterates natively.
    let script = write_script(
        "list_loop.ol",
        "fn total(xs) = {\n\
             println(\"start\")\n\
             let mut s = 0\n\
             for v in xs { s = (s + v * 7) % 1000003 }\n\
             s\n\
         }\n\
         let mut xs = []\n\
         for i in range(0, 50000) { xs = xs + [i] }\n\
         println(total(xs))\n",
    );
    let (on_out, on_err) = run(&script, &[("OLANG_OSR_DEBUG", "1")]);
    let (off_out, _) = run(&script, &[("OLANG_OSR_OFF", "1")]);
    assert!(on_err.contains("loop entered natively"), "{}", on_err);
    assert_eq!(on_out, off_out);
}

#[test]
fn a_mid_loop_error_reports_identically() {
    // The division fails at iteration 20000 — after the OSR threshold.
    // The native attempt is discarded and the VM re-raises with the
    // original spans; both runs must tell the same story.
    let script = write_script(
        "err_loop.ol",
        "fn risky(n) = {\n\
             println(\"start\")\n\
             let mut acc = 0\n\
             let mut i = 0\n\
             while i < n { acc = acc + 100 / (20000 - i) i = i + 1 }\n\
             acc\n\
         }\n\
         println(risky(50000))\n",
    );
    let (on_out, on_err) = run(&script, &[]);
    let (off_out, off_err) = run(&script, &[("OLANG_OSR_OFF", "1")]);
    assert_eq!(on_out, off_out);
    assert!(
        on_err.contains("Division by zero"),
        "the real error must surface: {}",
        on_err
    );
    assert_eq!(
        on_err, off_err,
        "the error report must be byte-identical across paths"
    );
}

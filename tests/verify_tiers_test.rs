//! `--verify-tiers <rate>` (W7): live tier self-verification. Native
//! results are re-executed on the VM dispatch and compared bit-for-bit
//! — sound because whitelisted native code is pure with respect to
//! caller-visible state. These tests pin the contract: verified runs
//! produce byte-identical output at every rate, and a divergence (here
//! forced through the self-test hook) aborts with the report.

use std::process::Command;

const PROGRAM: &str = "fn work(xs, n) = {\n\
     let mut acc = []\n\
     for i in 0..n { acc = acc + [xs[i] * 2.0 + 1.0] }\n\
     acc\n\
 }\n\
 fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)\n\
 let xs = map(0..20000, (i) => to_float(i))\n\
 let a = work(xs, 20000)\n\
 println(`${a[0]} ${a[19999]} ${len(a)} ${fib(22)}`)";

fn run(args: &[&str], envs: &[(&str, &str)]) -> (i32, String) {
    let dir = std::env::temp_dir().join("olang_verify_tiers_tests");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("probe.ol");
    std::fs::write(&path, PROGRAM).expect("write");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_olang"));
    cmd.args(args).arg("run").arg(&path);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), text)
}

#[test]
fn verified_runs_are_byte_identical_at_every_rate() {
    let (code, plain) = run(&[], &[]);
    assert_eq!(code, 0, "plain run failed:\n{plain}");
    for rate in ["1", "0.5", "0.01"] {
        let (code, verified) = run(&["--verify-tiers", rate], &[]);
        assert_eq!(code, 0, "verified run failed at rate {rate}:\n{verified}");
        assert_eq!(verified, plain, "output changed at rate {rate}");
    }
}

#[test]
fn a_divergence_aborts_with_the_report() {
    // The self-test hook flags every comparison as diverged, proving
    // the scream path end to end without needing a real engine bug.
    let (code, text) = run(&["--verify-tiers", "1"], &[("OLANG_VERIFY_SELFTEST", "1")]);
    assert_eq!(code, 102, "expected the divergence abort, got:\n{text}");
    assert!(text.contains("tier divergence"), "report missing:\n{text}");
    assert!(text.contains("native:"), "values missing:\n{text}");
    assert!(text.contains("engine bug"), "framing missing:\n{text}");
}

#[test]
fn an_out_of_range_rate_is_refused() {
    let (code, text) = run(&["--verify-tiers", "1.5"], &[]);
    assert_eq!(code, 2, "expected a usage error:\n{text}");
    assert!(text.contains("rate in 0..=1"), "{text}");
}

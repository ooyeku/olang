//! The consolidated example system — examples/demo, "Harborline" — must
//! stay healthy: every module parses, and a bounded, deterministic soak
//! run completes with its invariants intact. A language change that breaks
//! the demo is a CI failure, not a discovery a reader makes.
//!
//! (The full example sweep, including every package under examples/, runs
//! via `examples/run_all.ol`; this test pins the flagship.)

use olang::Parser;

fn parses(path: &str, source: &str) {
    Parser::new()
        .parse(source)
        .unwrap_or_else(|e| panic!("{} failed to PARSE: {}", path, e));
}

macro_rules! parse_test {
    ($name:ident, $file:literal) => {
        #[test]
        fn $name() {
            parses(
                concat!("examples/demo/", $file),
                include_str!(concat!("../examples/demo/", $file)),
            );
        }
    };
}

parse_test!(demo_main_parses, "main.ol");
parse_test!(demo_prelude_parses, "lib/prelude.ol");
parse_test!(demo_cargo_parses, "lib/cargo.ol");
parse_test!(demo_vessels_parses, "lib/vessels.ol");
parse_test!(demo_schedule_parses, "lib/schedule.ol");
parse_test!(demo_ledger_parses, "lib/ledger.ol");
parse_test!(demo_metrics_parses, "lib/metrics.ol");
parse_test!(demo_manifest_parses, "lib/manifest.ol");
parse_test!(demo_report_parses, "lib/report.ol");
parse_test!(demo_signing_parses, "lib/signing.ol");
parse_test!(demo_workers_parses, "lib/workers.ol");
parse_test!(demo_console_parses, "lib/console.ol");

/// Two bounded, deterministic simulated days, end to end: arrivals,
/// berthing, threaded unloading, tariff settlement, the signed digest
/// chain, and the daily invariant checks — exit 0 means every invariant
/// held. Runs the real binary, exactly as a user would.
#[test]
fn demo_soak_runs_clean() {
    let demo_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/demo");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_olang"))
        .current_dir(&demo_dir)
        .args([
            "main.ol", "--ticks", "48", "--fast", "--quiet", "--seed", "7",
        ])
        .output()
        .expect("failed to launch the demo");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "demo exited nonzero\n--- stdout ---\n{}\n--- stderr ---\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("all invariants held"),
        "missing invariant verdict\n{}",
        stdout
    );
    assert!(
        stdout.contains("HARBORLINE SHUTDOWN"),
        "missing shutdown summary\n{}",
        stdout
    );
    // Deterministic under a fixed seed: the run always serves the same
    // number of vessels for the same voyage.
    assert!(
        stdout.contains("vessels served: 16"),
        "seeded run drifted\n{}",
        stdout
    );
}

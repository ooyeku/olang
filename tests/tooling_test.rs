//! The `olang test` runner and `olang fmt` formatter, exercised through the
//! real binary the way a user runs them.

use std::path::PathBuf;
use std::process::Command;

fn olang() -> Command {
    Command::new(env!("CARGO_BIN_EXE_olang"))
}

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_tooling_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ── olang test ─────────────────────────────────────────────────────────

#[test]
fn test_runner_reports_passes_and_failures_and_exits_nonzero() {
    let dir = fixture_dir("mixed");
    std::fs::write(
        dir.join("math_test.ol"),
        r#"
fn double(x) = x * 2

test "doubling works" {
    assert_eq(double(2), 4)
}

test "deliberately red" {
    assert_eq(double(3), 7, "wrong on purpose")
}

test "runs after a failure" {
    assert_true(double(1) == 2)
}
"#,
    )
    .unwrap();

    let out = olang().arg("test").arg(&dir).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "failures must exit nonzero");
    assert!(stdout.contains("✓ doubling works"), "got: {stdout}");
    assert!(stdout.contains("✗ deliberately red"), "got: {stdout}");
    assert!(stdout.contains("wrong on purpose"), "got: {stdout}");
    assert!(
        stdout.contains("✓ runs after a failure"),
        "a failing block must not stop later blocks: {stdout}"
    );
    assert!(stdout.contains("2 passed, 1 failed"), "got: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_runner_green_suite_exits_zero() {
    let dir = fixture_dir("green");
    std::fs::write(
        dir.join("ok_test.ol"),
        "test \"arithmetic\" {\n    assert_eq(1 + 1, 2)\n}\n",
    )
    .unwrap();
    let out = olang().arg("test").arg(&dir).output().expect("run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed, 0 failed"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_runner_ignores_files_without_test_blocks() {
    let dir = fixture_dir("none");
    std::fs::write(dir.join("app.ol"), "println(\"side effect\")\n").unwrap();
    let out = olang().arg("test").arg(&dir).output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(stdout.contains("no test blocks found"), "got: {stdout}");
    assert!(
        !stdout.contains("side effect"),
        "files without tests must not execute: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── olang fmt ──────────────────────────────────────────────────────────

#[test]
fn fmt_check_flags_and_write_fixes_idempotently() {
    let dir = fixture_dir("fmt");
    let file = dir.join("messy.ol");
    std::fs::write(&file, "let x = 1   \n\n\n\n\tprintln(show(x))\n\n\n").unwrap();

    // --check: nonzero, does not modify
    let out = olang()
        .arg("fmt")
        .arg("--check")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("would reformat"));
    assert!(std::fs::read_to_string(&file).unwrap().contains("   \n"));

    // write mode fixes
    let out = olang().arg("fmt").arg(&dir).output().unwrap();
    assert!(out.status.success());
    let formatted = std::fs::read_to_string(&file).unwrap();
    // trailing ws stripped, blanks collapsed, leading tab -> 4 spaces
    // (indentation is converted, never removed), one final newline
    assert_eq!(formatted, "let x = 1\n\n    println(show(x))\n");

    // now clean and idempotent
    let out = olang()
        .arg("fmt")
        .arg("--check")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_leaves_multiline_template_strings_alone() {
    let dir = fixture_dir("fmt_tmpl");
    let file = dir.join("tmpl.ol");
    let src = "let t = `content   \n\ttab and trailing   \n\n\nkept`\nprintln(t)\n";
    std::fs::write(&file, src).unwrap();
    let out = olang().arg("fmt").arg(&dir).output().unwrap();
    assert!(out.status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), src);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_reports_and_fails_on_files_that_do_not_parse() {
    let dir = fixture_dir("fmt_bad");
    let file = dir.join("broken.ol");
    std::fs::write(&file, "let = = 1   \n").unwrap();
    let out = olang().arg("fmt").arg(&dir).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    // An unparseable file is a real error fmt must surface and fail on, not
    // silently claim "all formatted".
    assert!(stdout.contains("does not parse"), "got: {stdout}");
    assert!(
        !out.status.success(),
        "fmt should exit nonzero on an unparseable file"
    );
    // The broken file is never rewritten.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "let = = 1   \n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_and_fmt_fail_on_missing_paths() {
    // A named path that doesn't exist must be an error, not silent success.
    let t = olang()
        .arg("test")
        .arg("/no/such/path.ol")
        .output()
        .unwrap();
    assert!(!t.status.success(), "test on a missing path should fail");
    let f = olang().arg("fmt").arg("/no/such/path.ol").output().unwrap();
    assert!(!f.status.success(), "fmt on a missing path should fail");
}

#[test]
fn test_fails_on_a_file_that_does_not_parse() {
    let dir = fixture_dir("test_bad");
    let file = dir.join("broken.ol");
    std::fs::write(&file, "fn oops( = 1\n").unwrap();
    let out = olang().arg("test").arg(&file).output().unwrap();
    assert!(
        !out.status.success(),
        "a syntax error in a test file must fail the run, not pass CI"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── olang bench ────────────────────────────────────────────────────────

#[test]
fn bench_measures_saves_and_compares_a_baseline() {
    let dir = fixture_dir("bench");
    std::fs::write(
        dir.join("tiny.ol"),
        "fn f(n) = if n < 2 => n else => f(n - 1) + f(n - 2)\nprintln(f(15))\n",
    )
    .unwrap();
    let baseline = dir.join("base.json");

    // Measure and save.
    let out = olang()
        .args([
            "bench",
            dir.to_str().unwrap(),
            "--runs",
            "2",
            "--save",
            baseline.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("tiny"), "no row for the benchmark: {text}");
    assert!(baseline.exists(), "baseline not written");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&baseline).unwrap()).unwrap();
    assert!(doc["benchmarks"]["tiny"]["median_s"].as_f64().unwrap() > 0.0);

    // Compare against it: same program, so the row must read as noise-level.
    let out = olang()
        .args([
            "bench",
            dir.to_str().unwrap(),
            "--runs",
            "2",
            "--against",
            baseline.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    // Any comparison verdict counts — tiny debug-build runs jitter enough
    // that the row may legitimately read faster/slower rather than "~".
    assert!(
        text.contains("vs") || text.contains("than"),
        "comparison column missing: {text}"
    );
}

#[test]
fn bench_fail_on_regress_returns_nonzero_against_a_faster_baseline() {
    let dir = fixture_dir("bench_regress");
    std::fs::write(
        dir.join("steady.ol"),
        "fn f(n) = if n < 2 => n else => f(n - 1) + f(n - 2)\nprintln(f(18))\n",
    )
    .unwrap();
    let baseline = dir.join("base.json");
    // A baseline claiming the benchmark once ran absurdly fast: the real
    // run must register as a regression and fail under the flag.
    std::fs::write(
        &baseline,
        r#"{"benchmarks": {"steady": {"median_s": 0.00001}}}"#,
    )
    .unwrap();
    let out = olang()
        .args([
            "bench",
            dir.join("steady.ol").to_str().unwrap(),
            "--runs",
            "2",
            "--against",
            baseline.to_str().unwrap(),
            "--fail-on-regress",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "expected regression exit");
    assert!(String::from_utf8_lossy(&out.stdout).contains("slower"));
}

#[test]
fn bench_fails_cleanly_on_broken_programs_and_bad_paths() {
    let dir = fixture_dir("bench_broken");
    std::fs::write(dir.join("boom.ol"), "println(1 / 0)\n").unwrap();
    let out = olang()
        .args([
            "bench",
            dir.join("boom.ol").to_str().unwrap(),
            "--runs",
            "1",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("boom"));

    let out = olang()
        .args(["bench", "/no/such/path.ol"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// `olang test` answers for the project's own files: its `lib/` modules'
/// blocks run (imported or walked), a dependency's do not — Foundry's 13
/// tests were reported as 133, the rest Shuttle's. `--deps` runs them.
#[test]
fn test_runner_runs_the_projects_own_blocks_and_dependencies_only_with_deps() {
    let ws = fixture_dir("own_deps");
    let write = |rel: &str, text: &str| {
        let p = ws.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write(
        "mathlib/olang.toml",
        "[package]\nname = \"mathlib\"\nversion = \"1.0.0\"\n",
    );
    write(
        "mathlib/index.ol",
        "share fn square(x) = x * x\n\ntest \"the dependency's block\" {\n    assert_eq(square(3), 9)\n}\n",
    );
    write(
        "app/olang.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmathlib = { path = \"../mathlib\" }\n",
    );
    write(
        "app/lib/util.ol",
        "share fn twice(x) = x * 2\n\ntest \"the lib module's block\" {\n    assert_eq(twice(2), 4)\n}\n",
    );
    write(
        "app/tests/app_test.ol",
        "use mathlib { square }\nuse lib.util { twice }\n\ntest \"the app's block\" {\n    assert_eq(twice(square(2)), 8)\n}\n",
    );
    let app = ws.join("app");

    // The whole project, and its tests/ directory alone: the lib module's
    // block runs either way (walked, or at import), the dependency's never.
    for target in [app.clone(), app.join("tests")] {
        let out = olang()
            .arg("test")
            .arg(&target)
            .current_dir(&app)
            .output()
            .expect("run");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{}: {stdout}", target.display());
        assert!(stdout.contains("✓ the app's block"), "{stdout}");
        assert!(stdout.contains("✓ the lib module's block"), "{stdout}");
        assert!(
            !stdout.contains("the dependency's block"),
            "a dependency's block ran without --deps: {stdout}"
        );
        assert!(stdout.contains("2 passed, 0 failed"), "{stdout}");
    }

    let out = olang()
        .arg("test")
        .arg("--deps")
        .arg(&app)
        .current_dir(&app)
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("✓ the dependency's block"), "{stdout}");
    assert!(stdout.contains("3 passed, 0 failed"), "{stdout}");
    let _ = std::fs::remove_dir_all(&ws);
}

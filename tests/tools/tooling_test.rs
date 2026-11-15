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

// ── olang test ──────────────────────────────────────────────────────
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

/// `--only` runs no file without a matching block, and a file error is
/// printed under its own file's heading (OL-349).
#[test]
fn test_only_leaves_out_files_without_a_match_and_heads_a_file_error() {
    let dir = fixture_dir("only_setup");
    std::fs::write(
        dir.join("a_test.ol"),
        "test \"doubling works\" { assert_eq(2 * 2, 4) }\ntest \"after red\" { assert_true(true) }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b_test.ol"),
        "let x = 1 / 0\ntest \"never\" { assert_true(true) }\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        let out = olang()
            .arg("test")
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let (code, stdout) = run(&["--only", "after"]);
    assert_eq!(code, Some(0), "{stdout}");
    assert!(stdout.contains("1 passed, 0 failed"), "{stdout}");
    assert!(!stdout.contains("b_test"), "{stdout}");

    let (code, stdout) = run(&["--only", "zzz", "--fail-if-none"]);
    assert_eq!(code, Some(1), "{stdout}");
    assert!(stdout.contains("0 passed, 0 failed"), "{stdout}");

    // a file that does match: its setup error sits under its own heading
    let (code, stdout) = run(&["--only", "never"]);
    assert_eq!(code, Some(1), "{stdout}");
    let heading = stdout.find("b_test.ol").expect(&stdout);
    let error = stdout.find("file error").expect(&stdout);
    assert!(heading < error, "{stdout}");

    let out = olang()
        .args(["test", "--only", "after", "--format", "json"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(
        text.contains(r#""event":"skipped""#) && text.contains("never"),
        "{text}"
    );
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

// ── olang fmt ───────────────────────────────────────────────────────
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

// ── olang bench ─────────────────────────────────────────────────────
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

/// One failing file is reported with the child's own error and the run
/// goes on to the rest, ending with `finished` and exit 1 (OL-366).
#[test]
fn bench_reports_a_failing_file_and_times_the_others() {
    let dir = fixture_dir("bench_goes_on");
    std::fs::write(dir.join("bad.ol"), "println(1 / 0)\n").unwrap();
    std::fs::write(dir.join("good.ol"), "println(1 + 1)\n").unwrap();
    let out = olang()
        .args([
            "bench",
            "bad.ol",
            "good.ol",
            "--runs",
            "1",
            "--save",
            "base.json",
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stdout}{stderr}");
    assert!(
        stderr.contains("Division by zero"),
        "the child's reason is lost: {stderr}"
    );
    assert!(
        stdout.contains("good"),
        "the file after the failure was never timed: {stdout}"
    );
    assert!(
        stderr.contains("1 of 2 file(s) failed; no baseline saved"),
        "{stderr}"
    );
    assert!(
        !dir.join("base.json").exists(),
        "a partial baseline was saved"
    );

    let out = olang()
        .args([
            "bench", "bad.ol", "good.ol", "--runs", "1", "--format", "json",
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let events: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        ["start", "error", "result", "finished"],
        "{events:?}"
    );
    assert!(
        events[1]["message"]
            .as_str()
            .unwrap()
            .contains("Division by zero"),
        "{events:?}"
    );
    assert_eq!(events[3]["failed"], 1);
    assert_eq!(events[3]["files"], 1);
}

/// `olang test` answers for the project's own files: its `lib/` modules'
/// blocks run (imported or walked), a dependency's do not — a CI runner's 13
/// tests were reported as 133, the rest a web framework's. `--deps` runs them.
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

#[test]
fn a_walk_leaves_out_a_project_below_the_root_unless_it_is_named() {
    let ws = fixture_dir("nested_projects");
    let write = |rel: &str, text: &str| {
        let p = ws.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write(
        "app/olang.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    );
    write(
        "app/tests/app_test.ol",
        "test \"the app's block\" {\n    assert_eq(1 + 1, 2)\n}\n",
    );
    // A fixture project inside the app: a deliberately failing test, an
    // unformatted file and one that does not parse.
    let fixture = "app/tests/fixtures/sample";
    write(
        &format!("{fixture}/olang.toml"),
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\n",
    );
    write(
        &format!("{fixture}/tests/sample_test.ol"),
        "test \"the fixture's failing block\" {\n    assert_eq(1, 2)\n}\n",
    );
    write(&format!("{fixture}/messy.ol"), "let x = 1   \n\n\n\n");
    write(&format!("{fixture}/broken.ol"), "let = = 1\n");
    let app = ws.join("app");
    let run = |dir: &std::path::Path, args: &[&str]| {
        let out = olang().args(args).current_dir(dir).output().expect("run");
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    };

    // From the app's root, bare: the fixture is another project's.
    let (ok, out) = run(&app, &["test"]);
    assert!(ok, "{out}");
    assert!(out.contains("1 passed, 0 failed"), "{out}");
    assert!(!out.contains("the fixture's failing block"), "{out}");
    let (ok, out) = run(&app, &["fmt", "--check"]);
    assert!(ok, "fmt walked into the fixture: {out}");
    let (ok, out) = run(&app, &["check"]);
    assert!(ok, "check walked into the fixture: {out}");
    assert_eq!(
        std::fs::read_to_string(
            app.join(fixture.trim_start_matches("app/"))
                .join("messy.ol")
        )
        .unwrap(),
        "let x = 1   \n\n\n\n",
        "the fixture was rewritten"
    );

    // Named, it is walked: from the app's root or its own.
    let (ok, out) = run(&app, &["test", "tests/fixtures/sample"]);
    assert!(!ok && out.contains("1 failed"), "{out}");
    let (ok, _) = run(&app, &["fmt", "--check", "tests/fixtures/sample"]);
    assert!(!ok, "a named fixture is formatted");
    let (ok, _) = run(&app.join("tests/fixtures/sample"), &["check"]);
    assert!(!ok, "a fixture checked from its own root");

    // Outside any project a walk enters every project below it.
    let (ok, out) = run(&ws, &["test", "."]);
    assert!(!ok && out.contains("1 passed, 1 failed"), "{out}");
    let _ = std::fs::remove_dir_all(&ws);
}

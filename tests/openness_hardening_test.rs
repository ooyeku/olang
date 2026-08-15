//! The 0.59.0 openness-hardening pass: whole-payload checksums,
//! provenance-vs-source (`inspect --against`), the capability profiler
//! (`run --trace-caps`), and project-authored lints (`check --rules`).
//! End-to-end: every test invokes the built `olang` binary.

use std::process::Command;

fn olang_bin() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn tmp(sub: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("olang_openness_{}_{}", sub, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// O1a: the transparency digest covers source + manifest + lockfile, so
/// editing the embedded capability grant fails `--verify` — the source sha
/// alone never noticed that.
#[test]
fn verify_catches_a_widened_capability_grant() {
    let dir = tmp("digest");
    std::fs::write(
        dir.join("olang.toml"),
        "[package]\nname = \"greet\"\nversion = \"0.1.0\"\n\n[capabilities]\nfs = \"read\"\nnet = false\n",
    )
    .unwrap();
    std::fs::write(dir.join("olang.lock"), "lock-v1\n").unwrap();
    let src = dir.join("main.ol");
    std::fs::write(&src, "print(\"hi\")\n").unwrap();
    let out = dir.join("greet");

    let build = Command::new(olang_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run olang build");
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    // A pristine binary verifies.
    let ok = Command::new(olang_bin())
        .args(["inspect", out.to_str().unwrap(), "--verify"])
        .output()
        .expect("inspect --verify");
    assert!(ok.status.success(), "pristine binary should verify");
    let ok_text = String::from_utf8_lossy(&ok.stdout);
    assert!(ok_text.contains("digest:"), "should print a digest line");

    // Widen the embedded manifest in place (same byte length), then verify
    // fails — the manifest is part of the digest.
    let mut bytes = std::fs::read(&out).unwrap();
    let needle = b"net = false";
    let repl = b"net = true ";
    let pos = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("embedded manifest present");
    bytes[pos..pos + needle.len()].copy_from_slice(repl);
    std::fs::write(&out, &bytes).unwrap();

    let tampered = Command::new(olang_bin())
        .args(["inspect", out.to_str().unwrap(), "--verify"])
        .output()
        .expect("inspect --verify tampered");
    assert!(
        !tampered.status.success(),
        "a widened grant must fail --verify"
    );
    assert!(
        String::from_utf8_lossy(&tampered.stdout).contains("MISMATCH"),
        "stdout: {}",
        String::from_utf8_lossy(&tampered.stdout)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// O1b: `--against DIR` proves the binary was built from that source tree —
/// match on the real tree, mismatch when the source differs.
#[test]
fn against_proves_provenance_from_a_source_tree() {
    let dir = tmp("against");
    let src = dir.join("main.ol");
    std::fs::write(&src, "print(\"provenance\")\n").unwrap();
    let out = dir.join("prog");

    let build = Command::new(olang_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run olang build");
    assert!(build.status.success());

    // Against the real tree: match, exit 0.
    let good = Command::new(olang_bin())
        .args([
            "inspect",
            out.to_str().unwrap(),
            "--against",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("inspect --against");
    assert!(good.status.success(), "should match its own source tree");
    assert!(String::from_utf8_lossy(&good.stdout).contains("matches the source tree"));

    // Against a modified tree: mismatch, nonzero.
    let other = tmp("against_other");
    std::fs::write(other.join("main.ol"), "print(\"different\")\n").unwrap();
    let bad = Command::new(olang_bin())
        .args([
            "inspect",
            out.to_str().unwrap(),
            "--against",
            other.to_str().unwrap(),
        ])
        .output()
        .expect("inspect --against other");
    assert!(!bad.status.success(), "a changed source must not match");
    assert!(String::from_utf8_lossy(&bad.stdout).contains("DIFFERS"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&other);
}

/// O1c: `--trace-caps` reports the capabilities a run exercised and prints a
/// least-privilege manifest — read-only fs use suggests `fs = "read"` and
/// pins everything untouched shut.
#[test]
fn trace_caps_profiles_and_suggests_a_minimal_manifest() {
    let dir = tmp("trace");
    let src = dir.join("prof.ol");
    std::fs::write(&src, "let home = os.get_env(\"HOME\")\nprint(\"ok\")\n").unwrap();

    let run = Command::new(olang_bin())
        .args(["--trace-caps", src.to_str().unwrap()])
        .output()
        .expect("run --trace-caps");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "clean run should exit 0");
    assert!(stdout.contains("capability profile"), "stdout: {}", stdout);
    // env was exercised; nothing else.
    assert!(stdout.contains("env = true"), "stdout: {}", stdout);
    assert!(stdout.contains("net = false"), "stdout: {}", stdout);
    assert!(stdout.contains("fs = false"), "stdout: {}", stdout);

    let _ = std::fs::remove_dir_all(&dir);
}

/// O6: `check --rules` runs project-authored lints over the meta AST and
/// counts findings as problems (exit 1), leaving clean files clean.
#[test]
fn check_rules_runs_project_lints_over_the_meta_ast() {
    let dir = tmp("rules");
    // A rule: forbid bare unwrap(), reporting the offending line.
    std::fs::write(
        dir.join("rules.ol"),
        r#"
share fn rule_no_bare_unwrap(nodes) =
    nodes
      |> filter((n) => map_get(n, "kind") == "call" && map_get(n, "target") == "unwrap")
      |> map((n) => #{ "message": "bare unwrap()", "line": map_get(n, "line") })
"#,
    )
    .unwrap();
    let offender = dir.join("app.ol");
    std::fs::write(
        &offender,
        "fn load(p) = {\n    let a = fs.read_file(p).unwrap()\n    a\n}\n",
    )
    .unwrap();
    let clean = dir.join("clean.ol");
    std::fs::write(&clean, "fn add(a: Int, b: Int) = a + b\n").unwrap();

    let rules = dir.join("rules.ol");

    // The offender: one finding, exit 1, correct line.
    let hit = Command::new(olang_bin())
        .args([
            "check",
            offender.to_str().unwrap(),
            "--rules",
            rules.to_str().unwrap(),
        ])
        .output()
        .expect("check --rules");
    assert_eq!(hit.status.code(), Some(1), "a finding should exit 1");
    let hit_err = String::from_utf8_lossy(&hit.stderr);
    assert!(
        hit_err.contains("[rule_no_bare_unwrap]") && hit_err.contains(":2:"),
        "stderr: {}",
        hit_err
    );

    // The clean file: no findings, exit 0.
    let ok = Command::new(olang_bin())
        .args([
            "check",
            clean.to_str().unwrap(),
            "--rules",
            rules.to_str().unwrap(),
        ])
        .output()
        .expect("check --rules clean");
    assert!(ok.status.success(), "a clean file should pass");

    // A rules file with no rule_* functions is an error, not a silent pass.
    std::fs::write(dir.join("norules.ol"), "share fn helper(x) = x\n").unwrap();
    let norules = Command::new(olang_bin())
        .args([
            "check",
            clean.to_str().unwrap(),
            "--rules",
            dir.join("norules.ol").to_str().unwrap(),
        ])
        .output()
        .expect("check --rules norules");
    assert_eq!(norules.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&norules.stderr).contains("no rule_* functions"),
        "stderr: {}",
        String::from_utf8_lossy(&norules.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

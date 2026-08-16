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

/// A built binary records the platform it was built on, and `inspect`
/// reports it. A native binary only runs on its own OS/arch, so the line —
/// and its "this platform" marker on the machine that built it — tells you
/// whether a binary that arrived from elsewhere will run here.
#[test]
fn inspect_reports_the_build_platform() {
    let dir = tmp("platform");
    let src = dir.join("main.ol");
    std::fs::write(&src, "println(\"hi\")\n").unwrap();
    let out = dir.join("tool");

    let build = Command::new(olang_bin())
        .args(["build", src.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run olang build");
    assert!(build.status.success());

    let inspect = Command::new(olang_bin())
        .args(["inspect", out.to_str().unwrap()])
        .output()
        .expect("inspect");
    let text = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        text.contains("built on:"),
        "inspect should report the build platform; got:\n{text}"
    );
    // Built here and inspected here: the OS/arch match, so it is flagged as
    // the current platform.
    assert!(
        text.contains(std::env::consts::OS) && text.contains(std::env::consts::ARCH),
        "should name this machine's os/arch; got:\n{text}"
    );
    assert!(
        text.contains("[this platform]"),
        "a binary built and inspected on the same machine is this platform; got:\n{text}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// C1 regression: the digest binds the executed AST, so a binary whose AST
/// was swapped while its source was left clean fails `--verify` — on both
/// the digest and the source-faithfulness check. Without the fix, `--verify`
/// passed while `./binary` ran a different program than `--source` showed.
#[test]
fn verify_catches_a_swapped_ast() {
    // Parse the transparent-binary footer:
    // [source][ast][meta][src_len u64][ast_len u64][meta_len u64][8-byte magic].
    fn regions(bytes: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        assert_eq!(
            &bytes[bytes.len() - 8..],
            b"oLaNgMeT",
            "not a format-3 bundle"
        );
        let lens = &bytes[bytes.len() - 32..bytes.len() - 8];
        let u = |o: usize| u64::from_le_bytes(lens[o..o + 8].try_into().unwrap()) as usize;
        let (sl, al, ml) = (u(0), u(8), u(16));
        let payload = sl + al + ml;
        let base = bytes.len() - 32 - payload;
        let body = bytes[..base].to_vec();
        let src = bytes[base..base + sl].to_vec();
        let ast = bytes[base + sl..base + sl + al].to_vec();
        let meta = bytes[base + sl + al..base + sl + al + ml].to_vec();
        (body, src, ast, meta)
    }

    let dir = tmp("astswap");
    std::fs::write(dir.join("safe.ol"), "print(\"SAFE\")\n").unwrap();
    std::fs::write(dir.join("evil.ol"), "print(\"PWNED\")\n").unwrap();
    for name in ["safe", "evil"] {
        let s = Command::new(olang_bin())
            .current_dir(&dir)
            .args(["build", &format!("{name}.ol"), "-o", name])
            .output()
            .unwrap();
        assert!(s.status.success(), "build {name} failed");
    }

    let (body, safe_src, _safe_ast, safe_meta) = regions(&std::fs::read(dir.join("safe")).unwrap());
    let (_, _, evil_ast, _) = regions(&std::fs::read(dir.join("evil")).unwrap());

    // Splice: safe runtime + safe SOURCE + safe META + EVIL AST.
    let mut franken = body;
    franken.extend_from_slice(&safe_src);
    franken.extend_from_slice(&evil_ast);
    franken.extend_from_slice(&safe_meta);
    franken.extend_from_slice(&(safe_src.len() as u64).to_le_bytes());
    franken.extend_from_slice(&(evil_ast.len() as u64).to_le_bytes());
    franken.extend_from_slice(&(safe_meta.len() as u64).to_le_bytes());
    franken.extend_from_slice(b"oLaNgMeT");
    let fpath = dir.join("franken");
    std::fs::write(&fpath, &franken).unwrap();

    // --source still shows the clean source, but --verify must reject.
    let src = Command::new(olang_bin())
        .args(["inspect", fpath.to_str().unwrap(), "--source"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&src.stdout).contains("SAFE"));

    let verify = Command::new(olang_bin())
        .args(["inspect", fpath.to_str().unwrap(), "--verify"])
        .output()
        .unwrap();
    assert!(!verify.status.success(), "a swapped AST must fail --verify");
    let out = String::from_utf8_lossy(&verify.stdout);
    assert!(out.contains("MISMATCH"), "digest should mismatch: {out}");
    assert!(
        out.contains("DIVERGES"),
        "source should not match AST: {out}"
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

/// S5: `meta.parse` emits every AST child, so a lint over the node tree
/// sees calls hidden in match arms, `await`, map/struct literals, template
/// interpolations, and assertions. Before the fix these positions collapsed
/// to a summary node and a "no bare unwrap" rule returned clean on unsafe
/// code.
#[test]
fn rules_see_calls_in_previously_dropped_positions() {
    let dir = tmp("s5");
    std::fs::write(
        dir.join("rules.ol"),
        "share fn rule_no_bare_unwrap(nodes) =\n    nodes\n      |> filter((n) => map_get(n, \"kind\") == \"call\" && map_get(n, \"target\") == \"unwrap\")\n      |> map((n) => #{ \"message\": \"bare unwrap()\", \"line\": map_get(n, \"line\") })\n",
    )
    .unwrap();
    // Seven bare unwrap() calls, each in a position meta.parse used to drop:
    // two match arms, a map literal, a template interpolation, an await, and
    // both sides of an assert_eq.
    std::fs::write(
        dir.join("app.ol"),
        "fn a(v) = match v {\n    1 => unwrap(z),\n    _ => unwrap(other)\n}\nfn b(v) = #{ \"k\": unwrap(danger) }\nfn d(v) = `val ${unwrap(sneaky)}`\nfn e(v) = await unwrap(promised)\nfn f(v) = assert_eq(unwrap(a), unwrap(b))\n",
    )
    .unwrap();

    let out = Command::new(olang_bin())
        .current_dir(&dir)
        .args(["check", "app.ol", "--rules", "rules.ol"])
        .output()
        .expect("run check --rules");
    let findings = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("[rule_no_bare_unwrap]"))
        .count();
    assert_eq!(
        findings,
        7,
        "every hidden unwrap should be found; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// OM3: `--trace-caps --write` folds the suggested least-privilege
/// `[capabilities]` block into the package's olang.toml, and never
/// overwrites an existing one.
#[test]
fn trace_caps_write_authors_the_manifest() {
    let dir = tmp("write");
    std::fs::write(
        dir.join("olang.toml"),
        "[package]\nname = \"prof\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let src = dir.join("main.ol");
    std::fs::write(&src, "let h = os.get_env(\"HOME\")\nprint(\"ok\")\n").unwrap();

    // First run writes the block matching what the program used (env only).
    let first = Command::new(olang_bin())
        .current_dir(&dir)
        .args(["--trace-caps", "--write", "main.ol"])
        .output()
        .expect("run --trace-caps --write");
    assert!(first.status.success());
    assert!(
        String::from_utf8_lossy(&first.stderr).contains("wrote [capabilities]"),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let toml = std::fs::read_to_string(dir.join("olang.toml")).unwrap();
    assert!(toml.contains("[capabilities]"), "toml: {}", toml);
    assert!(toml.contains("env = true"), "toml: {}", toml);
    assert!(toml.contains("net = false"), "toml: {}", toml);

    // Second run must not overwrite the hand-editable block.
    let second = Command::new(olang_bin())
        .current_dir(&dir)
        .args(["--trace-caps", "--write", "main.ol"])
        .output()
        .expect("run --trace-caps --write again");
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("already has a [capabilities] block"),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// OM3: `olang caps` shows the declared grant of a source package,
/// including per-dependency attenuation — the static counterpart to
/// `--trace-caps`.
#[test]
fn caps_shows_the_declared_grant() {
    let dir = tmp("capsview");
    std::fs::write(
        dir.join("olang.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
         [capabilities]\nfs = true\n\n\
         [capabilities.dependencies.lib]\nfs = false\nnet = false\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let out = Command::new(olang_bin())
        .current_dir(&dir)
        .arg("caps")
        .output()
        .expect("run olang caps");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("package app: fs=true"),
        "stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("dependency lib: fs=false net=false"),
        "stdout: {}",
        stdout
    );

    // A directory with no manifest reports full capability.
    let bare = tmp("capsbare");
    let none = Command::new(olang_bin())
        .current_dir(&bare)
        .arg("caps")
        .output()
        .expect("run olang caps bare");
    assert!(
        String::from_utf8_lossy(&none.stdout).contains("full capability"),
        "stdout: {}",
        String::from_utf8_lossy(&none.stdout)
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&bare);
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

/// S7: `check --rules` runs the rules file in a sandbox with no
/// capabilities, so a hostile rules.ol cannot touch the filesystem, network,
/// or processes when it is loaded.
#[test]
fn check_rules_sandboxes_the_rules_file() {
    let dir = tmp("rulesbox");
    std::fs::write(
        dir.join("rules.ol"),
        "let _ = fs.read_file(\"/etc/passwd\")\nshare fn rule_noop(nodes) = []\n",
    )
    .unwrap();
    std::fs::write(dir.join("app.ol"), "fn a() = 1\n").unwrap();
    let out = Command::new(olang_bin())
        .current_dir(&dir)
        .args(["check", "app.ol", "--rules", "rules.ol"])
        .output()
        .expect("run check --rules");
    assert!(
        !out.status.success(),
        "a rule reaching for fs must be denied"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("capability 'fs' denied"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

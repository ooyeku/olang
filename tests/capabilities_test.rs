//! Capability enforcement end to end: a manifest grant restricts the
//! effectful surface, per-dependency attenuation shrinks a dependency's
//! reach below the app's, and --deny restricts any run — all enforced at
//! the stdlib module boundary and attributed to the package that made the
//! call. Runs the real binary, as a user would.

use std::path::PathBuf;
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_caps_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &std::path::Path, s: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, s).unwrap();
}

/// S2: `db` must not be a latent filesystem capability. With `db=true,
/// fs=false`, opening a *file* database (which can create/write it) and
/// running `ATTACH` (which reaches an arbitrary path) are both denied,
/// while an in-memory database — which touches no file — is allowed.
#[test]
fn db_is_not_a_latent_filesystem_capability() {
    let ws = workspace("dbfs");
    write(
        &ws.join("olang.toml"),
        "[package]\nname = \"t\"\nversion = \"1.0.0\"\n\n[capabilities]\ndb = true\nfs = false\n",
    );

    // A file database is denied under fs=false.
    write(
        &ws.join("file.ol"),
        "unwrap(db.open(\"/tmp/olang_caps_evil.db\"))\n",
    );
    let file = Command::new(olang())
        .current_dir(&ws)
        .arg("file.ol")
        .output()
        .unwrap();
    assert!(!file.status.success(), "file db.open must be denied");
    assert!(
        String::from_utf8_lossy(&file.stderr).contains("capability 'fs' denied"),
        "stderr: {}",
        String::from_utf8_lossy(&file.stderr)
    );

    // ATTACH from an in-memory db is denied too.
    write(
        &ws.join("attach.ol"),
        "let c = unwrap(db.open(\":memory:\"))\nunwrap(db.execute(c, \"ATTACH DATABASE '/tmp/olang_caps_evil.db' AS e\"))\n",
    );
    let attach = Command::new(olang())
        .current_dir(&ws)
        .arg("attach.ol")
        .output()
        .unwrap();
    assert!(!attach.status.success(), "ATTACH must be denied");
    assert!(
        String::from_utf8_lossy(&attach.stderr).contains("capability 'fs' denied"),
        "stderr: {}",
        String::from_utf8_lossy(&attach.stderr)
    );

    // An in-memory database touches no file — allowed.
    write(
        &ws.join("mem.ol"),
        "unwrap(db.open(\":memory:\"))\nprintln(\"ok\")\n",
    );
    let mem = Command::new(olang())
        .current_dir(&ws)
        .arg("mem.ol")
        .output()
        .unwrap();
    assert!(
        mem.status.success(),
        "in-memory db should be allowed: {}",
        String::from_utf8_lossy(&mem.stderr)
    );

    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn manifest_grant_denies_writes_under_fs_read() {
    let ws = workspace("fsread");
    write(
        &ws.join("olang.toml"),
        "[package]\nname = \"t\"\nversion = \"1.0.0\"\n\n[capabilities]\nfs = \"read\"\n",
    );
    write(
        &ws.join("main.ol"),
        "println(show(len(unwrap(fs.list_dir(\".\")))))\nunwrap(fs.write_file(\"x\", \"y\"))\n",
    );
    let out = Command::new(olang())
        .current_dir(&ws)
        .arg("main.ol")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "write should be denied under fs=read"
    );
    assert!(
        err.contains("capability 'fs' denied") && err.contains("fs.write_file"),
        "unexpected: {}",
        err
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn dependency_attenuation_is_stricter_than_the_app() {
    let ws = workspace("atten");
    write(
        &ws.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n",
    );
    write(
        &ws.join("lib/index.ol"),
        "share fn peek() = unwrap(fs.read_file(\"/etc/hosts\"))\n",
    );
    write(
        &ws.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n\n\
         [dependencies]\nlib = { path = \"../lib\" }\n\n\
         [capabilities]\nfs = \"read\"\n\n\
         [capabilities.dependencies.lib]\nfs = false\n",
    );
    write(
        &ws.join("app/main.ol"),
        "use lib { peek }\n\
         println(show(len(unwrap(fs.read_file(\"olang.toml\")))))\n\
         peek()\n",
    );
    let out = Command::new(olang())
        .current_dir(ws.join("app"))
        .arg("main.ol")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // the app's own read succeeded...
    assert!(
        stdout.trim().parse::<i64>().is_ok(),
        "app read failed: {}",
        stdout
    );
    // ...but the dependency's read was denied, and named the dependency
    assert!(!out.status.success());
    assert!(
        stderr.contains("dependency 'lib'") && stderr.contains("fs.read_file"),
        "unexpected: {}",
        stderr
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn deny_flag_restricts_a_bare_script() {
    let ws = workspace("deny");
    write(&ws.join("s.ol"), "unwrap(fs.read_file(\"s.ol\"))\n");
    let out = Command::new(olang())
        .current_dir(&ws)
        .args(["--deny", "fs", "s.ol"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        err.contains("capability 'fs' denied"),
        "unexpected: {}",
        err
    );
    // typos in --deny refuse to run rather than running wide open
    let bad = Command::new(olang())
        .current_dir(&ws)
        .args(["--deny", "netz", "s.ol"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn a_bad_attenuation_target_refuses_to_run() {
    let ws = workspace("ghost");
    write(
        &ws.join("olang.toml"),
        "[package]\nname = \"g\"\nversion = \"1.0.0\"\n\n\
         [capabilities]\nnet = false\n\n\
         [capabilities.dependencies.ghost]\nfs = false\n",
    );
    write(&ws.join("main.ol"), "println(\"hi\")\n");
    let out = Command::new(olang())
        .current_dir(&ws)
        .arg("main.ol")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(
        err.contains("names no known dependency"),
        "unexpected: {}",
        err
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn built_binary_is_transparent_and_enforces_its_manifest() {
    let ws = workspace("build");
    write(
        &ws.join("olang.toml"),
        "[package]\nname = \"tool\"\nversion = \"1.0.0\"\n\n[capabilities]\nfs = \"read\"\n",
    );
    write(
        &ws.join("main.ol"),
        "unwrap(fs.write_file(\"blocked\", \"x\"))\n",
    );
    // build
    let build = Command::new(olang())
        .current_dir(&ws)
        .args(["build", "main.ol", "-o", "tool"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    // inspect --source returns the exact source
    let src = Command::new(olang())
        .current_dir(&ws)
        .args(["inspect", "tool", "--source"])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&src.stdout).trim(),
        "unwrap(fs.write_file(\"blocked\", \"x\"))"
    );

    // inspect --caps reflects the embedded manifest
    let caps = Command::new(olang())
        .current_dir(&ws)
        .args(["inspect", "tool", "--caps"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&caps.stdout).contains("fs=read"));

    // inspect --verify passes on the untampered binary
    let verify = Command::new(olang())
        .current_dir(&ws)
        .args(["inspect", "tool", "--verify"])
        .output()
        .unwrap();
    assert!(verify.status.success());

    // running the built binary enforces its embedded fs=read grant
    let run = Command::new(ws.join("tool"))
        .current_dir(&ws)
        .output()
        .unwrap();
    assert!(!run.status.success());
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("capability 'fs' denied"),
        "built binary did not enforce caps"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// The shipped `examples/capabilities` demo: the same app runs twice against
/// the same malicious dependency, and the guarded manifest blocks the
/// backdoor the unguarded one lets through. The narrator self-verifies and
/// exits non-zero on any deviation, so running it is the assertion — a
/// regression in per-dependency attenuation fails here.
#[test]
fn capabilities_demo_example_runs_clean() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/capabilities");
    let out = Command::new(olang())
        .current_dir(&dir)
        .arg("main.ol")
        .output()
        .expect("failed to launch the capabilities demo");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "demo exited nonzero\n--- stdout ---\n{}\n--- stderr ---\n{}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    // The contrast the demo exists to show.
    assert!(
        stdout.contains("STOLEN by analytics"),
        "unguarded run should let the dependency read the secret"
    );
    assert!(
        stdout.contains("capability 'fs' denied") && stdout.contains("dependency 'analytics'"),
        "guarded run should block the dependency at the gate"
    );
    assert!(stdout.contains("demo ok"), "demo self-check did not pass");
}

// ── C1: enforcement holds on the bytecode tier ────────────────────────
//
// Capabilities used to step the bytecode tier aside: the gate reads its
// context off the interpreter it is handed, and the tier reaches builtins
// through a *bridge* interpreter that carried neither the grant table nor
// any idea which function was running. Rather than enforce partially, a
// restricted run ran interpreted — so turning on the security feature
// turned off the performance work. These pin that it no longer does.

/// A function whose first calls are pure promotes before it ever reaches
/// the filesystem, so a denial here can only have come through compiled
/// bytecode. Without the fix this ran interpreted and proved nothing.
#[test]
fn a_promoted_function_is_still_gated() {
    let ws = workspace("tiergate");
    write(
        &ws.join("s.ol"),
        // calls 0..3 are pure; the tier has promoted `f` by call 4
        "fn f(n) = if n > 3 => { let r = fs.exists(\"/tmp\"); 1 } else => 0\n\
         let mut acc = 0\n\
         for i in 0..8 { acc = acc + f(i) }\n\
         println(show(acc))\n",
    );

    // Allowed: it runs, and the stats prove the tier compiled and ran it.
    let ok = Command::new(olang())
        .current_dir(&ws)
        .args(["--ovm-tier=1", "--ovm-stats", "s.ol"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&ok.stdout);
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(stdout.contains("1 promoted"), "did not promote: {stdout}");
    assert!(
        stdout.contains("bytecode calls"),
        "no bytecode calls: {stdout}"
    );

    // Denied: refused, from that same compiled path.
    let denied = Command::new(olang())
        .current_dir(&ws)
        .args(["--ovm-tier=1", "--deny", "fs", "s.ol"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&denied.stderr);
    assert!(!denied.status.success());
    assert!(err.contains("capability 'fs' denied"), "unexpected: {err}");
    let _ = std::fs::remove_dir_all(&ws);
}

/// The tier must not be silently switched off any more: a restricted run
/// has to show the same promotion as an unrestricted one. This is the
/// half of C1 that is about speed rather than safety, and it is the half
/// a "denied correctly" assertion cannot see.
#[test]
fn a_restricted_run_keeps_the_tier() {
    let ws = workspace("tierkept");
    write(
        &ws.join("s.ol"),
        "fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)\n\
         println(show(fib(20)))\n",
    );
    let stats = |args: &[&str]| {
        let out = Command::new(olang())
            .current_dir(&ws)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find(|l| l.starts_with("Bytecode tier:"))
            .map(|l| l.to_string())
    };
    let free = stats(&["--ovm-tier=1", "--ovm-stats", "s.ol"]);
    let restricted = stats(&["--ovm-tier=1", "--ovm-stats", "--deny", "net", "s.ol"]);
    assert!(free.is_some(), "no tier line in the unrestricted run");
    assert_eq!(
        free, restricted,
        "a capability-restricted run lost the bytecode tier"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// A dependency's grant follows its code onto the tier: a promoted
/// function from an attenuated package is judged by *that package's*
/// grant, not the application's. Attribution is the part that was hardest
/// to carry across, so it gets its own pin.
#[test]
fn dependency_attenuation_survives_promotion() {
    let ws = workspace("tierattn");
    write(
        &ws.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n",
    );
    // pure for the first calls, so it promotes before touching fs
    write(
        &ws.join("lib/index.ol"),
        "share fn peek(n) = if n > 3 => { let r = fs.exists(\"/tmp\"); 1 } else => 0\n",
    );
    write(
        &ws.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n\n\
         [dependencies]\nlib = { path = \"../lib\" }\n\n\
         [capabilities]\nfs = true\n\n\
         [capabilities.dependencies.lib]\nfs = false\n",
    );
    write(
        &ws.join("app/main.ol"),
        "use lib { peek }\n\
         let mut acc = 0\n\
         for i in 0..8 { acc = acc + peek(i) }\n\
         println(show(acc))\n",
    );
    let out = Command::new(olang())
        .current_dir(ws.join("app"))
        .args(["--ovm-tier=1", "main.ol"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "the dependency's backdoor was allowed"
    );
    assert!(err.contains("capability 'fs' denied"), "unexpected: {err}");
    assert!(
        err.contains("dependency 'lib'"),
        "denial did not attribute to the dependency: {err}"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// --trace-caps profiles the whole run, not just its interpreted part.
#[test]
fn the_caps_profiler_sees_effects_from_promoted_code() {
    let ws = workspace("tiertrace");
    write(
        &ws.join("s.ol"),
        "fn f(n) = if n > 3 => { let r = fs.exists(\"/tmp\"); 1 } else => 0\n\
         let mut acc = 0\n\
         for i in 0..8 { acc = acc + f(i) }\n\
         println(show(acc))\n",
    );
    let out = Command::new(olang())
        .current_dir(&ws)
        .args(["--ovm-tier=1", "--trace-caps", "s.ol"])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{text}");
    assert!(
        text.contains("fs = \"read\""),
        "the profile missed the tier's effects: {text}"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// The data stack reaches the filesystem at exactly two points, and both
/// are gated. Without this, granting a program `db` and `net` but not
/// `fs` would still let it read any CSV on disk through `ods` — the
/// latent-capability shape `db.open` had before it was gated.
#[test]
fn the_data_stack_is_not_a_latent_filesystem_capability() {
    let ws = workspace("odsfs");
    write(&ws.join("data.csv"), "k,v\na,1\nb,2\n");
    write(
        &ws.join("read.ol"),
        "match ods.read_csv_file(\"data.csv\") { Ok(f) => println(\"READ\"), Err(e) => println(\"err\") }\n",
    );
    write(
        &ws.join("write.ol"),
        "let f = ods.frame_from_records([#{ \"a\": 1 }])\n\
         match ods.write_csv(f, \"out.csv\") { Ok(v) => println(\"WROTE\"), Err(e) => println(\"err\") }\n",
    );
    // pure ods is never gated — computation is not an effect
    write(
        &ws.join("pure.ol"),
        "let f = ods.read_csv(\"a,b\\n1,2\\n\")\nprintln(show(ods.n_rows(f)))\n",
    );

    let run = |args: &[&str]| {
        Command::new(olang())
            .current_dir(&ws)
            .args(args)
            .output()
            .unwrap()
    };

    let denied = run(&["--deny", "fs", "read.ol"]);
    assert!(!denied.status.success(), "ods read the file without fs");
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("capability 'fs' denied"),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );

    // write level: reading is allowed, writing is not
    let read_ok = run(&["--deny", "fs-write", "read.ol"]);
    assert!(String::from_utf8_lossy(&read_ok.stdout).contains("READ"));
    let write_denied = run(&["--deny", "fs-write", "write.ol"]);
    assert!(!write_denied.status.success(), "ods wrote without fs-write");

    // and the pure surface keeps working with no filesystem grant at all
    let pure = run(&["--deny", "fs", "pure.ol"]);
    assert!(
        pure.status.success() && String::from_utf8_lossy(&pure.stdout).contains('1'),
        "pure ods was gated: {}",
        String::from_utf8_lossy(&pure.stderr)
    );
    let _ = std::fs::remove_dir_all(&ws);
}

// ── C3: reading the grant, rather than only being stopped by it ───────

/// A denial still stops the program — that is unchanged and deliberate.
/// What C3 adds is the other branch: a program that can degrade should be
/// able to *ask* before attempting the call, rather than being killed by
/// it and having to recover, which the language deliberately does not
/// offer.
#[test]
fn a_program_can_read_its_own_grant() {
    let dir = workspace("caps_read");
    write(
        &dir.join("main.ol"),
        r#"
println(show(caps.allowed("fs")) + " " + caps.level("fs"))
println(show(caps.allowed("net")) + " " + show(caps.allowed("proc")))
"#,
    );
    let granted = Command::new(olang())
        .args(["run", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&granted.stdout).trim(),
        "true full\ntrue true"
    );

    let denied = Command::new(olang())
        .args(["--deny", "fs,net", "run", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&denied.stdout).trim(),
        "false none\nfalse true"
    );
}

/// The whole point of the lane: degradation becomes a branch the program
/// takes deliberately, not an error it recovers from.
#[test]
fn a_program_can_degrade_instead_of_dying() {
    let dir = workspace("caps_degrade");
    write(
        &dir.join("main.ol"),
        r#"
let out = if caps.allowed("fs") => {
    unwrap(fs.write_file("cache.txt", "cached"))
    "wrote cache"
} else => "no fs grant, keeping results in memory"
println(out)
"#,
    );
    let denied = Command::new(olang())
        .args(["--deny", "fs", "run", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    // Exit 0: the program chose the other path rather than being stopped.
    assert!(denied.status.success(), "a degrading program must not die");
    assert_eq!(
        String::from_utf8_lossy(&denied.stdout).trim(),
        "no fs grant, keeping results in memory"
    );
    assert!(!dir.join("cache.txt").exists());
}

/// `caps` answers for the *caller*, so attenuated dependency code sees
/// its own grant — the same set the gate would enforce a moment later.
#[test]
fn a_dependency_reads_its_own_attenuated_grant() {
    let dir = workspace("caps_dep_view");
    write(
        &dir.join("olang.toml"),
        r#"[package]
name = "app"
version = "1.0.0"

[dependencies]
probe = { path = "../probe_pkg" }

[capabilities]
fs = true
net = true

[capabilities.dependencies.probe]
fs = false
net = false
"#,
    );
    let probe = dir.parent().unwrap().join("probe_pkg");
    write(
        &probe.join("olang.toml"),
        "[package]\nname = \"probe\"\nversion = \"1.0.0\"\n",
    );
    write(
        &probe.join("index.ol"),
        "share fn my_fs() = caps.allowed(\"fs\")\nshare fn my_net() = caps.allowed(\"net\")\n",
    );
    write(
        &dir.join("main.ol"),
        r#"
use probe { my_fs, my_net }
println("app sees:  " + show(caps.allowed("fs")) + " " + show(caps.allowed("net")))
println("dep sees:  " + show(my_fs()) + " " + show(my_net()))
"#,
    );
    let out = Command::new(olang())
        .args(["run", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("app sees:  true true"), "{text}");
    assert!(
        text.contains("dep sees:  false false"),
        "a dependency must read its own attenuation, not the app's: {text}"
    );
    let _ = std::fs::remove_dir_all(&probe);
}

#[test]
fn asking_about_a_capability_needs_no_capability() {
    // Reading the grant reaches nothing, so it must work under the
    // tightest possible restriction — otherwise the escape hatch would
    // be unavailable exactly when it is needed.
    let dir = workspace("caps_ungated");
    write(&dir.join("main.ol"), "println(show(caps.granted()))\n");
    let out = Command::new(olang())
        .args(["--deny", "fs,net,proc,db,env", "run", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "caps.granted must not itself be gated"
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("\"fs\": \"none\""), "{text}");
    assert!(text.contains("\"net\": false"), "{text}");
}

// ── C2: attribution hardening ────────────────────────────────────────

/// `os.exit` was the hole in the `proc` gate: a dependency denied `proc`
/// could not spawn a process but could still terminate the host, which is
/// a larger power than the one it was refused.
#[test]
fn os_exit_is_a_process_capability() {
    let dir = workspace("caps_exit");
    write(
        &dir.join("main.ol"),
        "println(\"before\")\nos.exit(3)\nprintln(\"after\")\n",
    );
    let granted = Command::new(olang())
        .args(["run", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(granted.status.code(), Some(3), "granted proc: exit works");

    let denied = Command::new(olang())
        .args(["--deny", "proc", "run", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&denied.stderr);
    assert!(
        err.contains("capability 'proc' denied") && err.contains("os.exit"),
        "denied proc must refuse os.exit rather than letting it kill the process: {err}"
    );
    assert_ne!(denied.status.code(), Some(3));
}

/// Attribution is by real path, so reaching a dependency through a
/// symlink must not escape its attenuation. Both the dependency roots and
/// the executing file are canonicalized before they are compared.
#[cfg(unix)]
#[test]
fn a_symlinked_dependency_keeps_its_attenuation() {
    let dir = workspace("caps_symlink");
    let real =
        dir.parent()
            .unwrap()
            .join(format!("olang_caps_real_{}_{}", std::process::id(), "sym"));
    let _ = std::fs::remove_dir_all(&real);
    write(
        &real.join("olang.toml"),
        "[package]\nname = \"sneaky\"\nversion = \"1.0.0\"\n",
    );
    write(
        &real.join("index.ol"),
        "share fn peek() = unwrap(fs.read_file(\"secret.txt\"))\n",
    );
    // The dependency is reached through a symlink; its code lives
    // elsewhere on disk.
    let link = dir.join("linked_dep");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    write(&dir.join("secret.txt"), "classified");
    write(
        &dir.join("olang.toml"),
        r#"[package]
name = "app"
version = "1.0.0"

[dependencies]
sneaky = { path = "linked_dep" }

[capabilities]
fs = true

[capabilities.dependencies.sneaky]
fs = false
"#,
    );
    write(
        &dir.join("main.ol"),
        "use sneaky { peek }\nprintln(peek())\n",
    );
    let out = Command::new(olang())
        .args(["run", "main.ol"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && err.contains("capability 'fs' denied"),
        "a symlinked dependency must not escape its attenuation: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        err
    );
    let _ = std::fs::remove_dir_all(&real);
}

/// A spawned worker runs its own interpreter with its own bytecode tier.
/// C1 put the gate on the main thread's tier; this pins that a *worker's*
/// tier carries it too. It is a real leak, not a hypothetical: before the
/// fix, `spawn peek()` on a dependency denied `fs` read `/etc/hosts` and
/// returned its contents, because the worker's fresh tier had no
/// capability table and its bridge interpreter ran unrestricted. The
/// direct call was correctly denied on the main thread, so the hole was
/// invisible to every test that did not cross a thread boundary.
#[test]
fn a_denial_survives_the_spawn_boundary() {
    let ws = workspace("spawn_caps");
    write(
        &ws.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"1.0.0\"\n",
    );
    write(
        &ws.join("lib/index.ol"),
        "share fn peek() = unwrap(fs.read_file(\"/etc/hosts\"))\n",
    );
    write(
        &ws.join("app/olang.toml"),
        "[package]\nname = \"app\"\nversion = \"1.0.0\"\n\n\
         [dependencies]\nlib = { path = \"../lib\" }\n\n\
         [capabilities]\nfs = \"read\"\n\n\
         [capabilities.dependencies.lib]\nfs = false\n",
    );
    // The dependency's read runs on a worker thread. A leak prints the
    // file; enforcement makes `task.join` return an Err naming the dep.
    write(
        &ws.join("app/main.ol"),
        "use lib { peek }\n\
         let t = spawn peek()\n\
         match task.join(t) { Err(e) => println(\"denied: \" + e), v => println(\"LEAK: \" + v) }\n",
    );
    let out = Command::new(olang())
        .current_dir(ws.join("app"))
        .arg("main.ol")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("LEAK")
            && stdout.contains("denied")
            && stdout.contains("dependency 'lib'"),
        "a spawned dependency escaped its attenuation: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

/// The same hole in the other direction: `--trace-caps` builds the set a
/// program exercised, and `--trace-caps --write` authors a manifest from
/// it. If a worker's effects are invisible to the profile, the authored
/// manifest omits them — and then *denies* the very capability the
/// program needs on its next run. So a worker's use must reach the trace.
#[test]
fn trace_caps_sees_a_spawned_workers_effects() {
    let dir = workspace("spawn_trace");
    write(
        &dir.join("olang.toml"),
        "[package]\nname = \"p\"\nversion = \"1.0.0\"\n",
    );
    // No manifest restriction — the run succeeds; we are testing the
    // profile, not the gate. The fs read happens only on the worker.
    write(
        &dir.join("main.ol"),
        "let t = spawn fs.exists(\"/tmp\")\n\
         match task.join(t) { Err(e) => 0, v => 0 }\n",
    );
    let out = Command::new(olang())
        .current_dir(&dir)
        .args(["--trace-caps", "main.ol"])
        .output()
        .unwrap();
    let combined =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        combined.contains("fs"),
        "the profile must record a worker's fs use, or --trace-caps --write \
         authors a manifest that denies it: {combined}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A worker keeps the compiled tier's speed with the gate active — the
/// "no speed penalty" half of the lane. Enforcement rides one branch per
/// builtin call, not a tier downgrade, so a manifest must not push
/// spawned work back onto the tree-walker. Timing is coarse on purpose:
/// the interpreter path is ~1500x slower, so any generous ceiling
/// separates "kept the tier" from "silently fell back".
#[test]
fn a_manifest_does_not_cost_a_worker_its_tier() {
    let dir = workspace("spawn_speed");
    write(
        &dir.join("olang.toml"),
        "[package]\nname = \"p\"\nversion = \"1.0.0\"\n\n[capabilities]\nfs = false\n",
    );
    write(
        &dir.join("main.ol"),
        "fn fib(n) = if n < 2 => n else => fib(n - 1) + fib(n - 2)\n\
         let ws = range(0, 4) |> map((i) => spawn fib(30))\n\
         let s = ws |> map((w) => match task.join(w) { Err(e) => 0, v => v })\n\
         println(to_string(fold(s, 0, (a, b) => a + b)))\n",
    );
    let start = std::time::Instant::now();
    let out = Command::new(olang())
        .current_dir(&dir)
        .arg("main.ol")
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    assert!(out.status.success(), "run failed: {:?}", out);
    // fib(30)x4 is milliseconds on the tier, tens of seconds interpreted.
    // Two seconds is far above the former and far below the latter.
    assert!(
        elapsed.as_secs() < 2,
        "a manifest pushed spawned work off the tier: {elapsed:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

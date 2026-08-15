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

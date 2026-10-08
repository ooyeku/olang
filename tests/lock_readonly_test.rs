//! `OLANG_LOCK_READONLY=1`: a run resolves its dependencies as usual but
//! writes no `olang.lock` — an editor's passive children (a language
//! server, a REPL, a check) leave the project they read as it was.

use std::fs;
use std::process::Command;

fn project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("olang_lock_ro_{}_{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("app")).unwrap();
    fs::create_dir_all(dir.join("lib")).unwrap();
    fs::write(dir.join("lib/olang.toml"), "[package]\nname = \"lib\"\nversion = \"0.1.0\"\n").unwrap();
    fs::write(dir.join("lib/index.ol"), "share fn twice(n) = n * 2\n").unwrap();
    fs::write(dir.join("app/olang.toml"), "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies.lib]\npath = \"../lib\"\n").unwrap();
    fs::write(dir.join("app/main.ol"), "use lib\nprintln(lib.twice(21))\n").unwrap();
    dir
}

#[test]
fn a_run_writes_the_lock_unless_it_is_read_only() {
    let dir = project("run");
    let app = dir.join("app");
    let ro = Command::new(env!("CARGO_BIN_EXE_olang")).arg("main.ol").current_dir(&app).env("OLANG_LOCK_READONLY", "1").output().unwrap();
    assert!(ro.status.success(), "{}", String::from_utf8_lossy(&ro.stderr));
    assert_eq!(String::from_utf8_lossy(&ro.stdout).trim(), "42");
    assert!(!app.join("olang.lock").exists(), "a read-only run wrote olang.lock");
    let rw = Command::new(env!("CARGO_BIN_EXE_olang")).arg("main.ol").current_dir(&app).env_remove("OLANG_LOCK_READONLY").output().unwrap();
    assert!(rw.status.success());
    assert!(app.join("olang.lock").exists(), "a run writes olang.lock");
    let _ = fs::remove_dir_all(&dir);
}

//! `otc doctor`, `otc clean`, and the toolchain plumbing, driven
//! through the real binary against a fabricated `OLANG_HOME` — the
//! user's actual `~/.olang` is never touched. The network-dependent
//! half of `otc update` (release download and checksum verification)
//! is exercised manually against real releases; everything here is
//! offline: layout auditing, safe repairs, cleaning, default-toolchain
//! bookkeeping, and the refusal paths.

use std::path::Path;
use std::process::Command;

fn otc() -> &'static str {
    env!("CARGO_BIN_EXE_otc")
}

fn run(home: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(otc())
        .args(args)
        .env("OLANG_HOME", home)
        // Shelf resolution honors OLANG_SHELF first; point it into the
        // fake home so the test never reads the user's shelf.
        .env("OLANG_SHELF", home.join("shelf.toml"))
        .output()
        .expect("spawn otc");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn fake_home(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir()
        .join("olang_home_mgmt_tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn doctor_reports_and_fixes_the_legacy_layout() {
    let home = fake_home("doctor");
    // The mess the old setup script left behind.
    for d in ["config", "logs", "projects", "ovm-cache", "warm"] {
        std::fs::create_dir_all(home.join(d)).unwrap();
    }
    std::fs::write(home.join("olang"), "#!/bin/sh\n").unwrap();
    std::fs::write(home.join("olang_playground.wasm"), b"wasm").unwrap();
    std::fs::create_dir_all(home.join("cache/git-db")).unwrap();
    std::fs::write(home.join("cache/git-db/blob"), b"x").unwrap();

    // Read-only run: findings reported, nothing removed.
    let (ok, text) = run(&home, &["doctor"]);
    assert!(ok, "doctor failed:\n{text}");
    assert!(text.contains("legacy directory"), "{text}");
    assert!(text.contains("stray file"), "{text}");
    assert!(text.contains("--fix"), "{text}");
    assert!(
        home.join("config").exists(),
        "read-only run must not delete"
    );

    // --fix removes the junk and leaves the cache alone.
    let (ok, text) = run(&home, &["doctor", "--fix"]);
    assert!(ok, "doctor --fix failed:\n{text}");
    for d in ["config", "logs", "projects", "ovm-cache", "warm"] {
        assert!(!home.join(d).exists(), "{d} survived --fix");
    }
    assert!(!home.join("olang").exists());
    assert!(!home.join("olang_playground.wasm").exists());
    assert!(
        home.join("cache/git-db/blob").exists(),
        "cache is clean's job"
    );

    // A second run is clean.
    let (ok, text) = run(&home, &["doctor"]);
    assert!(ok);
    assert!(text.contains("everything checks out"), "{text}");
}

#[test]
fn doctor_flags_dead_shelf_entries() {
    let home = fake_home("shelf");
    std::fs::write(
        home.join("shelf.toml"),
        "[libraries]\nghost = \"/nonexistent/lib/path\"\n",
    )
    .unwrap();
    let (ok, text) = run(&home, &["doctor"]);
    assert!(ok, "{text}");
    assert!(text.contains("ghost"), "{text}");
    assert!(text.contains("missing path"), "{text}");
}

#[test]
fn clean_reclaims_cache_and_state_but_not_toolchains() {
    let home = fake_home("clean");
    std::fs::create_dir_all(home.join("cache/git")).unwrap();
    std::fs::write(home.join("cache/git/clone"), vec![0u8; 2048]).unwrap();
    std::fs::create_dir_all(home.join("state/warm")).unwrap();
    std::fs::write(home.join("state/warm/x.toml"), b"warm").unwrap();
    std::fs::create_dir_all(home.join("toolchains/0.79.0/bin")).unwrap();
    std::fs::write(home.join("toolchains/0.79.0/bin/olang"), b"bin").unwrap();

    // Default: cache only.
    let (ok, text) = run(&home, &["clean"]);
    assert!(ok, "{text}");
    assert!(!home.join("cache").exists());
    assert!(home.join("state/warm/x.toml").exists());

    // --all takes state too; toolchains are never clean's business.
    let (ok, _) = run(&home, &["clean", "--all"]);
    assert!(ok);
    assert!(!home.join("state").exists());
    assert!(home.join("toolchains/0.79.0/bin/olang").exists());
}

#[test]
fn toolchain_bookkeeping_lists_defaults_and_refuses_correctly() {
    let home = fake_home("toolchains");
    // Two fabricated toolchains; make 0.79.0 the default by shimming
    // the way `otc update` does.
    for v in ["0.78.0", "0.79.0"] {
        let bin = home.join("toolchains").join(v).join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("olang"), b"#!/bin/sh\n").unwrap();
        std::fs::write(bin.join("otc"), b"#!/bin/sh\n").unwrap();
    }
    std::fs::create_dir_all(home.join("bin")).unwrap();
    for name in ["olang", "otc"] {
        std::os::unix::fs::symlink(
            home.join("toolchains/0.79.0/bin").join(name),
            home.join("bin").join(name),
        )
        .unwrap();
    }

    let (ok, text) = run(&home, &["toolchain", "list"]);
    assert!(ok, "{text}");
    assert!(text.contains("0.78.0"), "{text}");
    assert!(text.contains("0.79.0 (default)"), "{text}");

    // Removing the default refuses; removing the other succeeds.
    let (ok, text) = run(&home, &["toolchain", "remove", "0.79.0"]);
    assert!(!ok, "removing the default must refuse:\n{text}");
    let (ok, _) = run(&home, &["toolchain", "remove", "0.78.0"]);
    assert!(ok);
    assert!(!home.join("toolchains/0.78.0").exists());

    // Defaulting to something not installed refuses with the install hint.
    let (ok, text) = run(&home, &["toolchain", "default", "0.50.0"]);
    assert!(!ok);
    assert!(text.contains("not installed"), "{text}");
}

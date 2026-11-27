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
    // `doctor` audits every olang/otc on PATH and migrates a legacy
    // `$HOME/.olang_history`: the run sees only the otc under test on its
    // PATH and a HOME inside the fake one, so an older installed otc is no
    // finding here and `--fix` never moves the user's history.
    let path = home.join("test-path");
    std::fs::create_dir_all(&path).expect("mkdir");
    if !path.join("otc").exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(otc(), path.join("otc")).expect("link otc");
        #[cfg(not(unix))]
        std::fs::copy(otc(), path.join("otc")).expect("copy otc");
    }
    let out = Command::new(otc())
        .args(args)
        .env("PATH", &path)
        .env("HOME", home)
        .env("OLANG_HOME", home)
        .env("OLANG_GITHUB", "http://127.0.0.1:9")
        .env("OLANG_GITHUB_API", "http://127.0.0.1:9")
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
    assert!(text.contains("is gone"), "{text}");
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
        fake_olang(&bin.join("olang"), v);
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
    let row = text.lines().find(|l| l.contains("0.79.0")).expect(&text);
    assert!(row.contains("managed") && row.contains("default"), "{text}");

    // Removing the default refuses; removing the other succeeds.
    let (ok, text) = run(&home, &["toolchain", "remove", "0.79.0"]);
    assert!(!ok, "removing the default must refuse:\n{text}");
    let (ok, _) = run(&home, &["toolchain", "remove", "0.78.0"]);
    assert!(ok);
    assert!(!home.join("toolchains/0.78.0").exists());

    // Switching to something not installed and not reachable refuses,
    // naming the command that lists what there is.
    let (ok, text) = run(&home, &["use", "0.50.0"]);
    assert!(!ok);
    assert!(
        text.contains("next: otc toolchain list --available"),
        "{text}"
    );
}

/// An executable that answers `--version` as olang `version` does.
fn fake_olang(path: &Path, version: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, format!("#!/bin/sh\necho olang {version}\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// `otc toolchain list` (and a bare `otc toolchain`) shows every olang on
/// the machine with the version its binary gives, which one runs, the
/// shims' default, that cargo's shadows the shims, and the fix (OL-577).
/// Cargo's is the newer here, so the fix brings the default up; it does
/// not remove the newest olang on the machine (OL-638).
#[test]
fn toolchain_list_shows_every_olang_which_runs_and_why() {
    let home = fake_home("toolchain_list");
    // the machine the issue saw: cargo's rc.8 first on PATH, 0.88.0 shims
    // (a copy, not a link) over a 0.80.0 toolchain folder
    let cargo = home.join("user/.cargo/bin");
    fake_olang(&cargo.join("olang"), "1.0.0-rc.8");
    fake_olang(&home.join("bin/olang"), "0.88.0");
    fake_olang(&home.join("toolchains/0.80.0/bin/olang"), "0.80.0");
    let path = std::env::join_paths([cargo.clone(), home.join("bin")]).unwrap();
    let list = |args: &[&str]| {
        let out = Command::new(otc())
            .args(args)
            .env("PATH", &path)
            .env("HOME", home.join("user"))
            .env("OLANG_HOME", &home)
            .env("OLANG_GITHUB", "http://127.0.0.1:9")
            .env("OLANG_GITHUB_API", "http://127.0.0.1:9")
            .env("OLANG_SHELF", home.join("shelf.toml"))
            .current_dir(&home)
            .output()
            .expect("spawn otc");
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    for args in [&["toolchain", "list"][..], &["toolchain"][..]] {
        let text = list(args);
        let row = |v: &str| {
            text.lines()
                .find(|l| l.contains(v))
                .expect(&text)
                .to_string()
        };
        assert!(
            row("1.0.0-rc.8").contains("cargo") && row("1.0.0-rc.8").contains("active"),
            "{text}"
        );
        assert!(row("1.0.0-rc.8").trim_start().starts_with('→'), "{text}");
        assert!(
            row("0.88.0").contains("shims") && row("0.88.0").contains("default"),
            "{text}"
        );
        assert!(row("0.80.0").contains("managed"), "{text}");
        assert!(text.contains("is ahead of the shims on PATH"), "{text}");
        assert!(
            text.contains("not the default 0.88.0: otc update"),
            "{text}"
        );
        assert!(text.contains("keep it"), "{text}");
        assert!(!text.contains("uninstall"), "{text}");
        assert!(!text.contains("cargo install olang"), "{text}");
    }
    let d: serde_json::Value =
        serde_json::from_str(list(&["toolchain", "list", "--format", "json"]).trim()).unwrap();
    assert_eq!(d["kind"], "toolchains");
    assert_eq!(d["active"]["version"], "1.0.0-rc.8");
    assert_eq!(d["active"]["kind"], "cargo");
    let rows = d["toolchains"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{d}");
    let shims = rows.iter().find(|r| r["default"] == true).unwrap();
    assert_eq!(shims["version"], "0.88.0");
    assert!(
        d["shadowed"]
            .as_str()
            .unwrap()
            .contains("ahead of the shims"),
        "{d}"
    );
    assert!(d["fix"].as_str().unwrap().starts_with("otc update"), "{d}");

    // in a project, its pin and what answers it
    std::fs::write(
        home.join("olang.toml"),
        "[package]\nname = \"p\"\nversion = \"0.1.0\"\nolang = \"0.80\"\n",
    )
    .unwrap();
    let text = list(&["toolchain", "list"]);
    assert!(text.contains("pins olang 0.80: it runs 0.80.0"), "{text}");
    let _ = std::fs::remove_file(home.join("olang.toml"));
}

/// What `otc toolchain list` advises when another olang shadows the
/// default (OL-638). `uninstall` appears only when the shadowing olang is
/// not the newer one and the default would run once it is gone, and then
/// with how to put a Cargo build back; `cargo install olang` is named
/// only as what not to run (on crates.io that name is another project's).
#[test]
fn toolchain_list_never_advises_removing_the_only_or_the_newest_olang() {
    // (the outside olang's folder, its version, the shims', shims on PATH)
    let cases = [
        ("user/.cargo/bin", "1.0.0-rc.10", "0.88.0", false),
        ("user/.cargo/bin", "1.0.0-rc.10", "0.88.0", true),
        ("user/.cargo/bin", "0.88.0", "1.0.0", true),
        ("user/.cargo/bin", "0.88.0", "1.0.0", false),
        ("user/.cargo/bin", "1.0.0", "1.0.0", true),
        ("opt/homebrew/bin", "1.0.2", "1.0.0", true),
        ("opt/homebrew/bin", "0.88.0", "1.0.0", true),
    ];
    for (i, (outside, theirs, ours, shims_on_path)) in cases.into_iter().enumerate() {
        let home = fake_home(&format!("toolchain_advice_{i}"));
        let outside = home.join(outside);
        fake_olang(&outside.join("olang"), theirs);
        fake_olang(&home.join("bin/olang"), ours);
        let mut dirs = vec![outside.clone()];
        if shims_on_path {
            dirs.push(home.join("bin"));
        }
        let out = Command::new(otc())
            .args(["toolchain", "list"])
            .env("PATH", std::env::join_paths(dirs).unwrap())
            .env("HOME", home.join("user"))
            .env("OLANG_HOME", &home)
            .env("OLANG_SHELF", home.join("shelf.toml"))
            .current_dir(&home)
            .output()
            .expect("spawn otc");
        assert!(out.status.success(), "{out:?}");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let case = format!("{theirs} over {ours}, shims on PATH: {shims_on_path}\n{text}");
        assert!(text.contains("is ahead of the shims on PATH"), "{case}");
        let export = format!("export PATH=\"{}:$PATH\"", home.join("bin").display());
        let newer = semver_newer(theirs, ours);
        if newer {
            // bring the default up; the newer olang stays
            assert!(text.contains(": otc update"), "{case}");
            assert!(!text.contains("uninstall"), "{case}");
            assert!(!text.contains(&export), "{case}");
        } else {
            assert!(text.contains(&export), "{case}");
            assert!(!text.contains("otc update"), "{case}");
            // removing it is offered only when the default then runs
            assert_eq!(text.contains("uninstall olang"), shims_on_path, "{case}");
        }
        if text.contains("cargo uninstall olang") {
            assert!(
                text.contains(&format!("the default {ours} then runs")),
                "{case}"
            );
            assert!(text.contains("the install script"), "{case}");
            assert!(
                text.contains("`cargo install olang` is not olang"),
                "{case}"
            );
        } else {
            assert!(!text.contains("cargo install olang"), "{case}");
        }
    }
}

fn semver_newer(a: &str, b: &str) -> bool {
    let key = |v: &str| {
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        let n: Vec<u64> = core.split('.').map(|p| p.parse().unwrap()).collect();
        (n, pre.is_empty())
    };
    key(a) > key(b)
}

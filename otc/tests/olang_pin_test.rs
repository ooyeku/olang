//! A project pins its olang: `[package] olang = "0.88"` in olang.toml (a
//! minimum: 0.88.0 or newer) is
//! resolved by `otc install` to the newest installed toolchain that
//! satisfies it and written to olang.lock (`olang = "0.88.1"`); a lock's
//! `olang` survives an install when the manifest names none (it was
//! dropped); a pin nothing installed satisfies is said with the command
//! that installs one, never installed; `otc toolchain which` names the
//! binary a project's pin resolves to, and `otc toolchain link` registers
//! a local build as a toolchain. Every command runs with its own
//! `OLANG_HOME` (a temporary folder of fake toolchains).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("otc-pin-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn otc(dir: &Path, home: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_otc"))
        .args(args)
        .current_dir(dir)
        .env("OLANG_HOME", home)
        .env("OLANG_SHELF", home.join("shelf.toml"))
        .env_remove("OLANG_LOCK_READONLY")
        .output()
        .expect("spawn otc");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

// A fake toolchain: `<home>/toolchains/<v>/bin/olang`.
fn toolchain(home: &Path, v: &str) -> PathBuf {
    let bin = home.join("toolchains").join(v).join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let f = bin.join("olang");
    std::fs::write(&f, "#!/bin/sh\necho fake olang\n").unwrap();
    f
}

fn project(dir: &Path, pin: Option<&str>) {
    let pin = pin
        .map(|p| format!("olang = \"{}\"\n", p))
        .unwrap_or_default();
    std::fs::write(dir.join("olang.toml"), format!("[package]\nname = \"app\"\nversion = \"0.1.0\"\n{}\n[dependencies]\nlib = {{ path = \"lib\" }}\n", pin)).unwrap();
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/olang.toml"),
        "[package]\nname = \"lib\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib/index.ol"), "share fn one() = 1\n").unwrap();
}

fn lock(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("olang.lock")).unwrap_or_default()
}

#[test]
fn a_pin_resolves_to_the_newest_installed_toolchain_that_satisfies_it() {
    let home = scratch("resolve-home");
    let dir = scratch("resolve");
    toolchain(&home, "0.87.0");
    toolchain(&home, "0.88.0");
    toolchain(&home, "0.88.1");
    // a pin is a minimum: the newest installed olang at or past it
    let newest = toolchain(&home, "0.89.0");
    project(&dir, Some("0.88"));
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(ok, "{out}");
    assert!(lock(&dir).contains("olang = \"0.89.0\""), "{}", lock(&dir));
    let (ok, out) = otc(&dir, &home, &["toolchain", "which"]);
    assert!(ok, "{out}");
    assert_eq!(PathBuf::from(out.trim()), newest);
    // installed again: the lock is left as it was
    let before = lock(&dir);
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(ok, "{out}");
    assert_eq!(lock(&dir), before);
    // the pin moved past the lock's olang: re-resolved
    toolchain(&home, "1.0.0");
    project(&dir, Some("1"));
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(ok, "{out}");
    assert!(lock(&dir).contains("olang = \"1.0.0\""), "{}", lock(&dir));
}

#[test]
fn a_locks_olang_survives_an_install_when_the_manifest_names_none() {
    let home = scratch("keep-home");
    let dir = scratch("keep");
    project(&dir, None);
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(ok, "{out}");
    // the lock written by an older install, then an olang recorded in it
    let with = lock(&dir).replacen("version = 1\n", "version = 1\nolang = \"0.86.0\"\n", 1);
    assert!(with.contains("olang = \"0.86.0\""), "{with}");
    std::fs::write(dir.join("olang.lock"), &with).unwrap();
    // re-resolved (--update), and replayed: kept both ways
    let (ok, out) = otc(&dir, &home, &["install", "--update"]);
    assert!(ok, "{out}");
    assert!(lock(&dir).contains("olang = \"0.86.0\""), "{}", lock(&dir));
    // a dependency added: the lock is rewritten, its olang kept
    std::fs::create_dir_all(dir.join("more")).unwrap();
    std::fs::write(
        dir.join("more/olang.toml"),
        "[package]\nname = \"more\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("more/index.ol"), "share fn two() = 2\n").unwrap();
    let m =
        std::fs::read_to_string(dir.join("olang.toml")).unwrap() + "more = { path = \"more\" }\n";
    std::fs::write(dir.join("olang.toml"), m).unwrap();
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(ok, "{out}");
    let l = lock(&dir);
    assert!(
        l.contains("olang = \"0.86.0\"") && l.contains("[package.more]"),
        "{l}"
    );
    // `otc add` rewrites the manifest: a pin in it is kept
    project(&dir, Some("0.86"));
    toolchain(&home, "0.86.0");
    let (ok, out) = otc(&dir, &home, &["add", "more", "--path", "more"]);
    assert!(ok, "{out}");
    assert!(
        std::fs::read_to_string(dir.join("olang.toml"))
            .unwrap()
            .contains("olang = \"0.86\"")
    );
}

#[test]
fn a_pin_nothing_installed_satisfies_is_said_never_installed() {
    let home = scratch("missing-home");
    let dir = scratch("missing");
    toolchain(&home, "0.80.0");
    project(&dir, Some("9.9"));
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("pins olang 9.9") && out.contains("otc toolchain install 9.9"),
        "{out}"
    );
    assert!(!lock(&dir).contains("olang ="), "{}", lock(&dir));
    // nothing was fetched into the toolchains
    let names: Vec<String> = std::fs::read_dir(home.join("toolchains"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["0.80.0"]);
    let (ok, out) = otc(&dir, &home, &["toolchain", "which"]);
    assert!(!ok);
    assert!(out.contains("no installed olang satisfies it"), "{out}");
}

#[test]
fn a_local_build_is_linked_as_a_toolchain() {
    let home = scratch("link-home");
    let dir = scratch("link");
    let build = scratch("link-build");
    let bin = build.join("target/release");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("olang"), "#!/bin/sh\n").unwrap();
    let (ok, out) = otc(
        &dir,
        &home,
        &["toolchain", "link", "9.9.0", build.to_str().unwrap()],
    );
    assert!(ok, "{out}");
    project(&dir, Some("9.9"));
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(ok, "{out}");
    assert!(lock(&dir).contains("olang = \"9.9.0\""), "{}", lock(&dir));
    let (ok, out) = otc(&dir, &home, &["toolchain", "which"]);
    assert!(ok, "{out}");
    assert_eq!(
        std::fs::canonicalize(out.trim()).unwrap(),
        std::fs::canonicalize(bin.join("olang")).unwrap()
    );
}

#[test]
fn otc_install_writes_a_lock_never_lock_and_says_so() {
    // `[package] lock = "never"` keeps runs, tests and the editor from
    // writing olang.lock; an explicit `otc install` still writes it, with
    // a note, because the user asked.
    let home = scratch("never-home");
    let dir = scratch("never");
    project(&dir, None);
    let toml = std::fs::read_to_string(dir.join("olang.toml")).unwrap();
    std::fs::write(
        dir.join("olang.toml"),
        toml.replace(
            "version = \"0.1.0\"\n",
            "version = \"0.1.0\"\nlock = \"never\"\n",
        ),
    )
    .unwrap();
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(ok, "{out}");
    assert!(lock(&dir).contains("[package.lib]"), "{out}");
    assert!(out.contains("lock = \"never\""), "{out}");
    // a second install that changes nothing says nothing extra
    let (ok, out) = otc(&dir, &home, &["install"]);
    assert!(
        ok && out.contains("olang.lock unchanged") && !out.contains("lock = \"never\""),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

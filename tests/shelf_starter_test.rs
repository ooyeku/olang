//! The starter libraries: every shelf ships stocked, and what it ships
//! actually works.
//!
//! Seeding happens exactly once — when the shelf file first comes into
//! being — so a removal sticks. Each starter materializes as a real
//! library directory whose test blocks run green under `olang test`,
//! which is the standing quality gate on the curated set: a starter
//! without tests, or with failing ones, cannot ship.
//!
//! One test rather than several: the shelf location is a process-global
//! environment variable, and parallel tests would race on it.

use olang::pkg::shelf::Shelf;
use olang::pkg::starter::STARTERS;
use std::process::Command;

fn olang() -> &'static str {
    env!("CARGO_BIN_EXE_olang")
}

#[test]
fn starters_seed_once_run_green_and_restore() {
    let base = std::env::temp_dir().join(format!("olang_starter_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    // SAFETY: test-only env mutation; the single test in this binary.
    unsafe { std::env::set_var("OLANG_SHELF", base.join("shelf.toml")) };

    // First touch seeds every starter, materialized and registered.
    let shelf = Shelf::load_or_seed().expect("seed");
    assert_eq!(shelf.libraries.len(), STARTERS.len());
    for lib in STARTERS {
        let dir = shelf.resolve(lib.name).expect("registered");
        assert!(dir.join("index.ol").exists(), "{} materialized", lib.name);
        assert!(
            dir.join("olang.toml").exists(),
            "{} has a manifest",
            lib.name
        );
        assert!(shelf.owns(dir), "{} is shelf-owned", lib.name);

        // The standing gate: each starter's own test blocks pass.
        let out = Command::new(olang())
            .arg("test")
            .arg(dir)
            .output()
            .expect("spawn olang test");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success() && text.contains("0 failed"),
            "starter '{}' tests must pass:\n{}",
            lib.name,
            text
        );
    }

    // A removal sticks: seeding is tied to the file's creation, not to
    // every load.
    let mut shelf = Shelf::load_or_seed().unwrap();
    shelf.libraries.remove("markdown");
    shelf.save().unwrap();
    let again = Shelf::load_or_seed().unwrap();
    assert!(
        again.resolve("markdown").is_none(),
        "a removed starter must not respawn on the next load"
    );

    // Restore brings it back; unknown names refuse and say what exists.
    let mut shelf = Shelf::load_or_seed().unwrap();
    let dir = shelf.restore_starter("markdown").expect("restore");
    assert!(dir.join("index.ol").exists());
    let err = shelf.restore_starter("nonsense").unwrap_err().to_string();
    assert!(err.contains("textkit"), "error lists the starters: {err}");

    unsafe { std::env::remove_var("OLANG_SHELF") };
    let _ = std::fs::remove_dir_all(&base);
}

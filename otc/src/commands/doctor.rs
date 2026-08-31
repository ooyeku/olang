//! `otc doctor` — audit the installation against the `~/.olang`
//! contract (src/home.rs in the runtime) and repair what is safely
//! repairable.
//!
//! The default run is read-only: a table of checks with what each one
//! found. `--fix` applies the safe repairs — removing legacy scaffold
//! directories and stray files the old setup script created, migrating
//! the REPL history, clearing warm hints left by other versions — and
//! reports what it did. It never touches installations it does not own
//! (Homebrew, cargo); those are reported so the shadowing order is
//! visible, and removing them is left to their own package managers.

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run(fix: bool) -> Result<()> {
    let Some(home) = olang::home::root() else {
        anyhow::bail!("no home directory could be determined");
    };
    let mut problems = 0usize;
    let mut fixed = 0usize;

    println!("olang doctor — auditing {}", home.display());
    println!();

    // ── every olang/otc on PATH, and which wins ──────────────────────
    println!("binaries on PATH:");
    let mut seen_any = false;
    for name in ["olang", "otc"] {
        for (i, dir) in path_dirs().iter().enumerate() {
            let cand = dir.join(name);
            if !cand.is_file() {
                continue;
            }
            seen_any = true;
            let ver = version_of(&cand);
            let origin = origin_of(&cand, &home);
            let marker = if i == first_hit(name) { "→" } else { " " };
            println!(
                "  {} {:5} {:10} {:12} {}",
                marker,
                name,
                ver.as_deref().unwrap_or("?"),
                origin,
                cand.display()
            );
        }
    }
    if !seen_any {
        println!("  none found");
        problems += 1;
    }
    let olang_v = which_version("olang");
    let otc_v = which_version("otc");
    match (&olang_v, &otc_v) {
        (Some(a), Some(b)) if a != b => {
            println!("  ✗ version mismatch: olang {} vs otc {}", a, b);
            problems += 1;
        }
        _ => {}
    }
    println!();

    // ── the bin directory and PATH ───────────────────────────────────
    let bin = home.join("bin");
    if bin.is_dir() {
        let on_path = path_dirs().iter().any(|d| d == &bin);
        if on_path {
            println!("✓ {} is on PATH", bin.display());
        } else {
            println!("✗ {} exists but is not on PATH", bin.display());
            println!("    add: export PATH=\"{}:$PATH\"", bin.display());
            problems += 1;
        }
    }

    // ── legacy scaffolding from the old setup script ─────────────────
    let legacy_dirs = [
        ".otc",
        "projects",
        "templates",
        "logs",
        "config",
        "packages",
        "docs",
        "examples",
        "ovm-cache",
        "warm", // pre-contract location; state/warm is current
    ];
    for d in legacy_dirs {
        let p = home.join(d);
        if !p.exists() {
            continue;
        }
        let empty = std::fs::read_dir(&p)
            .map(|mut r| r.next().is_none())
            .unwrap_or(false);
        let label = if empty {
            "empty legacy directory"
        } else {
            "legacy directory"
        };
        if fix {
            std::fs::remove_dir_all(&p)?;
            println!("✓ removed {} ({})", p.display(), label);
            fixed += 1;
        } else {
            println!("✗ {} ({}) — --fix removes it", p.display(), label);
            problems += 1;
        }
    }
    for f in ["olang", "otc", "olang_playground.wasm"] {
        let p = home.join(f);
        if p.is_file() {
            if fix {
                std::fs::remove_file(&p)?;
                println!("✓ removed stray file {}", p.display());
                fixed += 1;
            } else {
                println!("✗ stray file {} — --fix removes it", p.display());
                problems += 1;
            }
        }
    }

    // ── history migration ────────────────────────────────────────────
    if let Some(hist) = olang::home::history()
        && !hist.exists()
        && let Some(home_dir) = std::env::var_os("HOME")
    {
        let legacy = PathBuf::from(home_dir).join(".olang_history");
        if legacy.exists() {
            if fix {
                if let Some(parent) = hist.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&legacy, &hist)?;
                println!("✓ migrated REPL history to {}", hist.display());
                fixed += 1;
            } else {
                println!(
                    "✗ REPL history at legacy {} — --fix migrates it",
                    legacy.display()
                );
                problems += 1;
            }
        }
    }

    // ── shelf integrity ──────────────────────────────────────────────
    match olang::pkg::shelf::Shelf::load() {
        Ok(shelf) => {
            let mut broken = 0;
            for (name, path) in &shelf.libraries {
                if !path.exists() {
                    println!(
                        "✗ shelf entry '{}' points at a missing path: {}",
                        name,
                        path.display()
                    );
                    broken += 1;
                }
            }
            if broken == 0 {
                println!(
                    "✓ shelf: {} entries, all paths resolve",
                    shelf.libraries.len()
                );
            } else {
                problems += broken;
                println!("    (otc lib remove <name> drops a dead entry)");
            }
        }
        Err(e) => {
            println!("✗ shelf manifest unreadable: {}", e);
            problems += 1;
        }
    }

    // ── sizes of the prunable parts ──────────────────────────────────
    for (label, path) in [
        ("cache", home.join("cache")),
        ("state", home.join("state")),
        ("toolchains", home.join("toolchains")),
    ] {
        if path.exists() {
            println!(
                "  {} holds {} (otc clean reclaims cache/state)",
                label,
                human(dir_size(&path))
            );
        }
    }

    println!();
    if fix {
        println!("doctor applied {} fix(es).", fixed);
    } else if problems == 0 {
        println!("everything checks out.");
    } else {
        println!(
            "{} finding(s); run `otc doctor --fix` to apply the safe repairs.",
            problems
        );
    }
    Ok(())
}

fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

fn first_hit(name: &str) -> usize {
    for (i, dir) in path_dirs().iter().enumerate() {
        if dir.join(name).is_file() {
            return i;
        }
    }
    usize::MAX
}

fn version_of(bin: &Path) -> Option<String> {
    let out = Command::new(bin).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().nth(1).map(|s| s.to_string())
}

fn which_version(name: &str) -> Option<String> {
    let dir = path_dirs().into_iter().find(|d| d.join(name).is_file())?;
    version_of(&dir.join(name))
}

fn origin_of(p: &Path, home: &Path) -> &'static str {
    let s = p.to_string_lossy();
    if p.starts_with(home.join("bin")) {
        "olang-home"
    } else if s.contains("/.cargo/") {
        "cargo"
    } else if s.contains("/Cellar/") || s.contains("/homebrew/") || s.contains("/linuxbrew/") {
        "homebrew"
    } else {
        "other"
    }
}

pub(crate) fn dir_size(p: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(p) {
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                total += dir_size(&path);
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    total
}

pub(crate) fn human(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
    } else if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1u64 << 20) as f64)
    } else if bytes >= 1 << 10 {
        format!("{:.1} KB", bytes as f64 / (1u64 << 10) as f64)
    } else {
        format!("{} B", bytes)
    }
}

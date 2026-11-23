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
use olang::pkg::shelf::{Shelf, ShelfCopy};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The shelf's copies and local folders, checked: each copy's folder is
/// there and still its checksum; each local folder is there; nothing is
/// under the shelf's home that no copy records. `--fix` removes the
/// orphans. Returns (problems, fixed).
fn check_shelf(shelf: &Shelf, fix: bool) -> (usize, usize) {
    let (mut problems, mut fixed) = (0, 0);
    let mut copies = 0;
    for (name, lib) in &shelf.libraries {
        if let Some(l) = &lib.local
            && !l.exists()
        {
            println!(
                "✗ shelf library '{name}' has a local folder that is gone: {} (`otc shelf add <its new place>` re-registers it, `otc shelf remove {name}@local` forgets it)",
                l.display()
            );
            problems += 1;
        }
        for c in &lib.copies {
            copies += 1;
            let dir = c.path();
            if !dir.is_dir() {
                println!(
                    "✗ shelf copy {name} {} is recorded but its folder is gone: {} (`otc shelf remove {name}@{}` forgets it)",
                    c.version,
                    dir.display(),
                    c.short_commit().unwrap_or_else(|| c.version.clone())
                );
                problems += 1;
                continue;
            }
            if let Some(want) = &c.checksum
                && olang::pkg::cache::checksum_dir(&dir).ok().as_ref() != Some(want)
            {
                println!(
                    "✗ shelf copy {name} {} was changed since it was fetched ({}): remove it and fetch it again",
                    c.version,
                    dir.display()
                );
                problems += 1;
            }
        }
    }
    // folders under the shelf's home no copy records
    let recorded: Vec<PathBuf> = shelf
        .libraries
        .values()
        .flat_map(|l| l.copies.iter().map(ShelfCopy::path))
        .collect();
    let home = Shelf::home();
    if let Ok(names) = std::fs::read_dir(&home) {
        for n in names.flatten() {
            let np = n.path();
            let leaves: Vec<PathBuf> = if n.file_name().to_string_lossy().starts_with('.') {
                vec![np.clone()]
            } else if np.is_dir() {
                std::fs::read_dir(&np)
                    .map(|r| r.flatten().map(|e| e.path()).collect())
                    .unwrap_or_default()
            } else {
                vec![np.clone()]
            };
            for leaf in leaves {
                if recorded.iter().any(|r| r == &leaf) {
                    continue;
                }
                if fix {
                    olang::pkg::shelf::remove_tree(&leaf);
                    let _ = std::fs::remove_file(&leaf);
                    println!(
                        "✓ removed {} (on the shelf's disk, recorded nowhere)",
                        leaf.display()
                    );
                    fixed += 1;
                } else {
                    println!(
                        "✗ {} is on the shelf's disk but recorded nowhere — --fix removes it",
                        leaf.display()
                    );
                    problems += 1;
                }
            }
            if np.is_dir() && std::fs::read_dir(&np).is_ok_and(|mut r| r.next().is_none()) {
                let _ = std::fs::remove_dir(&np);
            }
        }
    }
    if problems == 0 {
        println!(
            "✓ shelf: {} librar{}, {copies} cop{}, every folder there and as fetched",
            shelf.libraries.len(),
            if shelf.libraries.len() == 1 {
                "y"
            } else {
                "ies"
            },
            if copies == 1 { "y" } else { "ies" }
        );
    }
    (problems, fixed)
}

/// Move a shelf in the old layout (format 1: `revs`/`repos` tables,
/// snapshots under `shelf/pins/<name>-<sha>`, starters materialized at
/// `shelf/<name>/`) to this olang's (copies at `shelf/<name>/<version>/`).
/// Returns the libraries moved.
pub(crate) fn migrate_shelf() -> Result<usize> {
    use olang::pkg::shelf::folder_version;
    let file = Shelf::file();
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let table: toml::Table = toml::from_str(&text).unwrap_or_default();
    let get_map = |key: &str| -> Vec<(String, String)> {
        table
            .get(key)
            .and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let libraries = get_map("libraries");
    let revs: std::collections::BTreeMap<String, String> = get_map("revs").into_iter().collect();
    let repos: std::collections::BTreeMap<String, String> = get_map("repos").into_iter().collect();
    // a v2 shelf with folders under the shelf's home (old starters) keeps its copies
    let mut shelf = Shelf::load().unwrap_or_default();
    let home = Shelf::home();
    // everything old under the home moves aside first: copies are made
    // from there into the new places, which may be inside the old ones
    let aside = home.with_file_name(".shelf-migrating");
    olang::pkg::shelf::remove_tree(&aside);
    if home.is_dir() {
        std::fs::rename(&home, &aside)?;
    }
    let moved_path = |p: &Path| -> PathBuf {
        match p.strip_prefix(&home) {
            Ok(rest) => aside.join(rest),
            Err(_) => p.to_path_buf(),
        }
    };
    // the v2 copies already recorded move back as they were
    for lib in shelf.libraries.values_mut() {
        for c in &mut lib.copies {
            let from = aside.join(&c.dir);
            let to = c.path();
            if from.is_dir() {
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::rename(&from, &to)?;
            }
        }
    }
    let mut moved = 0;
    let mut old: Vec<(String, PathBuf)> = libraries
        .iter()
        .map(|(n, p)| (n.clone(), PathBuf::from(p)))
        .collect();
    // v2 local folders that live under the old home are starters too
    for (name, lib) in shelf.libraries.iter_mut() {
        if let Some(l) = lib.local.clone()
            && l.starts_with(&home)
        {
            lib.local = None;
            old.push((name.clone(), l));
        }
    }
    for (name, path) in old {
        let src = moved_path(&path);
        let rev = revs.get(&name).cloned();
        if !path.starts_with(&home) {
            // a local folder: referenced, as before
            let lib = shelf.libraries.entry(name.clone()).or_default();
            lib.local = Some(path.clone());
            moved += 1;
            continue;
        }
        if !src.is_dir() {
            continue;
        }
        let copy = ShelfCopy {
            version: folder_version(&src).unwrap_or_else(|| "0.0.0".to_string()),
            source: if rev.is_some() {
                "folder".to_string()
            } else if olang::pkg::starter::get(&name).is_some() {
                "starter".to_string()
            } else {
                "folder".to_string()
            },
            from: repos.get(&name).cloned(),
            reference: rev.clone(),
            commit: rev.clone(),
            checksum: None,
            dir: String::new(),
            fetched: None,
        };
        let made = shelf.add_copy(&name, &src, copy)?;
        if rev.is_some() {
            shelf.libraries.get_mut(&name).expect("added").default = made.short_commit();
        }
        moved += 1;
    }
    // every other old snapshot, which a project's lock may still pin
    if let Ok(pins) = std::fs::read_dir(aside.join("pins")) {
        for e in pins.flatten() {
            let leaf = e.file_name().to_string_lossy().into_owned();
            let Some((name, short)) = leaf.rsplit_once('-') else {
                continue;
            };
            if leaf.starts_with('.') || !short.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            if shelf
                .library(name)
                .and_then(|l| l.by_commit(short))
                .is_some()
            {
                continue;
            }
            let copy = ShelfCopy {
                version: folder_version(&e.path()).unwrap_or_else(|| "0.0.0".to_string()),
                source: "folder".to_string(),
                from: repos.get(name).cloned(),
                reference: Some(short.to_string()),
                commit: Some(short.to_string()),
                checksum: None,
                dir: String::new(),
                fetched: None,
            };
            shelf.add_copy(name, &e.path(), copy)?;
        }
    }
    shelf.save()?;
    olang::pkg::shelf::remove_tree(&aside);
    Ok(moved)
}

/// Toolchain folders that are old-style links (`toolchains/<version>/bin/
/// olang` a symlink into a checkout) become linked builds named by their
/// branch; the default is recorded in toolchains.toml. Returns (problems,
/// fixed).
fn migrate_toolchains(fix: bool) -> Result<(usize, usize)> {
    use olang::pkg::toolchain::{Linked, Toolchains};
    let (mut problems, mut fixed) = (0, 0);
    let Some(dir) = olang::home::toolchains() else {
        return Ok((0, 0));
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok((0, 0));
    };
    let mut tc = Toolchains::load();
    let shim = olang::home::bin().map(|b| b.join("olang"));
    let shim_target = shim.as_ref().and_then(|s| std::fs::read_link(s).ok());
    for e in entries.flatten() {
        let bin = e.path().join("bin").join("olang");
        let Ok(target) = std::fs::read_link(&bin) else {
            continue;
        };
        let Some(checkout) = target
            .ancestors()
            .find(|a| a.join("Cargo.toml").is_file())
            .map(Path::to_path_buf)
        else {
            continue;
        };
        let folder = e.file_name().to_string_lossy().into_owned();
        let branch = Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|b| !b.is_empty() && b != "HEAD")
            .unwrap_or_else(|| format!("local-{folder}"));
        if !fix {
            println!(
                "✗ toolchain folder {folder} is an old-style link to {} — --fix makes it the linked build '{branch}'",
                checkout.display()
            );
            problems += 1;
            continue;
        }
        let linked = Linked {
            checkout: checkout.clone(),
        };
        if shim_target.as_ref() == Some(&bin)
            && let Some(shim) = &shim
        {
            let staged = shim.with_file_name(".olang.new");
            let _ = std::fs::remove_file(&staged);
            #[cfg(unix)]
            std::os::unix::fs::symlink(linked.olang(), &staged)?;
            std::fs::rename(&staged, shim)?;
            tc.default = Some(branch.clone());
        }
        tc.linked.insert(branch.clone(), linked);
        std::fs::remove_dir_all(e.path())?;
        println!(
            "✓ toolchain folder {folder} is now the linked build '{branch}' ({})",
            checkout.display()
        );
        fixed += 1;
    }
    if fix && tc.default.is_none() {
        tc.default = super::update::default_toolchain().ok().flatten();
    }
    if fix {
        tc.save()?;
    }
    Ok((problems, fixed))
}

pub fn run(fix: bool) -> Result<()> {
    let Some(home) = olang::home::root() else {
        return Err(super::fail(
            "no home directory could be determined",
            "export OLANG_HOME=<a folder> (or set HOME)",
        ));
    };
    let mut problems = 0usize;
    let mut fixed = 0usize;

    println!("otc doctor — auditing {}", home.display());
    println!();

    // ── every olang/otc on PATH, and which wins ─────────────────────
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

    // ── the bin directory and PATH ──────────────────────────────────
    let bin = home.join("bin");
    if bin.is_dir() {
        let on_path = path_dirs().iter().any(|d| d == &bin);
        let managed = std::fs::read_dir(home.join("toolchains"))
            .map(|mut r| r.next().is_some())
            .unwrap_or(false);
        if on_path {
            println!("✓ {} is on PATH", bin.display());
        } else if managed {
            // Shims exist and a toolchain is installed behind them:
            // being off PATH defeats `otc update`.
            println!("✗ {} exists but is not on PATH", bin.display());
            println!("    add: export PATH=\"{}:$PATH\"", bin.display());
            problems += 1;
        } else {
            println!(
                "  note: {} is not on PATH (only matters once `otc update` manages toolchains)",
                bin.display()
            );
        }
    }

    // ── legacy scaffolding from the old setup script ────────────────
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

    // ── history migration ───────────────────────────────────────────
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

    // ── the old layout, moved by --fix ──────────────────────────────
    let (p, f) = migrate_toolchains(fix)?;
    problems += p;
    fixed += f;
    if olang::pkg::shelf::Shelf::old_layout() {
        if fix {
            let moved = migrate_shelf()?;
            println!(
                "✓ moved the shelf to this olang's layout ({moved} librar{})",
                if moved == 1 { "y" } else { "ies" }
            );
            fixed += 1;
        } else {
            println!(
                "✗ the shelf is in the old layout (pins under shelf/pins/, starters in shelf/<name>/) — --fix moves it to shelf/<name>/<version>/"
            );
            problems += 1;
        }
    }

    // ── shelf integrity ─────────────────────────────────────────────
    match olang::pkg::shelf::Shelf::load() {
        Ok(shelf) => {
            let (p, f) = check_shelf(&shelf, fix);
            problems += p;
            fixed += f;
        }
        Err(e) => {
            println!("✗ shelf manifest unreadable: {}", e);
            problems += 1;
        }
    }

    // ── the project here: a lock behind the shelf (information) ─────
    if let Ok(cwd) = std::env::current_dir()
        && let Some(root) = olang::pkg::manifest::Manifest::find_root(&cwd)
    {
        let behind = olang::pkg::lock_behind_shelf(&root);
        if !behind.is_empty() {
            println!("project {}:", root.display());
            for line in behind {
                println!("  note: {}", line);
            }
        }
    }

    // ── sizes of the prunable parts ─────────────────────────────────
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

pub(crate) fn path_dirs() -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    std::env::var_os("PATH")
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|d| seen.insert(d.clone()))
                .collect()
        })
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

//! `olang doctor [PATH]` — audit a project the way `otc doctor` audits
//! the home: the manifest parses, every dependency resolves, the lock
//! agrees with the manifest, the binary carries its browser runtime,
//! and nothing from the old build-and-copy workflow is left behind.
//! Each of these failed in a browser or a CI log before it failed here.

use std::path::{Path, PathBuf};

struct Finding {
    ok: bool,
    line: String,
}

fn ok(line: impl Into<String>) -> Finding {
    Finding {
        ok: true,
        line: line.into(),
    }
}

fn bad(line: impl Into<String>) -> Finding {
    Finding {
        ok: false,
        line: line.into(),
    }
}

/// Run the audit; the exit code is the number of findings, capped at 1.
pub fn run(path: &Path) -> i32 {
    let start = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let root = crate::pkg::manifest::Manifest::find_root(&start).unwrap_or(start.clone());
    println!("olang doctor — auditing {}", root.display());
    println!("  olang {}", crate::version::VERSION);
    let findings = audit(&root);
    let problems = findings.iter().filter(|f| !f.ok).count();
    for f in &findings {
        println!("  {} {}", if f.ok { "✓" } else { "✗" }, f.line);
    }
    if problems == 0 {
        println!("everything checks out.");
        0
    } else {
        println!(
            "{} finding{}.",
            problems,
            if problems == 1 { "" } else { "s" }
        );
        1
    }
}

fn audit(root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();

    // ── the manifest and its dependencies ────────────────────────────
    let manifest = match crate::pkg::manifest::Manifest::load(root) {
        Ok(m) => {
            out.push(ok(format!(
                "olang.toml: package {} {}",
                m.package.name, m.package.version
            )));
            Some(m)
        }
        Err(crate::pkg::manifest::ManifestError::NotFound(_)) => {
            out.push(ok(
                "no olang.toml: a bare script directory (nothing to resolve)",
            ));
            None
        }
        Err(e) => {
            out.push(bad(format!("olang.toml does not parse: {e}")));
            None
        }
    };
    let shelf = crate::pkg::shelf::Shelf::load().ok();
    if let Some(m) = &manifest {
        for (name, dep) in &m.dependencies {
            match dep {
                crate::pkg::manifest::Dependency::Path { path } => {
                    let dir = root.join(path);
                    if dir.join("olang.toml").exists() || dir.join("index.ol").exists() {
                        out.push(ok(format!("dependency {name}: {}", dir.display())));
                    } else {
                        out.push(bad(format!(
                            "dependency {name}: path {} has no olang.toml or index.ol",
                            dir.display()
                        )));
                    }
                }
                crate::pkg::manifest::Dependency::Shelf { shelf: entry } => {
                    match shelf.as_ref().and_then(|s| s.libraries.get(entry)) {
                        Some(dir) if dir.exists() => {
                            out.push(ok(format!("dependency {name}: shelf {entry} at {}", dir.display())))
                        }
                        Some(dir) => out.push(bad(format!(
                            "dependency {name}: shelf {entry} points at {}, which is gone (otc doctor)",
                            dir.display()
                        ))),
                        None => out.push(bad(format!(
                            "dependency {name}: shelf entry {entry} is not registered (otc shelf add)"
                        ))),
                    }
                }
                other => out.push(ok(format!("dependency {name}: {other:?}"))),
            }
        }
        // ── the lock agrees with the manifest ────────────────────────
        let lock_path = root.join("olang.lock");
        match crate::pkg::lock::Lockfile::load(root) {
            Ok(_) if !lock_path.exists() => {
                if m.dependencies.is_empty() {
                    out.push(ok("no olang.lock: no dependencies to pin"));
                } else {
                    out.push(ok("no olang.lock yet: written on the first run"));
                }
            }
            Ok(lock) => {
                let missing: Vec<&String> = m
                    .dependencies
                    .keys()
                    .filter(|d| !lock.package.contains_key(*d))
                    .collect();
                let extra: Vec<&String> = lock
                    .package
                    .keys()
                    .filter(|d| !m.dependencies.contains_key(*d))
                    .collect();
                if missing.is_empty() && extra.is_empty() {
                    out.push(ok(format!(
                        "olang.lock: {} package(s), agrees with the manifest",
                        lock.package.len()
                    )));
                } else {
                    let mut line = String::from("olang.lock disagrees with olang.toml:");
                    if !missing.is_empty() {
                        line.push_str(&format!(
                            " not locked: {}",
                            missing
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    if !extra.is_empty() {
                        line.push_str(&format!(
                            " locked but not declared: {}",
                            extra
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    line.push_str(" (rerun the program, or delete the lock)");
                    out.push(bad(line));
                }
            }
            Err(_) if root.join("olang.lock").exists() => out.push(bad(
                "olang.lock exists but does not parse (delete it; it is regenerated)",
            )),
            Err(_) => {
                if m.dependencies.is_empty() {
                    out.push(ok("no olang.lock: no dependencies to pin"));
                } else {
                    out.push(ok("no olang.lock yet: written on the first run"));
                }
            }
        }
    }

    // ── the pin a consumer keeps on olang itself ─────────────────────
    let pin = root.join(".olang-ref");
    if pin.exists() {
        let text = std::fs::read_to_string(&pin).unwrap_or_default();
        out.push(ok(format!(
            ".olang-ref pins {} — this binary is olang {} (the pin is checked where olang is built from it)",
            text.trim(),
            crate::version::VERSION
        )));
    }

    // ── the browser runtime ──────────────────────────────────────────
    match crate::runtime_wasm::embedded() {
        Some(bytes) => out.push(ok(format!(
            "browser runtime embedded: {} KB, hash {}",
            bytes.len() / 1024,
            crate::runtime_wasm::hash().unwrap_or("")
        ))),
        None => out.push(bad(
            "this olang binary carries no browser runtime: `cargo xtask install` (or `make install`) builds one in",
        )),
    }
    // Leftovers from the build-and-copy workflow the embedded runtime
    // replaced: a copy on disk is never served and only drifts.
    let mut leftovers = Vec::new();
    for dir in walk(root, 4) {
        for name in [
            "olang_playground.wasm",
            "olang_playground.wasm.br",
            "olang_playground.wasm.gz",
        ] {
            let p = dir.join(name);
            if p.exists() {
                leftovers.push(p);
            }
        }
    }
    if leftovers.is_empty() {
        out.push(ok(
            "no stale runtime copies (static/olang_playground.wasm and siblings)",
        ));
    } else {
        for p in leftovers {
            out.push(bad(format!(
                "{} is a copy the runtime no longer reads — delete it (serve uses runtime.wasm())",
                p.strip_prefix(root).unwrap_or(&p).display()
            )));
        }
    }
    out
}

/// Directories under `root`, `depth` levels down, skipping the usual
/// build and dependency trees.
fn walk(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    if depth == 0 {
        return out;
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if p.is_dir() && !name.starts_with('.') && name != "target" && name != "node_modules" {
                out.extend(walk(&p, depth - 1));
            }
        }
    }
    out
}

//! Project dependency management: `otc add`, `remove`, `list`, `install`.
//!
//! These edit `olang.toml`, resolve into `olang.lock`, and report the
//! graph. The design center is the local workflow: a dependency is a
//! directory on this machine — named on your shelf, or pointed at by
//! path — and every mutation ends by resolving, so the lockfile is never
//! left behind the manifest.

use olang::pkg::lock::{LockedSource, Lockfile};
use olang::pkg::manifest::{Dependency, Manifest, PackageMeta};
use olang::pkg::shelf::Shelf;
use olang::pkg::{InstallOptions, install};
use semver::Version;
use std::path::{Path, PathBuf};

/// The nearest enclosing project root, or None.
fn find_root() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| Manifest::find_root(&cwd))
}

/// The project root, creating an `olang.toml` in the current directory
/// when none exists anywhere above — `otc add` in a fresh directory
/// should start the project, not lecture about a missing init step.
fn root_or_init() -> anyhow::Result<PathBuf> {
    if let Some(root) = find_root() {
        return Ok(root);
    }
    let cwd = std::env::current_dir()?;
    let name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "my-project".to_string());
    let manifest = Manifest {
        package: PackageMeta {
            name: name.clone(),
            version: Version::new(0, 1, 0),
            description: None,
            authors: Vec::new(),
            license: None,
        },
        dependencies: Default::default(),
        capabilities: None,
    };
    manifest.save(&cwd).map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("No olang.toml found — created one for '{}'", name);
    Ok(cwd)
}

fn registry_from_env() -> Option<PathBuf> {
    std::env::var("OLANG_REGISTRY").ok().map(PathBuf::from)
}

/// `otc add <spec>` — one positional argument, two meanings:
///
///   otc add ../my-lib     a path (anything with a separator, or `.`-led)
///   otc add my-lib        a name, looked up on your shelf
///
/// `--path` forces the path reading for the rare directory name that
/// looks like a bare name.
pub fn add(spec: &str, path_flag: Option<&str>, force: bool) -> anyhow::Result<()> {
    let root = root_or_init()?;
    let mut manifest = Manifest::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;

    let looks_like_path =
        path_flag.is_some() || spec.contains(['/', '\\']) || spec.starts_with('.');

    let (name, dep) = if looks_like_path {
        let given = path_flag.unwrap_or(spec);
        // The path the user typed is relative to where they stand; the
        // manifest records it relative to the project root, so the
        // project stays coherent no matter which subdirectory the
        // command ran from.
        let cwd = std::env::current_dir()?;
        let absolute = if Path::new(given).is_absolute() {
            PathBuf::from(given)
        } else {
            cwd.join(given)
        };
        let dir = std::fs::canonicalize(&absolute)
            .map_err(|_| anyhow::anyhow!("'{}' is not a directory that exists", given))?;
        if !dir.is_dir() {
            anyhow::bail!("'{}' is not a directory", given);
        }
        let name = Manifest::load(&dir)
            .ok()
            .map(|m| m.package.name)
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .ok_or_else(|| anyhow::anyhow!("cannot infer a name for '{}'", given))?;
        let recorded = pathdiff(&dir, &root).unwrap_or_else(|| dir.display().to_string());
        (name, Dependency::Path { path: recorded })
    } else {
        // A bare name is a shelf lookup. Failing here with the fix named
        // beats recording a dependency that can never resolve.
        let shelf = Shelf::load_or_seed().map_err(|e| anyhow::anyhow!("{}", e))?;
        match shelf.resolve(spec) {
            Some(dir) => {
                println!("  {} → {} (from your shelf)", spec, dir.display());
                (
                    spec.to_string(),
                    Dependency::Shelf {
                        shelf: spec.to_string(),
                    },
                )
            }
            None => {
                let shelved: Vec<&String> = shelf.libraries.keys().collect();
                let hint = if shelved.is_empty() {
                    "your shelf is empty — register a library first: otc lib add <path>".to_string()
                } else {
                    format!(
                        "on your shelf: {}",
                        shelved
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                anyhow::bail!(
                    "'{}' is not on your shelf ({})\n\
                     Register it (otc lib add <path>) or add it by path (otc add <path>)",
                    spec,
                    hint
                );
            }
        }
    };

    if let Some(existing) = manifest.dependencies.get(&name)
        && !force
    {
        anyhow::bail!(
            "'{}' is already a dependency ({}); pass --force to change its source",
            name,
            describe_dependency(existing)
        );
    }
    manifest.dependencies.insert(name.clone(), dep);
    manifest.save(&root).map_err(|e| anyhow::anyhow!("{}", e))?;

    // Resolve immediately: `add` means "usable now", not "usable after a
    // second command you may not know about".
    let opts = InstallOptions {
        registry: registry_from_env(),
        ..Default::default()
    };
    install(&root, &opts).map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Added '{}' and updated olang.lock", name);
    Ok(())
}

fn describe_dependency(dep: &Dependency) -> String {
    match dep {
        Dependency::Path { path } => format!("path {}", path),
        Dependency::Git { git, .. } => format!("git {}", git),
        Dependency::Registry(req) => format!("registry {}", req),
        Dependency::RegistryExplicit { .. } => "registry".to_string(),
        Dependency::Shelf { shelf } => format!("shelf {}", shelf),
    }
}

pub fn remove(name: &str) -> anyhow::Result<()> {
    let root = find_root().ok_or_else(|| anyhow::anyhow!("no olang.toml found"))?;
    let mut manifest = Manifest::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    if manifest.dependencies.remove(name).is_none() {
        let have: Vec<&String> = manifest.dependencies.keys().collect();
        anyhow::bail!(
            "'{}' is not a dependency (this project has: {})",
            name,
            if have.is_empty() {
                "none".to_string()
            } else {
                have.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
    }
    manifest.save(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    // Re-resolve so the lock drops the entry too.
    let opts = InstallOptions {
        registry: registry_from_env(),
        ..Default::default()
    };
    install(&root, &opts).map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("Removed '{}' and updated olang.lock", name);
    Ok(())
}

/// `otc list` — the project, its dependencies, where each one lives, and
/// one level of what they pull in.
pub fn list() -> anyhow::Result<()> {
    let root = find_root().ok_or_else(|| anyhow::anyhow!("no olang.toml found"))?;
    let manifest = Manifest::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    let lock = Lockfile::load(&root).unwrap_or_default();

    println!("{} {}", manifest.package.name, manifest.package.version);
    if manifest.dependencies.is_empty() {
        println!("  (no dependencies — add one: otc add <name-or-path>)");
        return Ok(());
    }
    let direct: Vec<(&String, &Dependency)> = manifest.dependencies.iter().collect();
    for (i, (name, dep)) in direct.iter().enumerate() {
        let last = i + 1 == direct.len();
        let branch = if last { "└──" } else { "├──" };
        let locked = lock.package.get(*name);
        let where_from = match dep {
            Dependency::Shelf { shelf } => {
                let resolved = locked
                    .and_then(|p| match &p.source {
                        LockedSource::Path { path } => Some(path.clone()),
                        LockedSource::Shelf { shelf } => olang::pkg::shelf::Shelf::load()
                            .ok()
                            .and_then(|s| s.resolve(shelf).cloned())
                            .map(|dir| dir.display().to_string())
                            .or_else(|| Some("(not on this machine's shelf)".to_string())),
                        _ => None,
                    })
                    .unwrap_or_else(|| "(unresolved — run otc install)".to_string());
                format!("shelf:{} → {}", shelf, resolved)
            }
            Dependency::Path { path } => format!("path {}", path),
            Dependency::Git { git, .. } => {
                let rev = locked
                    .and_then(|p| match &p.source {
                        LockedSource::Git { rev, .. } => {
                            Some(format!(" @ {}", &rev[..rev.len().min(8)]))
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                format!("git {}{}", git, rev)
            }
            Dependency::Registry(req) => format!("registry {}", req),
            Dependency::RegistryExplicit { version, .. } => format!("registry {}", version),
        };
        let version = locked
            .and_then(|p| p.version.clone())
            .map(|v| format!(" {}", v))
            .unwrap_or_default();
        println!("{} {}{}  {}", branch, name, version, where_from);
        if let Some(p) = locked {
            let child_prefix = if last { "    " } else { "│   " };
            for (j, child) in p.dependencies.iter().enumerate() {
                let clast = j + 1 == p.dependencies.len();
                let cbranch = if clast { "└──" } else { "├──" };
                println!("{}{} {}", child_prefix, cbranch, child);
            }
        }
    }
    Ok(())
}

/// `otc install` — make the lockfile true. `--update` re-resolves
/// everything to the newest satisfying sources; `--frozen` fails instead
/// of changing the lock (CI's reproducibility gate).
pub fn do_install(frozen: bool, update: bool, verbose: bool) -> anyhow::Result<()> {
    let root = find_root().ok_or_else(|| anyhow::anyhow!("no olang.toml found"))?;
    let lock_before = std::fs::read_to_string(root.join("olang.lock")).ok();
    let opts = InstallOptions {
        frozen,
        registry: registry_from_env(),
        refresh: update,
    };
    // A lock written on another machine can name paths this one does not
    // have; say so before re-resolving, so the change to the lock is not
    // a surprise.
    if let Ok(lock) = Lockfile::load(&root) {
        for (name, locked) in &lock.package {
            if let LockedSource::Path { path } = &locked.source {
                let dir = if std::path::Path::new(path).is_absolute() {
                    std::path::PathBuf::from(path)
                } else {
                    root.join(path)
                };
                if !dir.exists() {
                    println!(
                        "the lock names a path that is not here ({} -> {}) — re-resolving",
                        name, path
                    );
                }
            }
        }
    }
    let map = install(&root, &opts).map_err(|e| anyhow::anyhow!("{}", e))?;
    println!(
        "Resolved {} dependenc{}",
        map.len(),
        if map.len() == 1 { "y" } else { "ies" }
    );
    if verbose {
        for (name, dir) in &map {
            println!("  {} -> {}", name, dir.display());
        }
    }
    let lock_after = std::fs::read_to_string(root.join("olang.lock")).ok();
    if lock_before == lock_after {
        println!("olang.lock unchanged");
    } else {
        println!("Wrote olang.lock");
    }
    Ok(())
}

/// `to` expressed relative to `from` when the walk is simple (shared
/// prefix, then up-and-over); None falls back to the absolute path.
fn pathdiff(to: &Path, from: &Path) -> Option<String> {
    let to: Vec<_> = to.components().collect();
    let from: Vec<_> = from.components().collect();
    let common = to
        .iter()
        .zip(from.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return None;
    }
    let mut parts: Vec<String> =
        std::iter::repeat_n("..".to_string(), from.len() - common).collect();
    for c in &to[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        return Some(".".to_string());
    }
    Some(parts.join("/"))
}

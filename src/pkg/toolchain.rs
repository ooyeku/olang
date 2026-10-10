//! The olang a project pins, and the installed toolchain that answers it.
//!
//! A project names the olang it is written for in its manifest:
//!
//! ```toml
//! [package]
//! name = "app"
//! version = "0.1.0"
//! olang = "0.88"        # a requirement, as a dependency's: ^0.88 (0.88.x)
//! ```
//!
//! The requirement reads as Cargo's do (`"0.88"` is `^0.88`: any 0.88.x;
//! `"=0.88.1"` that release only; `">=0.87, <0.90"` a range). `otc
//! install` writes the toolchain that resolved it to `olang.lock` (its
//! `olang` key, an exact version) and keeps a lock's `olang` when the
//! manifest names none. What is installed is `~/.olang/toolchains/
//! <version>/bin/olang` (`otc update`, `otc toolchain install`; a local
//! build registered with `otc toolchain link <version> <binary>`), and
//! the olang running now. Nothing here downloads anything: a pin no
//! installed olang satisfies is said, with the command that installs one.

use std::path::{Path, PathBuf};

/// The requirement `olang.toml` pins (`[package] olang`), if any.
pub fn manifest_pin(root: &Path) -> Option<String> {
    super::manifest::Manifest::load(root).ok()?.package.olang
}

/// The exact olang `olang.lock` records, if any.
pub fn lock_pin(root: &Path) -> Option<String> {
    super::lock::Lockfile::load(root).ok()?.olang
}

/// Whether version `v` satisfies requirement `req` (Cargo's reading; a
/// leading `v` on either is ignored). A requirement or version that does
/// not parse satisfies nothing.
pub fn satisfies(req: &str, v: &str) -> bool {
    let req = req.trim().trim_start_matches('v');
    let v = v.trim().trim_start_matches('v');
    match (semver::VersionReq::parse(req), semver::Version::parse(v)) {
        (Ok(r), Ok(v)) => r.matches(&v),
        _ => false,
    }
}

/// The toolchains installed under `~/.olang/toolchains` (or
/// `$OLANG_HOME/toolchains`): each version's directory holding
/// `bin/olang` (or `olang`), newest first.
pub fn installed() -> Vec<(String, PathBuf)> {
    let Some(dir) = crate::home::toolchains() else {
        return Vec::new();
    };
    installed_in(&dir)
}

/// `installed` for a toolchains directory.
pub fn installed_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let bin = binary_in(&e.path())?;
            Some((name.trim_start_matches('v').to_string(), bin))
        })
        .collect();
    out.sort_by(|a, b| newer(&b.0, &a.0));
    out
}

/// A toolchain directory's olang: `bin/olang`, else `olang`.
fn binary_in(dir: &Path) -> Option<PathBuf> {
    [dir.join("bin").join("olang"), dir.join("olang")].into_iter().find(|p| p.is_file())
}

// Versions in order (by semver where both parse, else by text).
fn newer(a: &str, b: &str) -> std::cmp::Ordering {
    match (semver::Version::parse(a), semver::Version::parse(b)) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

/// The newest installed toolchain satisfying `req`: `(version, binary)`.
pub fn resolve(req: &str) -> Option<(String, PathBuf)> {
    installed().into_iter().find(|(v, _)| satisfies(req, v))
}

/// What a project's pin comes to on this machine.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// The project pins nothing.
    Unpinned,
    /// An installed toolchain answers the pin: its version and binary.
    Installed { pin: String, version: String, bin: PathBuf },
    /// The olang running now answers it (no toolchain of its own).
    Running { pin: String },
    /// Nothing installed satisfies it.
    Missing { pin: String },
}

/// The olang a project's children should run: its lock's exact version
/// when installed, else the newest installed toolchain satisfying its
/// manifest's requirement, else the running olang when it satisfies it.
pub fn for_project(root: &Path) -> Resolved {
    let lock = lock_pin(root);
    let req = manifest_pin(root);
    let pin = match (&req, &lock) {
        (Some(r), _) => r.clone(),
        (None, Some(l)) => format!("={}", l.trim_start_matches('v')),
        (None, None) => return Resolved::Unpinned,
    };
    let shown = req.clone().unwrap_or_else(|| lock.clone().unwrap_or_default());
    // the lock's exact version first, while the manifest still allows it
    if let Some(l) = &lock
        && req.as_deref().is_none_or(|r| satisfies(r, l))
        && let Some((v, bin)) = installed().into_iter().find(|(v, _)| v == l.trim_start_matches('v'))
    {
        return Resolved::Installed { pin: shown, version: v, bin };
    }
    if let Some((version, bin)) = resolve(&pin) {
        return Resolved::Installed { pin: shown, version, bin };
    }
    if satisfies(&pin, crate::VERSION) {
        return Resolved::Running { pin: shown };
    }
    Resolved::Missing { pin: shown }
}

/// The `olang` key `otc install` writes: with a manifest pin, the lock's
/// version while it still satisfies it, else the newest installed olang
/// that does (a toolchain or the running one); without one, the lock's
/// as it was (a lock's `olang` is never dropped).
pub fn lock_entry(manifest_pin: Option<&str>, previous: Option<&str>) -> Option<String> {
    let Some(req) = manifest_pin else {
        return previous.map(|s| s.to_string());
    };
    if let Some(p) = previous
        && satisfies(req, p)
    {
        return Some(p.trim_start_matches('v').to_string());
    }
    let mut candidates: Vec<String> = installed().into_iter().map(|(v, _)| v).collect();
    candidates.push(crate::VERSION.to_string());
    candidates.sort_by(|a, b| newer(b, a));
    candidates.into_iter().find(|v| satisfies(req, v))
}

/// The command that installs a toolchain for `pin` (said, never run).
pub fn install_hint(pin: &str) -> String {
    let v = pin.trim().trim_start_matches(['=', '^', '~', 'v']);
    format!("otc toolchain install {}", v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_reads_as_cargo_reads_a_requirement() {
        assert!(satisfies("0.88", "0.88.3"));
        assert!(!satisfies("0.88", "0.89.0"));
        assert!(!satisfies("0.88", "0.87.9"));
        assert!(satisfies("=0.88.1", "0.88.1"));
        assert!(!satisfies("=0.88.1", "0.88.2"));
        assert!(satisfies(">=0.87, <0.90", "0.89.0"));
        assert!(satisfies("v0.88", "v0.88.0"));
        assert!(!satisfies("not a version", "0.88.0"));
    }

    #[test]
    fn toolchains_are_found_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        for v in ["0.87.0", "0.88.1", "0.88.0"] {
            let bin = dir.path().join(v).join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("olang"), "").unwrap();
        }
        // a directory without an olang is not a toolchain
        std::fs::create_dir_all(dir.path().join("0.99.0")).unwrap();
        let found: Vec<String> = installed_in(dir.path()).into_iter().map(|(v, _)| v).collect();
        assert_eq!(found, vec!["0.88.1", "0.88.0", "0.87.0"]);
    }
}

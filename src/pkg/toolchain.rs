//! The olang a project pins, and the installed toolchain that answers it.
//!
//! A project names the oldest olang it runs on in its manifest, as Cargo's
//! `rust-version` names the oldest Rust:
//!
//! ```toml
//! [package]
//! name = "app"
//! version = "0.1.0"
//! olang = "0.88"        # 0.88.0 or newer: 0.89 and 1.x satisfy it
//! ```
//!
//! A version with one, two or three parts (`"1"` is 1.0.0); nothing else
//! (`olang check` reports anything else). `otc install` writes the
//! toolchain that resolved it to `olang.lock` (its `olang` key, an exact
//! version) and keeps a lock's `olang` when the manifest names none; the
//! newest installed toolchain satisfying it is the one chosen; an olang
//! older than it warns as it runs the project. What is installed is
//! `~/.olang/toolchains/<version>/bin/olang` (`otc update`, `otc
//! toolchain install`; a local build registered with `otc toolchain link
//! <version> <binary>`), and the olang running now. Nothing here downloads
//! anything: a pin no installed olang satisfies is said, with the command
//! that installs one.

use std::path::{Path, PathBuf};

/// The requirement `olang.toml` pins (`[package] olang`), if any.
pub fn manifest_pin(root: &Path) -> Option<String> {
    super::manifest::Manifest::load(root).ok()?.package.olang
}

/// The exact olang `olang.lock` records, if any.
pub fn lock_pin(root: &Path) -> Option<String> {
    super::lock::Lockfile::load(root).ok()?.olang
}

/// The oldest version a `[package] olang` value names: `"0.88"` is
/// 0.88.0, `"1"` 1.0.0, `"0.88.2"` itself (a leading `v` ignored).
/// `None` for anything that is not one to three numbers.
pub fn minimum(req: &str) -> Option<semver::Version> {
    let req = req.trim().trim_start_matches('v');
    let parts: Vec<&str> = req.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut n = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        n[i] = p.parse().ok()?;
    }
    Some(semver::Version::new(n[0], n[1], n[2]))
}

/// Whether version `v` satisfies the pin `req`: `v` is `req` or newer. A
/// pin or version that does not parse satisfies nothing.
pub fn satisfies(req: &str, v: &str) -> bool {
    match (
        minimum(req),
        semver::Version::parse(v.trim().trim_start_matches('v')),
    ) {
        (Some(min), Ok(v)) => v >= min,
        _ => false,
    }
}

/// The warning an olang older than a project's pin says as it runs it.
pub fn too_old_warning(req: &str) -> Option<String> {
    if minimum(req).is_none() || satisfies(req, crate::VERSION) {
        return None;
    }
    Some(format!(
        "warning: this project needs olang {} or newer (olang.toml's [package] olang); this is {}",
        req.trim(),
        crate::VERSION
    ))
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
    [dir.join("bin").join("olang"), dir.join("olang")]
        .into_iter()
        .find(|p| p.is_file())
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
    Installed {
        pin: String,
        version: String,
        bin: PathBuf,
    },
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
        (None, Some(l)) => l.trim_start_matches('v').to_string(),
        (None, None) => return Resolved::Unpinned,
    };
    let shown = req
        .clone()
        .unwrap_or_else(|| lock.clone().unwrap_or_default());
    // the lock's exact version first, while the manifest still allows it
    if let Some(l) = &lock
        && req.as_deref().is_none_or(|r| satisfies(r, l))
        && let Some((v, bin)) = installed()
            .into_iter()
            .find(|(v, _)| v == l.trim_start_matches('v'))
    {
        return Resolved::Installed {
            pin: shown,
            version: v,
            bin,
        };
    }
    if let Some((version, bin)) = resolve(&pin) {
        return Resolved::Installed {
            pin: shown,
            version,
            bin,
        };
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
    let v = pin.trim().trim_start_matches(['=', '^', '~', '>', 'v']);
    format!("otc toolchain install {}", v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_is_a_minimum_version() {
        assert!(satisfies("0.88", "0.88.3"));
        assert!(satisfies("0.88", "0.89.0"));
        assert!(satisfies("0.88", "1.2.0"));
        assert!(!satisfies("0.88", "0.87.9"));
        assert!(satisfies("1", "1.0.0"));
        assert!(!satisfies("1.0", "0.99.0"));
        assert!(satisfies("0.88.1", "0.88.1"));
        assert!(!satisfies("0.88.1", "0.88.0"));
        assert!(satisfies("v0.88", "v0.88.0"));
        assert!(!satisfies("not a version", "0.88.0"));
        assert!(minimum("^0.88").is_none() && minimum("0.88.1.2").is_none());
        assert!(minimum("banana").is_none() && minimum("").is_none());
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
        let found: Vec<String> = installed_in(dir.path())
            .into_iter()
            .map(|(v, _)| v)
            .collect();
        assert_eq!(found, vec!["0.88.1", "0.88.0", "0.87.0"]);
    }
}

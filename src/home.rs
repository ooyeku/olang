//! The `~/.olang` contract.
//!
//! Every path the toolchain writes under the user's home directory is
//! defined here, and every writer goes through these functions — no
//! other module may invent a location. The layout:
//!
//! ```text
//! ~/.olang/
//!   bin/          shims pointing at the default toolchain
//!   toolchains/   <version>/bin/{olang, otc} — managed by `otc update`
//!   shelf/        registered library copies
//!   shelf.toml    the shelf manifest
//!   cache/        git/, git-db/ — dependency clones (prunable)
//!   state/        warm/, history — regenerable runtime state
//! ```
//!
//! `OLANG_HOME` overrides the root (tests point it at a temporary
//! directory); the finer-grained overrides some subsystems accept for
//! their own tests (`OLANG_SHELF`, `OLANG_CACHE`, `OLANG_WARM_DIR`)
//! take precedence over the layout and are documented on the
//! subsystems themselves. `otc doctor` audits a home directory against
//! this contract and `otc clean` reclaims the prunable parts.

use std::path::PathBuf;

/// The root: `$OLANG_HOME`, else `~/.olang`. `None` only when no home
/// directory can be determined at all.
pub fn root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("OLANG_HOME") {
        return Some(PathBuf::from(p));
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|h| PathBuf::from(h).join(".olang"))
}

/// Shim directory the installer puts on PATH.
pub fn bin() -> Option<PathBuf> {
    root().map(|r| r.join("bin"))
}

/// Side-by-side toolchain installs, one directory per version.
pub fn toolchains() -> Option<PathBuf> {
    root().map(|r| r.join("toolchains"))
}

/// The shelf manifest (`shelf.toml`).
pub fn shelf_manifest() -> Option<PathBuf> {
    root().map(|r| r.join("shelf.toml"))
}

/// Registered library copies.
pub fn shelf_dir() -> Option<PathBuf> {
    root().map(|r| r.join("shelf"))
}

/// Dependency cache root (`git/`, `git-db/` live under it).
pub fn cache() -> Option<PathBuf> {
    root().map(|r| r.join("cache"))
}

/// Regenerable runtime state.
pub fn state() -> Option<PathBuf> {
    root().map(|r| r.join("state"))
}

/// Tier warm hints.
pub fn warm() -> Option<PathBuf> {
    state().map(|s| s.join("warm"))
}

/// REPL history.
pub fn history() -> Option<PathBuf> {
    state().map(|s| s.join("history"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn olang_home_overrides_the_root() {
        // Serialized by the env-mutating test convention used elsewhere.
        unsafe { std::env::set_var("OLANG_HOME", "/tmp/olang-home-test") };
        assert_eq!(
            super::warm().unwrap(),
            std::path::PathBuf::from("/tmp/olang-home-test/state/warm")
        );
        unsafe { std::env::remove_var("OLANG_HOME") };
    }
}

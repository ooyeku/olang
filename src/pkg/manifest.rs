//! The project manifest, `olang.toml`.
//!
//! A manifest declares the package's identity and its dependencies. Because
//! olang packages are source (no build step, no ABI), a dependency is just a
//! pointer to a directory of `.ol` files: a local path, a git repository, or
//! a named registry version.

use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The parsed `olang.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub package: PackageMeta,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
    /// The `[capabilities]` block: what this package (and, attenuated,
    /// each dependency) is allowed to touch. Absent = full capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<crate::caps::CapsConfig>,
    /// The `[check]` block: how `olang check` treats this project's
    /// advisories. `promote = ["exhaustiveness", "shape"]` turns those
    /// warning classes into errors that fail the check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<CheckConfig>,
}

/// `[check]` in olang.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CheckConfig {
    /// Warning classes promoted to errors: `exhaustiveness` (a `match`
    /// over an enum or a literal union missing a case), `shape` (a
    /// literal key a declared record shape does not carry), `result` (a
    /// discarded `Result`), `all`.
    #[serde(default)]
    pub promote: Vec<String>,
    /// Functions pinned to native code: a name, a `*` pattern, or
    /// `path:pattern` (the path relative to the project's root). `olang
    /// check --tier` fails when the tier refuses one, and a run keeping
    /// tier statistics names one that falls back (src/tier_stats.rs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageMeta {
    pub name: String,
    pub version: Version,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    /// Files and directories (relative to the package) an application
    /// built with `olang build --app` carries for `asset.read`: fonts,
    /// images, data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
    /// The oldest olang this package runs on, as `rust-version` is Rust's:
    /// `"0.88"` is 0.88.0 or newer (1.x included), `"1"` 1.0.0 or newer.
    /// `otc install` records the toolchain that resolved it in olang.lock;
    /// an editor runs that toolchain for the project; an older olang
    /// running it warns (src/pkg/toolchain.rs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub olang: Option<String>,
    /// Who may write this project's `olang.lock`: `"auto"` (the default)
    /// lets any install refresh it; `"never"` keeps every incidental
    /// install (a run, `olang test`, the REPL, the language server, a
    /// check) from creating or rewriting it — the existing lock is still
    /// replayed, and without one resolution happens in memory. Only an
    /// explicit `otc install` / `add` / `remove` writes it. For test
    /// fixtures whose committed lock must not drift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<String>,
}

/// The values `[package] lock` accepts.
pub const LOCK_VALUES: &[&str] = &["auto", "never"];

/// A dependency's source. `toml` distinguishes these by which key is present,
/// so the enum is untagged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Dependency {
    /// `dep = "1.2.0"` — a registry dependency by version requirement.
    Registry(String),
    /// `dep = { path = "../thing" }` — a local directory.
    Path { path: String },
    /// `dep = { git = "url", tag/rev/branch = "..." }` — a git repository.
    Git {
        git: String,
        #[serde(default)]
        tag: Option<String>,
        #[serde(default)]
        rev: Option<String>,
        #[serde(default)]
        branch: Option<String>,
    },
    /// `dep = { version = "1.2", registry = "..." }` — explicit registry form.
    RegistryExplicit {
        version: String,
        #[serde(default)]
        registry: Option<String>,
    },
    /// `dep = { shelf = "my-lib" }` — a library registered on the user's
    /// local shelf (`otc lib add`). The manifest records only the name,
    /// which keeps it free of machine-specific paths; the shelf supplies
    /// the directory at install time and the lockfile pins what resolved.
    Shelf { shelf: String },
}

impl Dependency {
    /// The version requirement, for registry dependencies (used by the
    /// resolver). Path and git deps resolve without version selection.
    pub fn version_req(&self) -> Option<&str> {
        match self {
            Dependency::Registry(v) => Some(v),
            Dependency::RegistryExplicit { version, .. } => Some(version),
            _ => None,
        }
    }

    pub fn is_registry(&self) -> bool {
        matches!(
            self,
            Dependency::Registry(_) | Dependency::RegistryExplicit { .. }
        )
    }
}

/// The keys a capability table takes; `dependencies` only at the top.
const CAPABILITY_KEYS: &[&str] = &["fs", "net", "proc", "db", "env"];

fn check_capability_keys(caps: &toml::Value) -> Result<(), ManifestError> {
    let unknown = |table: &toml::value::Table, at: &str, deps: bool| {
        for key in table.keys() {
            if !(CAPABILITY_KEYS.contains(&key.as_str()) || (deps && key == "dependencies")) {
                return Err(ManifestError::Parse(format!(
                    "{} has no capability '{}' (known: {}); an unknown capability is refused, never ignored",
                    at,
                    key,
                    CAPABILITY_KEYS.join(", ")
                )));
            }
        }
        Ok(())
    };
    let toml::Value::Table(t) = caps else {
        return Err(ManifestError::Parse(
            "[capabilities] must be a table".into(),
        ));
    };
    unknown(t, "[capabilities]", true)?;
    if let Some(deps) = t.get("dependencies") {
        let toml::Value::Table(deps) = deps else {
            return Err(ManifestError::Parse(
                "[capabilities.dependencies] must be a table".into(),
            ));
        };
        for (name, spec) in deps {
            let toml::Value::Table(spec) = spec else {
                return Err(ManifestError::Parse(format!(
                    "[capabilities.dependencies.{}] must be a table",
                    name
                )));
            };
            unknown(
                spec,
                &format!("[capabilities.dependencies.{}]", name),
                false,
            )?;
        }
    }
    Ok(())
}

/// Errors reading or writing a manifest.
#[derive(Debug)]
pub enum ManifestError {
    Io(std::io::Error),
    Parse(String),
    NotFound(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(e) => write!(f, "{}", e),
            ManifestError::Parse(e) => write!(f, "invalid olang.toml: {}", e),
            ManifestError::NotFound(p) => write!(f, "no olang.toml found at {}", p),
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    /// Parse a manifest from TOML text. A key `[capabilities]` does not
    /// know is refused, never ignored: a misspelled or newer capability
    /// must not leave its grant wide open.
    pub fn from_toml(text: &str) -> Result<Self, ManifestError> {
        if let Ok(toml::Value::Table(t)) = toml::from_str::<toml::Value>(text)
            && let Some(caps) = t.get("capabilities")
        {
            check_capability_keys(caps)?;
        }
        toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))
    }

    /// Whether manifest text declares a `[capabilities]` block — read
    /// even when the text is not a valid manifest, so a broken one that
    /// meant to restrict is refused rather than run with no grant at all.
    pub fn declares_capabilities(text: &str) -> bool {
        match toml::from_str::<toml::Value>(text) {
            Ok(toml::Value::Table(t)) => t.contains_key("capabilities"),
            _ => text.contains("capabilities"),
        }
    }

    /// `load`, refusing a manifest that declares `[capabilities]` but
    /// cannot be read: `Ok(None)` when there is no usable manifest and
    /// nothing was restricted, `Err` when a restriction was asked for and
    /// cannot be honoured.
    pub fn load_for_capabilities(dir: &Path) -> Result<Option<Self>, ManifestError> {
        match Self::load(dir) {
            Ok(m) => Ok(Some(m)),
            Err(ManifestError::NotFound(_)) => Ok(None),
            Err(e) => {
                let text = crate::vfs::read_to_string(&dir.join("olang.toml")).unwrap_or_default();
                if Self::declares_capabilities(&text) {
                    Err(e)
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Load `olang.toml` from a directory.
    pub fn load(dir: &Path) -> Result<Self, ManifestError> {
        let path = dir.join("olang.toml");
        if !crate::vfs::exists(&path) {
            return Err(ManifestError::NotFound(path.display().to_string()));
        }
        let text = crate::vfs::read_to_string(&path).map_err(ManifestError::Io)?;
        Self::from_toml(&text)
    }

    /// Serialize back to TOML (used by `otc add`/`init`).
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Write `olang.toml` into a directory.
    pub fn save(&self, dir: &Path) -> Result<(), ManifestError> {
        std::fs::write(dir.join("olang.toml"), self.to_toml()).map_err(ManifestError::Io)
    }

    /// `[package] lock = "never"`: no incidental install writes the lock.
    /// An unknown value reads as `"auto"` (`olang check` warns about it).
    pub fn lock_never(&self) -> bool {
        self.package.lock.as_deref() == Some("never")
    }

    /// A `[package] olang` that is not a version, with the message
    /// `olang check` reports.
    pub fn olang_value_problem(&self) -> Option<String> {
        let v = self.package.olang.as_deref()?;
        if crate::pkg::toolchain::minimum(v).is_some() {
            return None;
        }
        Some(format!(
            "[package] olang = \"{}\" is not a version: it names the oldest olang the package runs on, as \"0.88\" or \"1.0.2\"",
            v
        ))
    }

    /// A `[package] lock` value olang does not know, with the message
    /// `olang check` warns with.
    pub fn lock_value_problem(&self) -> Option<String> {
        let v = self.package.lock.as_deref()?;
        if LOCK_VALUES.contains(&v) {
            return None;
        }
        Some(format!(
            "[package] lock = \"{}\" is not a lock policy (expected \"auto\" or \"never\"); treated as \"auto\"",
            v
        ))
    }

    /// Walk upward from `start` to find the nearest directory containing an
    /// `olang.toml`, returning that directory (the project root).
    pub fn find_root(start: &Path) -> Option<std::path::PathBuf> {
        let mut dir = if crate::vfs::is_file(start) {
            start.parent()?.to_path_buf()
        } else {
            start.to_path_buf()
        };
        loop {
            if crate::vfs::exists(&dir.join("olang.toml")) {
                return Some(dir);
            }
            if !dir.pop() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_dependency_kinds() {
        let text = r#"
[package]
name = "app"
version = "0.2.0"
description = "a test app"

[dependencies]
alpha = "1.4.0"
beta = { path = "../beta" }
epsilon = { shelf = "epsilon" }
gamma = { git = "https://example.com/gamma", tag = "v2.0.0" }
delta = { git = "https://example.com/delta", rev = "abc123" }
"#;
        let m = Manifest::from_toml(text).unwrap();
        assert_eq!(m.package.name, "app");
        assert_eq!(m.package.version, Version::new(0, 2, 0));
        assert_eq!(m.dependencies["alpha"].version_req(), Some("1.4.0"));
        assert!(matches!(m.dependencies["beta"], Dependency::Path { .. }));
        assert!(matches!(
            m.dependencies["epsilon"],
            Dependency::Shelf { .. }
        ));
        assert!(matches!(
            m.dependencies["gamma"],
            Dependency::Git { tag: Some(_), .. }
        ));
        assert!(matches!(
            m.dependencies["delta"],
            Dependency::Git { rev: Some(_), .. }
        ));
    }

    #[test]
    fn round_trips_through_toml() {
        let text = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
lib = "2.0.0"
"#;
        let m = Manifest::from_toml(text).unwrap();
        let reparsed = Manifest::from_toml(&m.to_toml()).unwrap();
        assert_eq!(m, reparsed);
    }
}

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
}

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
    /// Parse a manifest from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, ManifestError> {
        toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))
    }

    /// Load `olang.toml` from a directory.
    pub fn load(dir: &Path) -> Result<Self, ManifestError> {
        let path = dir.join("olang.toml");
        if !path.exists() {
            return Err(ManifestError::NotFound(path.display().to_string()));
        }
        let text = std::fs::read_to_string(&path).map_err(ManifestError::Io)?;
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

    /// Walk upward from `start` to find the nearest directory containing an
    /// `olang.toml`, returning that directory (the project root).
    pub fn find_root(start: &Path) -> Option<std::path::PathBuf> {
        let mut dir = if start.is_file() {
            start.parent()?.to_path_buf()
        } else {
            start.to_path_buf()
        };
        loop {
            if dir.join("olang.toml").exists() {
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

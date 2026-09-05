//! The lockfile, `olang.lock`.
//!
//! Records the exact resolved source of every dependency (transitively) so a
//! fresh `otc install` reproduces byte-identical code. Git deps pin a commit
//! SHA; path deps pin the resolved path; registry deps pin a version plus a
//! content checksum.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The whole lockfile: one entry per resolved package, keyed by name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Lockfile {
    /// Format version, so future changes can migrate.
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub package: BTreeMap<String, LockedPackage>,
}

fn default_version() -> u32 {
    1
}

/// One resolved dependency, pinned exactly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LockedPackage {
    /// Resolved version (registry/git deps carry the semver they resolved to).
    #[serde(default)]
    pub version: Option<String>,
    /// The pinned source: exactly one of path / git+rev / registry.
    pub source: LockedSource,
    /// sha256 of the package's source tree, for integrity on re-fetch.
    #[serde(default)]
    pub checksum: Option<String>,
    /// Direct dependency names, so the graph is reconstructable from the lock.
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum LockedSource {
    Path {
        path: String,
    },
    Git {
        git: String,
        rev: String,
        /// The ref the manifest asked for when this rev was pinned (a tag,
        /// branch, or explicit rev; None = default HEAD). Lets install tell
        /// "manifest unchanged, replay the pin" from "the requested ref
        /// changed, re-resolve". Absent in pre-existing lockfiles, which
        /// simply re-resolve once and are then stamped.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
    },
    Registry {
        registry: String,
    },
    /// A shelf library, by the name it is registered under on this
    /// machine (`~/.olang/shelf.toml`). Recorded by name so the lock is
    /// portable: another machine resolves the same name through its own
    /// shelf instead of a path that only existed here.
    Shelf {
        shelf: String,
    },
}

impl Lockfile {
    pub fn new() -> Self {
        Lockfile {
            version: default_version(),
            package: BTreeMap::new(),
        }
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Load `olang.lock` from a directory, or an empty lock if absent.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join("olang.lock");
        if !path.exists() {
            return Ok(Lockfile::new());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        Self::from_toml(&text)
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::write(dir.join("olang.lock"), self.to_toml()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut lock = Lockfile::new();
        lock.package.insert(
            "beta".to_string(),
            LockedPackage {
                version: Some("1.2.0".to_string()),
                source: LockedSource::Git {
                    git: "https://example.com/beta".to_string(),
                    rev: "deadbeef".to_string(),
                    reference: Some("v1.2.0".to_string()),
                },
                checksum: Some("abc".to_string()),
                dependencies: vec!["gamma".to_string()],
            },
        );
        let text = lock.to_toml();
        let reparsed = Lockfile::from_toml(&text).unwrap();
        assert_eq!(lock, reparsed);
    }

    #[test]
    fn a_shelf_entry_is_recorded_by_name_not_machine_path() {
        let mut lock = Lockfile::new();
        lock.package.insert(
            "web".to_string(),
            LockedPackage {
                version: None,
                source: LockedSource::Shelf {
                    shelf: "web".to_string(),
                },
                checksum: None,
                dependencies: vec![],
            },
        );
        let text = lock.to_toml();
        assert!(text.contains("kind = \"shelf\""), "{text}");
        assert!(!text.contains("/Users/"), "{text}");
        assert_eq!(Lockfile::from_toml(&text).unwrap(), lock);
    }
}

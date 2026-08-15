//! The package registry: a git repository of TOML index entries.
//!
//! An index is deliberately just files on disk (cloned from a git repo, or a
//! local directory for testing). For a package `foo`, the index holds
//! `foo.toml` listing every published version and where its source lives.
//! This needs no hosted service to start — the registry is a git repo, and
//! `otc publish` adds an entry to it.

use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// All published versions of one package.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexEntry {
    #[serde(default, rename = "release")]
    pub releases: Vec<Release>,
}

/// One published version: where its source is and its integrity checksum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: Version,
    /// Git URL the source is fetched from.
    pub git: String,
    /// Exact commit for this release.
    pub rev: String,
    /// sha256 of the source tree at that commit.
    #[serde(default)]
    pub checksum: Option<String>,
    /// This release's own registry dependencies (name -> version req),
    /// so the resolver can walk the graph without fetching first.
    #[serde(default)]
    pub dependencies: std::collections::BTreeMap<String, String>,
}

/// A registry backed by a local directory of `<name>.toml` index files.
pub struct Registry {
    root: PathBuf,
}

#[derive(Debug)]
pub enum RegistryError {
    Io(std::io::Error),
    Parse(String),
    NotFound(String),
    AlreadyPublished { name: String, version: String },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Io(e) => write!(f, "{}", e),
            RegistryError::Parse(e) => write!(f, "invalid index entry: {}", e),
            RegistryError::NotFound(n) => write!(f, "package '{}' not found in registry", n),
            RegistryError::AlreadyPublished { name, version } => write!(
                f,
                "{}@{} is already published — releases are append-only; bump the version, or pass --force to deliberately rewrite it",
                name, version
            ),
        }
    }
}

impl std::error::Error for RegistryError {}

impl Registry {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Registry { root: root.into() }
    }

    /// Read all published releases for a package name.
    pub fn entry(&self, name: &str) -> Result<IndexEntry, RegistryError> {
        let path = self.root.join(format!("{}.toml", name));
        if !path.exists() {
            return Err(RegistryError::NotFound(name.to_string()));
        }
        let text = std::fs::read_to_string(&path).map_err(RegistryError::Io)?;
        toml::from_str(&text).map_err(|e| RegistryError::Parse(e.to_string()))
    }

    /// Every version published for a package.
    pub fn versions(&self, name: &str) -> Result<Vec<Version>, RegistryError> {
        Ok(self
            .entry(name)?
            .releases
            .into_iter()
            .map(|r| r.version)
            .collect())
    }

    /// Look up one exact release.
    pub fn release(&self, name: &str, version: &Version) -> Result<Release, RegistryError> {
        self.entry(name)?
            .releases
            .into_iter()
            .find(|r| &r.version == version)
            .ok_or_else(|| RegistryError::NotFound(format!("{}@{}", name, version)))
    }

    /// Publish (append) a release into the index, creating the entry if new.
    /// Add a release to the index. Releases are append-only: publishing a
    /// version that already exists is an error unless `overwrite` is set —
    /// silently replacing a published release's rev/checksum is exactly the
    /// history rewrite the checksum field exists to catch.
    pub fn publish(
        &self,
        name: &str,
        release: Release,
        overwrite: bool,
    ) -> Result<(), RegistryError> {
        std::fs::create_dir_all(&self.root).map_err(RegistryError::Io)?;
        let mut entry = self.entry(name).unwrap_or_default();
        if let Some(existing) = entry.releases.iter().find(|r| r.version == release.version) {
            if !overwrite {
                return Err(RegistryError::AlreadyPublished {
                    name: name.to_string(),
                    version: release.version.to_string(),
                });
            }
            // An explicit overwrite of identical content is a no-op; of
            // different content, a deliberate history rewrite.
            let _ = existing;
        }
        entry.releases.retain(|r| r.version != release.version);
        entry.releases.push(release);
        entry.releases.sort_by(|a, b| a.version.cmp(&b.version));
        let text =
            toml::to_string_pretty(&entry).map_err(|e| RegistryError::Parse(e.to_string()))?;
        std::fs::write(self.root.join(format!("{}.toml", name)), text)
            .map_err(RegistryError::Io)?;
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

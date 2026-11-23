//! The package registry: a git repository of TOML index entries.
//!
//! An index is deliberately just files on disk (cloned from a git repo, or a
//! local directory for testing). For a package `foo`, the index holds
//! `foo.toml` listing every published version and where its source lives.
//! This needs no hosted service: the registry is a git repo, and
//! `otc publish` writes an entry into a checkout of it.
//!
//! `OLANG_REGISTRY` names the index: a folder, or a git address
//! (`https://…`, `git@…`, `file://…`) that is cloned into the cache and
//! refreshed when a name or a version is not in the copy held, on
//! `otc install --update`, `otc add` and `otc search`.

use semver::Version;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The index used when `OLANG_REGISTRY` names none. There is no public
/// index yet; its address goes here when there is one.
pub const DEFAULT_REGISTRY: Option<&str> = None;

/// The index to use: what the caller was given, else `OLANG_REGISTRY`,
/// else the default.
pub fn configured(given: Option<&Path>) -> Option<String> {
    given
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::var("OLANG_REGISTRY").ok())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| DEFAULT_REGISTRY.map(str::to_string))
}

/// Whether an index is named by a git address rather than a folder.
pub fn is_remote(spec: &str) -> bool {
    spec.contains("://") || spec.starts_with("git@")
}

/// A name an index can hold: it becomes a file's name, so letters,
/// digits, `-` and `_` only.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// All published versions of one package.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexEntry {
    /// What the library is, from its olang.toml at its latest publish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Its license, from its olang.toml at its latest publish.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
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
    /// The folder inside the repository that holds the package, when it
    /// is not the repository's root (`frameworks/web-sdk`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdir: Option<String>,
    /// sha256 of the package's source tree at that commit. An install
    /// refuses a release without one.
    #[serde(default)]
    pub checksum: Option<String>,
    /// Withdrawn: no new resolution chooses it; a lock that already
    /// names it still installs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub withdrawn: bool,
    /// Why it was withdrawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// This release's own registry dependencies (name -> version req),
    /// so the resolver can walk the graph without fetching first.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub dependencies: std::collections::BTreeMap<String, String>,
}

/// A registry backed by a local directory of `<name>.toml` index files.
pub struct Registry {
    root: PathBuf,
    /// The git address the folder is a clone of, when the index was
    /// named by one.
    remote: Option<String>,
    offline: bool,
    refreshed: Cell<bool>,
}

#[derive(Debug)]
pub enum RegistryError {
    Io(std::io::Error),
    Parse(String),
    Git(String),
    NotFound(String),
    AlreadyPublished { name: String, version: String },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Io(e) => write!(f, "{}", e),
            RegistryError::Parse(e) => write!(f, "invalid index entry: {}", e),
            RegistryError::Git(e) => write!(f, "{}", e),
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
    /// An index that is a folder on this machine.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Registry {
            root: root.into(),
            remote: None,
            offline: false,
            refreshed: Cell::new(false),
        }
    }

    /// The index `spec` names: a folder used in place, or a git address
    /// cloned into the cache (`cache/registry/<hash>`). `refresh` brings
    /// a clone already held up to date first; `offline` touches no
    /// network, and an index never cloned is then an error.
    pub fn open(spec: &str, offline: bool, refresh: bool) -> Result<Self, RegistryError> {
        if !is_remote(spec) {
            return Ok(Registry::at(spec));
        }
        if spec.starts_with('-') {
            return Err(RegistryError::Git(format!(
                "a registry's address cannot start with '-' ({spec:?})"
            )));
        }
        let root = super::cache::cache_root()
            .map_err(|e| RegistryError::Git(e.to_string()))?
            .join("registry")
            .join(super::cache::url_hash(spec));
        let reg = Registry {
            root,
            remote: Some(spec.to_string()),
            offline,
            refreshed: Cell::new(false),
        };
        if !reg.root.join(".git").exists() {
            if offline {
                return Err(RegistryError::Git(format!(
                    "the registry {spec} is not on this machine yet (it is fetched by `olang run` or `otc install`)"
                )));
            }
            if let Some(parent) = reg.root.parent() {
                std::fs::create_dir_all(parent).map_err(RegistryError::Io)?;
            }
            let _ = std::fs::remove_dir_all(&reg.root);
            git(
                None,
                &[
                    "clone",
                    "--quiet",
                    "--depth",
                    "1",
                    "--",
                    spec,
                    &reg.root.to_string_lossy(),
                ],
            )
            .map_err(|e| RegistryError::Git(format!("cannot fetch the registry {spec}: {e}")))?;
            reg.refreshed.set(true);
        } else if refresh {
            reg.refresh();
        }
        Ok(reg)
    }

    /// Bring a cloned index up to date, once per open. A failure (no
    /// network) leaves the copy held as it is. True when it was tried.
    fn refresh(&self) -> bool {
        if self.remote.is_none() || self.offline || self.refreshed.replace(true) {
            return false;
        }
        if git(
            Some(&self.root),
            &["fetch", "--quiet", "--depth", "1", "origin", "HEAD"],
        )
        .is_ok()
        {
            let _ = git(
                Some(&self.root),
                &["reset", "--quiet", "--hard", "FETCH_HEAD"],
            );
        }
        true
    }

    /// What names this index for a person: the address, else the folder.
    pub fn name(&self) -> String {
        self.remote
            .clone()
            .unwrap_or_else(|| self.root.display().to_string())
    }

    fn read_entry(&self, name: &str) -> Result<IndexEntry, RegistryError> {
        if !valid_name(name) {
            return Err(RegistryError::NotFound(name.to_string()));
        }
        let path = self.root.join(format!("{}.toml", name));
        if !path.exists() {
            return Err(RegistryError::NotFound(name.to_string()));
        }
        let text = std::fs::read_to_string(&path).map_err(RegistryError::Io)?;
        toml::from_str(&text).map_err(|e| RegistryError::Parse(e.to_string()))
    }

    /// Read all published releases for a package name. A name a cloned
    /// index does not hold is looked for again after one refresh.
    pub fn entry(&self, name: &str) -> Result<IndexEntry, RegistryError> {
        match self.read_entry(name) {
            Err(RegistryError::NotFound(_)) if self.refresh() => self.read_entry(name),
            other => other,
        }
    }

    /// Every version a resolution may choose: the published ones that
    /// were not withdrawn.
    pub fn versions(&self, name: &str) -> Result<Vec<Version>, RegistryError> {
        Ok(self
            .entry(name)?
            .releases
            .into_iter()
            .filter(|r| !r.withdrawn)
            .map(|r| r.version)
            .collect())
    }

    /// Look up one exact release, withdrawn or not.
    pub fn release(&self, name: &str, version: &Version) -> Result<Release, RegistryError> {
        let find = |entry: IndexEntry| entry.releases.into_iter().find(|r| &r.version == version);
        if let Some(r) = find(self.entry(name)?) {
            return Ok(r);
        }
        if self.refresh()
            && let Some(r) = find(self.read_entry(name)?)
        {
            return Ok(r);
        }
        Err(RegistryError::NotFound(format!("{}@{}", name, version)))
    }

    /// Every package the index holds, by name.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let stem = path.file_stem()?.to_str()?.to_string();
                (path.extension()? == "toml" && valid_name(&stem)).then_some(stem)
            })
            .collect();
        names.sort();
        names
    }

    fn write_entry(&self, name: &str, entry: &IndexEntry) -> Result<(), RegistryError> {
        let text =
            toml::to_string_pretty(entry).map_err(|e| RegistryError::Parse(e.to_string()))?;
        std::fs::write(self.root.join(format!("{}.toml", name)), text).map_err(RegistryError::Io)
    }

    /// Record what the package is beside its releases.
    pub fn set_about(
        &self,
        name: &str,
        description: Option<String>,
        license: Option<String>,
    ) -> Result<(), RegistryError> {
        let mut entry = self.read_entry(name)?;
        entry.description = description;
        entry.license = license;
        self.write_entry(name, &entry)
    }

    /// Withdraw a release (or put it back): it stays in the index, and a
    /// lock that names it still installs, but no new resolution chooses it.
    pub fn set_withdrawn(
        &self,
        name: &str,
        version: &Version,
        withdrawn: bool,
        reason: Option<String>,
    ) -> Result<(), RegistryError> {
        let mut entry = self.read_entry(name)?;
        let release = entry
            .releases
            .iter_mut()
            .find(|r| &r.version == version)
            .ok_or_else(|| RegistryError::NotFound(format!("{}@{}", name, version)))?;
        release.withdrawn = withdrawn;
        release.reason = if withdrawn { reason } else { None };
        self.write_entry(name, &entry)
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
        if !valid_name(name) {
            return Err(RegistryError::Parse(format!(
                "'{name}' cannot be a registry name (letters, digits, '-' and '_')"
            )));
        }
        let mut entry = self.read_entry(name).unwrap_or_default();
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
        self.write_entry(name, &entry)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let out = cmd.args(args).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

//! The library shelf: a per-user registry of local libraries, by name.
//!
//! The shelf is the local-first answer to "how do I use my own library
//! from another project without remembering where it lives". Register a
//! library once (`otc lib add ~/code/my-lib`) and any project can depend
//! on it by name (`otc add my-lib`); the manifest records the *name*
//! (`my-lib = { shelf = "my-lib" }`), which keeps `olang.toml` free of
//! machine-specific paths, and the lockfile records the resolved path
//! and checksum, so a moved or edited library is noticed rather than
//! silently drifted past.
//!
//! A fresh shelf ships stocked: the first touch seeds the curated
//! starter libraries (see `pkg::starter`) — materialized under the
//! shelf's home directory and registered like anything else, removable
//! and restorable like anything else.
//!
//! Stored as one small TOML file at `~/.olang/shelf.toml`
//! (`OLANG_SHELF` overrides the location, which is how tests isolate;
//! starter libraries materialize into a `shelf/` directory beside it).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Shelf {
    /// Library name -> absolute path of its directory.
    #[serde(default)]
    pub libraries: BTreeMap<String, PathBuf>,
    /// Library name -> the commit it was shelved at (`otc lib add <path>
    /// --rev <sha>`). Such a library is a snapshot the shelf owns, not a
    /// pointer at a checkout that moves; the rest follow their directory.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub revs: BTreeMap<String, String>,
}

#[derive(Debug)]
pub enum ShelfError {
    Io(String),
    Parse(String),
    /// The path being registered is not a directory, or carries no code.
    NotALibrary(String),
}

impl std::fmt::Display for ShelfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShelfError::Io(e) => write!(f, "{}", e),
            ShelfError::Parse(e) => write!(f, "invalid shelf.toml: {}", e),
            ShelfError::NotALibrary(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for ShelfError {}

impl Shelf {
    /// Where the shelf lives: `$OLANG_SHELF`, else `~/.olang/shelf.toml`.
    pub fn file() -> PathBuf {
        if let Ok(p) = std::env::var("OLANG_SHELF") {
            return PathBuf::from(p);
        }
        crate::home::shelf_manifest().unwrap_or_else(|| PathBuf::from(".olang-shelf.toml"))
    }

    /// Load the shelf; a missing file is an empty shelf, not an error.
    pub fn load() -> Result<Shelf, ShelfError> {
        let path = Self::file();
        if !path.exists() {
            return Ok(Shelf::default());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| ShelfError::Io(e.to_string()))?;
        toml::from_str(&text).map_err(|e| ShelfError::Parse(e.to_string()))
    }

    pub fn save(&self) -> Result<(), ShelfError> {
        let path = Self::file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ShelfError::Io(e.to_string()))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| ShelfError::Parse(e.to_string()))?;
        std::fs::write(&path, text).map_err(|e| ShelfError::Io(e.to_string()))
    }

    /// Register a library. The path must be a directory that looks like
    /// an olang library — an `index.ol` (the resolver's entry point) or
    /// an `olang.toml` naming it. Returns the name it registered under:
    /// the manifest's package name when one exists, the directory name
    /// otherwise, `explicit_name` overriding both.
    pub fn add(&mut self, path: &Path, explicit_name: Option<&str>) -> Result<String, ShelfError> {
        let dir = std::fs::canonicalize(path).map_err(|e| {
            ShelfError::NotALibrary(format!("'{}' is not reachable: {}", path.display(), e))
        })?;
        if !dir.is_dir() {
            return Err(ShelfError::NotALibrary(format!(
                "'{}' is not a directory",
                dir.display()
            )));
        }
        let manifest_name = crate::pkg::manifest::Manifest::load(&dir)
            .ok()
            .map(|m| m.package.name);
        if !dir.join("index.ol").exists() && manifest_name.is_none() {
            return Err(ShelfError::NotALibrary(format!(
                "'{}' has neither an index.ol nor an olang.toml — `use` would find nothing there \
                 (scaffold one with `otc new <name> --lib`)",
                dir.display()
            )));
        }
        let name = explicit_name
            .map(str::to_string)
            .or(manifest_name)
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "library".to_string());
        self.libraries.insert(name.clone(), dir);
        // Re-registering a name by directory unpins it.
        self.revs.remove(&name);
        Ok(name)
    }

    /// Register a library as it stands at one commit of its repository.
    /// The tree at `rev` is written under the shelf's own directory
    /// (`pins/<name>-<sha>`), so the shelved library no longer follows the
    /// checkout, and the full commit id is recorded — in the shelf, and
    /// from there in the `olang.lock` of every project that depends on
    /// it. Returns the registered name and the full commit id.
    pub fn add_at_rev(
        &mut self,
        path: &Path,
        explicit_name: Option<&str>,
        rev: &str,
    ) -> Result<(String, String), ShelfError> {
        let repo = std::fs::canonicalize(path).map_err(|e| {
            ShelfError::NotALibrary(format!("'{}' is not reachable: {}", path.display(), e))
        })?;
        let git = |args: &[&str]| -> Result<std::process::Output, ShelfError> {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .map_err(|e| ShelfError::Io(format!("git: {}", e)))
        };
        let resolved = git(&["rev-parse", "--verify", &format!("{rev}^{{commit}}")])?;
        if !resolved.status.success() {
            return Err(ShelfError::NotALibrary(format!(
                "'{}' has no commit '{}': {}",
                repo.display(),
                rev,
                String::from_utf8_lossy(&resolved.stderr).trim()
            )));
        }
        let sha = String::from_utf8_lossy(&resolved.stdout).trim().to_string();
        // The library may sit below the repository's root: archive the
        // subtree it occupies.
        let prefix = git(&["rev-parse", "--show-prefix"])?;
        let prefix = String::from_utf8_lossy(&prefix.stdout).trim().to_string();
        let tree = if prefix.is_empty() {
            sha.clone()
        } else {
            format!("{}:{}", sha, prefix.trim_end_matches('/'))
        };
        // Run from the repository's top level: from inside a subdirectory
        // `git archive` narrows to that subdirectory as well, so
        // `<sha>:<prefix>` named a path under the prefix twice and the
        // archive came back empty.
        let top = git(&["rev-parse", "--show-toplevel"])?;
        let top = String::from_utf8_lossy(&top.stdout).trim().to_string();
        let archive = std::process::Command::new("git")
            .arg("-C")
            .arg(if top.is_empty() {
                repo.as_path()
            } else {
                Path::new(&top)
            })
            .args(["archive", "--format=tar", &tree])
            .output()
            .map_err(|e| ShelfError::Io(format!("git: {}", e)))?;
        if !archive.status.success() {
            return Err(ShelfError::Io(format!(
                "git archive {}: {}",
                tree,
                String::from_utf8_lossy(&archive.stderr).trim()
            )));
        }
        let staging = Self::home()
            .join("pins")
            .join(format!(".staging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| ShelfError::Io(e.to_string()))?;
        let staging = std::fs::canonicalize(&staging).unwrap_or(staging);
        let mut untar = std::process::Command::new("tar")
            .arg("-x")
            .arg("-C")
            .arg(&staging)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ShelfError::Io(format!("tar: {}", e)))?;
        {
            use std::io::Write;
            let mut stdin = untar.stdin.take().expect("piped");
            stdin
                .write_all(&archive.stdout)
                .map_err(|e| ShelfError::Io(e.to_string()))?;
        }
        if !untar
            .wait()
            .map_err(|e| ShelfError::Io(e.to_string()))?
            .success()
        {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(ShelfError::Io(
                "tar could not unpack the archive".to_string(),
            ));
        }
        // Named like any other library: from the snapshot's own manifest.
        let name = match self.add(&staging, explicit_name) {
            Ok(name) => name,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        let name = if explicit_name.is_none() && name.starts_with(".staging-") {
            repo.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(name)
        } else {
            name
        };
        let pinned =
            Self::home()
                .join("pins")
                .join(format!("{}-{}", name, &sha[..sha.len().min(12)]));
        let _ = std::fs::remove_dir_all(&pinned);
        std::fs::rename(&staging, &pinned).map_err(|e| ShelfError::Io(e.to_string()))?;
        self.libraries.retain(|_, dir| dir != &staging);
        let pinned = std::fs::canonicalize(&pinned).unwrap_or(pinned);
        self.libraries.insert(name.clone(), pinned);
        self.revs.insert(name.clone(), sha.clone());
        Ok((name, sha))
    }

    /// The commit a shelf name is pinned at, if it is.
    pub fn rev_of(&self, name: &str) -> Option<&str> {
        self.revs.get(name).map(String::as_str)
    }

    /// The directory a shelf name resolves to, if registered.
    pub fn resolve(&self, name: &str) -> Option<&PathBuf> {
        self.libraries.get(name)
    }

    /// Where starter libraries materialize: a `shelf/` directory beside
    /// the shelf file.
    pub fn home() -> PathBuf {
        Self::file()
            .parent()
            .map(|p| p.join("shelf"))
            .unwrap_or_else(|| PathBuf::from("shelf"))
    }

    /// Load the shelf, seeding a brand-new one with the starter
    /// libraries. Seeding happens only when the shelf *file* does not
    /// exist yet — a shelf whose starters were deliberately removed
    /// stays exactly as its owner left it.
    pub fn load_or_seed() -> Result<Shelf, ShelfError> {
        if Self::file().exists() {
            return Self::load();
        }
        let mut shelf = Shelf::default();
        for lib in crate::pkg::starter::STARTERS {
            // A failed materialization skips that library rather than
            // failing the shelf: the shelf must work on a read-only
            // home, just with nothing pre-stocked.
            if shelf.restore_starter(lib.name).is_err() {
                shelf.libraries.remove(lib.name);
            }
        }
        shelf.save()?;
        Ok(shelf)
    }

    /// Materialize (or refresh) one starter library and register it.
    /// Unknown names error, naming what exists.
    pub fn restore_starter(&mut self, name: &str) -> Result<PathBuf, ShelfError> {
        let lib = crate::pkg::starter::get(name).ok_or_else(|| {
            ShelfError::NotALibrary(format!(
                "'{}' is not a starter library (they are: {})",
                name,
                crate::pkg::starter::STARTERS
                    .iter()
                    .map(|s| s.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        let dir = Self::home().join(lib.name);
        crate::pkg::starter::materialize(lib, &dir).map_err(|e| ShelfError::Io(e.to_string()))?;
        self.libraries.insert(lib.name.to_string(), dir.clone());
        Ok(dir)
    }

    /// Is this registered path a starter the shelf itself materialized
    /// (as opposed to a user directory the shelf merely points at)?
    /// Removal deletes such directories — the shelf owns them.
    pub fn owns(&self, path: &Path) -> bool {
        path.starts_with(Self::home())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test rather than several: the shelf location comes from a
    /// process-global environment variable, and parallel tests would
    /// race on it.
    #[test]
    fn shelf_round_trips_and_validates() {
        let dir = std::env::temp_dir().join(format!("olang_shelf_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: test-only env mutation, single-threaded within this test.
        unsafe { std::env::set_var("OLANG_SHELF", dir.join("shelf.toml")) };

        // Empty shelf loads from nothing.
        let mut shelf = Shelf::load().unwrap();
        assert!(shelf.libraries.is_empty());

        // A directory with no index.ol and no manifest is refused.
        let junk = dir.join("junk");
        std::fs::create_dir_all(&junk).unwrap();
        assert!(shelf.add(&junk, None).is_err());

        // A library shape registers under its manifest name.
        let lib = dir.join("greeting-lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(
            lib.join("olang.toml"),
            "[package]\nname = \"greetings\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(lib.join("index.ol"), "share fn hi() = \"hi\"\n").unwrap();
        let name = shelf.add(&lib, None).unwrap();
        assert_eq!(name, "greetings");
        shelf.save().unwrap();

        // Reload sees it; resolve returns the canonical path.
        let again = Shelf::load().unwrap();
        assert_eq!(
            again.resolve("greetings").unwrap(),
            &std::fs::canonicalize(&lib).unwrap()
        );

        unsafe { std::env::remove_var("OLANG_SHELF") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}

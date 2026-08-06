//! Package management for olang: manifest, lockfile, source fetching,
//! version resolution, and a registry.
//!
//! Because olang packages are source (no build step, no ABI), the whole job
//! is: read `olang.toml`, resolve each dependency to a directory of `.ol`
//! files (a local path, a fetched git checkout, or a resolved registry
//! release), pin the result in `olang.lock`, and hand the interpreter a
//! **dependency map** (`name -> directory`) it uses to resolve `use` paths.

pub mod cache;
pub mod lock;
pub mod manifest;
pub mod registry;
pub mod resolver;

use lock::{LockedPackage, LockedSource, Lockfile};
use manifest::{Dependency, Manifest};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The map the interpreter needs: dependency name -> the directory whose
/// `.ol` files that dependency exposes.
pub type DependencyMap = BTreeMap<String, PathBuf>;

#[derive(Debug)]
pub enum PkgError {
    Manifest(String),
    Lock(String),
    Fetch(String),
    Resolve(String),
    Registry(String),
}

impl std::fmt::Display for PkgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PkgError::Manifest(e) => write!(f, "manifest error: {}", e),
            PkgError::Lock(e) => write!(f, "lockfile error: {}", e),
            PkgError::Fetch(e) => write!(f, "fetch error: {}", e),
            PkgError::Resolve(e) => write!(f, "resolve error: {}", e),
            PkgError::Registry(e) => write!(f, "registry error: {}", e),
        }
    }
}

impl std::error::Error for PkgError {}

/// Options controlling an install.
#[derive(Default)]
pub struct InstallOptions {
    /// Fail instead of writing the lockfile if resolution would change it.
    pub frozen: bool,
    /// Registry index directory, if registry dependencies are used.
    pub registry: Option<PathBuf>,
}

/// Resolve and fetch all dependencies of the project rooted at `root`,
/// writing `olang.lock` and returning the dependency map for execution.
///
/// This is the heart of `otc install` and of loading a project to run it.
pub fn install(root: &Path, options: &InstallOptions) -> Result<DependencyMap, PkgError> {
    let manifest = Manifest::load(root).map_err(|e| PkgError::Manifest(e.to_string()))?;
    let mut lock = Lockfile::new();
    let mut dep_map = DependencyMap::new();

    // Registry resolution (MVS) over just the registry dependencies.
    let registry_roots: Vec<(String, String)> = manifest
        .dependencies
        .iter()
        .filter_map(|(name, dep)| dep.version_req().map(|r| (name.clone(), r.to_string())))
        .collect();

    let resolution = if registry_roots.is_empty() {
        Default::default()
    } else {
        let reg_root = options.registry.clone().ok_or_else(|| {
            PkgError::Registry("registry dependencies present but no registry configured".into())
        })?;
        let reg = registry::Registry::at(reg_root);
        resolver::resolve(&registry_roots, &reg).map_err(|e| PkgError::Resolve(e.to_string()))?
    };

    // Direct dependencies from the manifest.
    for (name, dep) in &manifest.dependencies {
        let (dir, locked) = resolve_dependency(root, name, dep, &resolution, options)?;
        dep_map.insert(name.clone(), dir);
        lock.package.insert(name.clone(), locked);
    }

    // Registry deps selected transitively but not named directly: fetch and
    // lock them too, so the whole graph is available.
    for (name, version) in &resolution {
        if lock.package.contains_key(name) {
            continue;
        }
        let reg_root = options.registry.clone().unwrap();
        let reg = registry::Registry::at(&reg_root);
        let release = reg
            .release(name, version)
            .map_err(|e| PkgError::Registry(e.to_string()))?;
        let git_ref = cache::GitRef::Rev(release.rev.clone());
        let fetched =
            cache::fetch_git(&release.git, &git_ref).map_err(|e| PkgError::Fetch(e.to_string()))?;
        dep_map.insert(name.clone(), fetched.dir.clone());
        lock.package.insert(
            name.clone(),
            LockedPackage {
                version: Some(version.to_string()),
                source: LockedSource::Registry {
                    registry: name.clone(),
                },
                checksum: release.checksum.clone(),
                dependencies: release.dependencies.keys().cloned().collect(),
            },
        );
    }

    // Reproducibility gate for CI.
    if options.frozen {
        let existing = Lockfile::load(root).map_err(PkgError::Lock)?;
        if existing != lock && !existing.package.is_empty() {
            return Err(PkgError::Lock(
                "lockfile is out of date (run without --frozen to update)".into(),
            ));
        }
    }

    lock.save(root).map_err(PkgError::Lock)?;
    Ok(dep_map)
}

/// Resolve one manifest dependency to (directory, lock entry).
fn resolve_dependency(
    root: &Path,
    name: &str,
    dep: &Dependency,
    resolution: &resolver::Resolution,
    options: &InstallOptions,
) -> Result<(PathBuf, LockedPackage), PkgError> {
    match dep {
        Dependency::Path { path } => {
            let dir = normalize(root, path);
            let checksum = cache::checksum_dir(&dir).ok();
            Ok((
                dir.clone(),
                LockedPackage {
                    version: None,
                    source: LockedSource::Path { path: path.clone() },
                    checksum,
                    dependencies: sub_dependency_names(&dir),
                },
            ))
        }
        Dependency::Git {
            git,
            tag,
            rev,
            branch,
        } => {
            let git_ref = if let Some(r) = rev {
                cache::GitRef::Rev(r.clone())
            } else if let Some(t) = tag {
                cache::GitRef::Tag(t.clone())
            } else if let Some(b) = branch {
                cache::GitRef::Branch(b.clone())
            } else {
                cache::GitRef::Default
            };
            let fetched =
                cache::fetch_git(git, &git_ref).map_err(|e| PkgError::Fetch(e.to_string()))?;
            let checksum = cache::checksum_dir(&fetched.dir).ok();
            let deps = sub_dependency_names(&fetched.dir);
            Ok((
                fetched.dir.clone(),
                LockedPackage {
                    version: None,
                    source: LockedSource::Git {
                        git: git.clone(),
                        rev: fetched.rev.unwrap_or_default(),
                    },
                    checksum,
                    dependencies: deps,
                },
            ))
        }
        Dependency::Registry(_) | Dependency::RegistryExplicit { .. } => {
            let version = resolution.get(name).ok_or_else(|| {
                PkgError::Resolve(format!("registry dependency '{}' was not resolved", name))
            })?;
            let reg_root = options
                .registry
                .clone()
                .ok_or_else(|| PkgError::Registry("no registry configured".into()))?;
            let reg = registry::Registry::at(&reg_root);
            let release = reg
                .release(name, version)
                .map_err(|e| PkgError::Registry(e.to_string()))?;
            let fetched = cache::fetch_git(&release.git, &cache::GitRef::Rev(release.rev.clone()))
                .map_err(|e| PkgError::Fetch(e.to_string()))?;
            Ok((
                fetched.dir.clone(),
                LockedPackage {
                    version: Some(version.to_string()),
                    source: LockedSource::Registry {
                        registry: name.to_string(),
                    },
                    checksum: release.checksum.clone(),
                    dependencies: release.dependencies.keys().cloned().collect(),
                },
            ))
        }
    }
}

/// The direct dependency names of a sub-package (for the lock graph), read
/// from its own `olang.toml` if it has one.
fn sub_dependency_names(dir: &Path) -> Vec<String> {
    Manifest::load(dir)
        .map(|m| m.dependencies.keys().cloned().collect())
        .unwrap_or_default()
}

/// Resolve a possibly-relative path against the project root.
fn normalize(root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

/// Load the dependency map for a project without re-resolving, from an
/// existing lockfile plus the manifest — the fast path for running a project.
/// Falls back to a full install if anything is missing.
pub fn dependency_map(root: &Path, options: &InstallOptions) -> Result<DependencyMap, PkgError> {
    // For correctness and simplicity, resolving is idempotent and cheap for
    // path deps and cached git deps, so just install.
    install(root, options)
}

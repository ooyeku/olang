//! Package management for olang: manifest, lockfile, source fetching,
//! version resolution, and a registry.
//!
//! Because olang packages are source (no build step, no ABI), the whole job
//! is: read `olang.toml`, resolve each dependency to a directory of `.ol`
//! files (a local path, a fetched git checkout, or a resolved registry
//! release), pin the result in `olang.lock`, and hand the interpreter a
//! **dependency map** (`name -> directory`) it uses to resolve `use` paths.

pub mod cache;
pub mod github;
pub mod lock;
pub mod manifest;
pub mod projects;
pub mod registry;
pub mod resolver;
pub mod shelf;
pub mod starter;
pub mod toolchain;

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
    /// Ignore the existing lockfile and re-resolve everything to the newest
    /// satisfying sources (`otc pkg update`). Without this, an existing lock
    /// that still covers the manifest is replayed exactly.
    pub refresh: bool,
    /// Re-pin a shelf library whose shelf registration moved since the
    /// lock was written (`otc install`): a lock entry whose recorded rev
    /// differs from the rev this machine's shelf now holds the name at —
    /// re-shelved at another commit, pinned where it followed its
    /// directory, or unpinned where it was pinned — is re-resolved rather
    /// than replayed. Off for running a program, where such a lock is
    /// refused instead of quietly drifted past; `frozen` still fails on it.
    pub repin_shelf: bool,
    /// The user asked for this install (`otc install`, `add`, `remove`):
    /// it writes the lock even where `[package] lock = "never"`, which
    /// only keeps incidental installs — a run, a test, the REPL, the
    /// language server, a check — from touching it.
    pub explicit: bool,
    /// Names whose lock pins may move to the newest their manifest
    /// allows (`otc upgrade`): every other versioned shelf or GitHub
    /// dependency keeps the copy its lock pins while the manifest allows
    /// it (rule 1), however the shelf changed.
    pub upgrade: Vec<String>,
    /// Fetch nothing: use what this machine already holds (a cached git
    /// checkout, a path, the shelf) and say a dependency that is not
    /// there as missing; write no lock. What opening a project does (a
    /// REPL serving an editor, `olang check --tier`): reading a folder
    /// must not clone from the network a manifest names. Fetching stays
    /// with the commands that install (`olang run`, `otc install`).
    pub offline: bool,
}

/// Resolve and fetch all dependencies of the project rooted at `root`,
/// writing `olang.lock` and returning the dependency map for execution.
///
/// This is the heart of `otc install` and of loading a project to run it.
pub fn install(root: &Path, options: &InstallOptions) -> Result<DependencyMap, PkgError> {
    let manifest = Manifest::load(root).map_err(|e| PkgError::Manifest(e.to_string()))?;

    // A lock written by a newer olang is refused by name: neither
    // replayed in part nor rewritten with what this olang does not know.
    if let Err(e) = Lockfile::load(root)
        && Lockfile::is_newer(&e)
    {
        return Err(PkgError::Lock(e));
    }

    // A lockfile that still covers the manifest is replayed exactly: the
    // pinned revs are fetched (offline once cached) and the lock is left
    // byte-identical. Only a manifest edit the lock doesn't cover — or an
    // explicit refresh — re-resolves.
    if !options.refresh
        && let Some(mut map) = replay_lock(root, &manifest, options)?
    {
        if let Some((_, e)) = transitive_shelf(&mut map, options).into_iter().next() {
            return Err(e);
        }
        return Ok(map);
    }

    let mut lock = Lockfile::new();
    let mut dep_map = DependencyMap::new();

    // Registry resolution (MVS) over just the registry dependencies.
    // A bare version string is a registry requirement only where a
    // registry is configured; otherwise it asks the shelf for that version.
    let registry_roots: Vec<(String, String)> = if options.registry.is_some() {
        manifest
            .dependencies
            .iter()
            .filter_map(|(name, dep)| dep.version_req().map(|r| (name.clone(), r.to_string())))
            .collect()
    } else {
        Vec::new()
    };
    let previous = Lockfile::load(root).ok();

    let resolution = if registry_roots.is_empty() {
        Default::default()
    } else {
        let reg = open_registry(
            options,
            "registry dependencies present but no registry configured",
        )?;
        resolver::resolve(&registry_roots, &reg).map_err(|e| PkgError::Resolve(e.to_string()))?
    };

    // Direct dependencies from the manifest.
    for (name, dep) in &manifest.dependencies {
        let (dir, locked) = resolve_dependency_with(
            root,
            name,
            dep,
            &resolution,
            options,
            previous.as_ref().and_then(|l| l.package.get(name)),
        )?;
        dep_map.insert(name.clone(), dir);
        lock.package.insert(name.clone(), locked);
    }

    // A dependency's own path dependencies, at any depth: each resolves
    // against the manifest that names it (a desktop app → a UI framework →
    // a TUI framework, all by path), and is locked by its path from this root. A
    // name the project or a nearer package already maps keeps that
    // mapping, as a direct dependency always wins.
    let mut queue: Vec<PathBuf> = dep_map.values().cloned().collect();
    let mut seen: std::collections::HashSet<PathBuf> = queue.iter().cloned().collect();
    while let Some(dir) = queue.pop() {
        let Ok(sub) = Manifest::load(&dir) else {
            continue;
        };
        for (name, dep) in &sub.dependencies {
            let Dependency::Path { path } = dep else {
                continue;
            };
            if dep_map.contains_key(name) {
                continue;
            }
            let target = normalize(&dir, path);
            if !target.exists() {
                return Err(PkgError::Resolve(format!(
                    "dependency '{}' of {} points at path '{}' ({}), which does not exist",
                    name,
                    dir.display(),
                    path,
                    target.display()
                )));
            }
            let recorded = relative_to(root, &target);
            dep_map.insert(name.clone(), target.clone());
            lock.package.insert(
                name.clone(),
                LockedPackage {
                    version: None,
                    source: LockedSource::Path { path: recorded },
                    checksum: cache::checksum_dir(&target).ok(),
                    dependencies: sub_dependency_names(&target),
                },
            );
            if seen.insert(target.clone()) {
                queue.push(target);
            }
        }
    }

    // Registry deps selected transitively but not named directly: fetch and
    // lock them too, so the whole graph is available.
    for (name, version) in &resolution {
        if lock.package.contains_key(name) {
            continue;
        }
        let reg = open_registry(options, "no registry configured")?;
        let (dir, release) = fetch_release(&reg, name, version, options, true)?;
        dep_map.insert(name.clone(), dir);
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

    // The olang the project pins, resolved; a lock's kept when the
    // manifest names none (src/pkg/toolchain.rs).
    let previous = Lockfile::load(root).ok().and_then(|l| l.olang);
    lock.olang = toolchain::lock_entry(manifest.package.olang.as_deref(), previous.as_deref());

    // Reproducibility gate for CI.
    if options.frozen {
        let existing = Lockfile::load(root).map_err(PkgError::Lock)?;
        if existing != lock && !existing.package.is_empty() {
            return Err(PkgError::Lock(
                "lockfile is out of date (run without --frozen to update)".into(),
            ));
        }
    }

    // An editor's passive children (a language server, a REPL, a check)
    // resolve as a run does but never write into the project they read:
    // OLANG_LOCK_READONLY=1 keeps the lock (or its absence) as it is. A
    // project whose manifest says `lock = "never"` (a test fixture) is read
    // the same way by everything but an explicit otc command.
    // A copy on the shelf is read-only: what it resolves to is never
    // written into it.
    let shelf_copy = root.starts_with(shelf::Shelf::home());
    if !lock_readonly()
        && !options.offline
        && !shelf_copy
        && (options.explicit || !manifest.lock_never())
    {
        lock.save(root).map_err(PkgError::Lock)?;
    }
    if options.explicit && !options.offline {
        projects::record(root);
    }
    if let Some((_, e)) = transitive_shelf(&mut dep_map, options).into_iter().next() {
        return Err(e);
    }
    Ok(dep_map)
}

/// A dependency's own shelf dependencies that the project does not name,
/// at any depth, added to `map`: each as the dependency's own `olang.lock`
/// pins it — that commit's snapshot, its checksum verified — or, where
/// that lock pins no commit, as this machine's shelf holds it (skipped
/// when the shelf does not hold it). A pinned dependency's lock is part
/// of its snapshot, so the project's lock pins these through it; they are
/// not written into the project's lock. A name
/// the project or a nearer package maps keeps that mapping. Returns the
/// pinned ones that could not be produced, by name.
fn transitive_shelf(map: &mut DependencyMap, options: &InstallOptions) -> Vec<(String, PkgError)> {
    let mut failed = Vec::new();
    let mut queue: Vec<PathBuf> = map.values().cloned().collect();
    let mut seen: std::collections::HashSet<PathBuf> = queue.iter().cloned().collect();
    while let Some(dir) = queue.pop() {
        let Ok(sub) = Manifest::load(&dir) else {
            continue;
        };
        let sub_lock = Lockfile::load(&dir).ok();
        for (name, dep) in &sub.dependencies {
            let Some((sname, _, ask)) = shelf_ask(name, dep, options) else {
                continue;
            };
            let sname = &sname;
            if let Some(have) = map.get(name) {
                // one project gets one version of a name: a library asking
                // for a version the project's copy does not meet is refused,
                // naming both
                if let Some(req) = ask.as_deref().and_then(|a| a.strip_prefix("version "))
                    && let Some(v) = shelf::folder_version(have)
                    && !shelf::version_matches(req, &v)
                    && !failed.iter().any(|(n, _)| n == name)
                {
                    failed.push((
                        name.clone(),
                        PkgError::Resolve(format!(
                            "'{name}': this project gets {name} {v} ({}), and {} asks for {name} {req}; \
                             one project uses one version of a name. Ask for a version both allow \
                             (`otc upgrade {name} --major` moves the project's requirement), or upgrade {}",
                            have.display(),
                            sub.package.name,
                            sub.package.name
                        )),
                    ));
                }
                continue;
            }
            if failed.iter().any(|(n, _)| n == name) {
                continue;
            }
            let pinned = sub_lock
                .as_ref()
                .and_then(|l| l.package.get(name))
                .filter(|l| {
                    matches!(&l.source, LockedSource::Shelf { shelf, rev: Some(_), .. } if shelf == sname)
                });
            let resolved = match pinned {
                Some(locked) => locked_dir(&dir, name, locked, options)
                    .and_then(|d| verify_checksum(name, locked.checksum.as_deref(), &d).map(|_| d)),
                // Unpinned and not on this shelf: left to the `use` that
                // names it, as before it was looked for here.
                None => match resolve_dependency(&dir, name, dep, &Default::default(), options) {
                    Ok((d, _)) => Ok(d),
                    Err(_) => continue,
                },
            };
            match resolved {
                Ok(target) => {
                    map.insert(name.clone(), target.clone());
                    if seen.insert(target.clone()) {
                        queue.push(target);
                    }
                }
                Err(e) => failed.push((
                    name.clone(),
                    PkgError::Resolve(format!(
                        "'{}', which {} depends on: {}",
                        name, sub.package.name, e
                    )),
                )),
            }
        }
    }
    failed
}

/// Whether this process may not write `olang.lock` (`OLANG_LOCK_READONLY=1`).
pub fn lock_readonly() -> bool {
    std::env::var("OLANG_LOCK_READONLY")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

/// `install`, but a dependency that cannot be resolved does not take the
/// others with it: every resolvable manifest dependency is returned, and
/// the failures come back by name with their reason. For running a
/// program — a module that never imports the missing package still runs,
/// and the `use` that does is told which package is missing and why. The
/// lock is left untouched on this path.
pub fn install_lenient(
    root: &Path,
    options: &InstallOptions,
) -> (DependencyMap, Vec<(String, String)>) {
    match install(root, options) {
        Ok(map) => (map, Vec::new()),
        Err(first) => {
            let mut map = DependencyMap::new();
            let mut missing = Vec::new();
            let Ok(manifest) = Manifest::load(root) else {
                return (map, vec![("<manifest>".to_string(), first.to_string())]);
            };
            let resolution = Default::default();
            let lock = Lockfile::load(root).ok();
            for (name, dep) in &manifest.dependencies {
                // A shelf library the lock pins at a commit is that
                // commit's snapshot, as the lock replays it; one this
                // machine cannot produce is missing, by that reason, rather
                // than run at whatever commit the shelf holds.
                if let Some(locked) = lock.as_ref().and_then(|l| l.package.get(name))
                    && matches!(&locked.source, LockedSource::Shelf { shelf, rev: Some(_), .. }
                        if shelf_ask(name, dep, options).is_some_and(|(s, _, _)| &s == shelf))
                {
                    match locked_dir(root, name, locked, options).and_then(|dir| {
                        verify_checksum(name, locked.checksum.as_deref(), &dir).map(|_| dir)
                    }) {
                        Ok(dir) => {
                            map.insert(name.clone(), dir);
                        }
                        Err(e) => missing.push((name.clone(), e.to_string())),
                    }
                    continue;
                }
                match resolve_dependency(root, name, dep, &resolution, options) {
                    Ok((dir, _)) => {
                        map.insert(name.clone(), dir);
                    }
                    // a registry dependency has no version here (nothing
                    // was resolved): the reason is the install's own
                    Err(_) if dep.is_registry() => missing.push((name.clone(), first.to_string())),
                    Err(e) => missing.push((name.clone(), e.to_string())),
                }
            }
            for (name, e) in transitive_shelf(&mut map, options) {
                missing.push((name, e.to_string()));
            }
            if missing.is_empty() {
                missing.push(("<dependencies>".to_string(), first.to_string()));
            }
            (map, missing)
        }
    }
}

/// Resolve one manifest dependency to (directory, lock entry).
fn resolve_dependency(
    root: &Path,
    name: &str,
    dep: &Dependency,
    resolution: &resolver::Resolution,
    options: &InstallOptions,
) -> Result<(PathBuf, LockedPackage), PkgError> {
    resolve_dependency_with(root, name, dep, resolution, options, None)
}

/// A dependency the shelf answers: `(shelf name, GitHub repository, what
/// is asked)`. A bare version string is one where no registry is
/// configured. `None` for a path, git or registry dependency.
pub fn shelf_ask(
    name: &str,
    dep: &Dependency,
    options: &InstallOptions,
) -> Option<(String, Option<String>, Option<String>)> {
    match dep {
        Dependency::Shelf { shelf, version } => Some((
            shelf.clone(),
            None,
            version.as_ref().map(|v| format!("version {v}")),
        )),
        Dependency::GitHub {
            github,
            version,
            tag,
            branch,
            rev,
        } => {
            let shelf_name = github.rsplit('/').next().unwrap_or(github).to_string();
            let ask = rev
                .as_ref()
                .map(|r| format!("rev {r}"))
                .or_else(|| tag.as_ref().map(|t| format!("tag {t}")))
                .or_else(|| branch.as_ref().map(|b| format!("branch {b}")))
                .or_else(|| version.as_ref().map(|v| format!("version {v}")));
            // the library's own name, when the shelf holds this repository
            let shelf_name = shelf::Shelf::load()
                .ok()
                .and_then(|s| {
                    s.libraries
                        .iter()
                        .find(|(n, l)| {
                            n.as_str() == name
                                && l.copies
                                    .iter()
                                    .any(|c| shelf::same_origin(c.from.as_deref(), github))
                        })
                        .map(|(n, _)| n.clone())
                })
                .unwrap_or_else(|| {
                    if name.is_empty() {
                        shelf_name
                    } else {
                        name.to_string()
                    }
                });
            Some((shelf_name, Some(github.clone()), ask))
        }
        Dependency::Registry(v) if options.registry.is_none() => {
            Some((name.to_string(), None, Some(format!("version {v}"))))
        }
        Dependency::RegistryExplicit {
            version,
            registry: None,
        } if options.registry.is_none() => {
            Some((name.to_string(), None, Some(format!("version {version}"))))
        }
        _ => None,
    }
}

/// Resolve a shelf or GitHub dependency by the rules (docs/packages.md, "Which copy a dependency gets"):
/// 1. the copy `previous` (its lock entry) pins, while the manifest asks
///    the same and the name is not being upgraded;
/// 2. what the manifest asks for, newest held first, fetched from GitHub
///    when none is held and the dependency names its repository;
/// 3. with nothing asked, the shelf's default, local folder, or newest copy.
fn resolve_shelf(
    name: &str,
    sname: &str,
    github: Option<&str>,
    ask: Option<&str>,
    options: &InstallOptions,
    previous: Option<&LockedPackage>,
) -> Result<(PathBuf, LockedPackage), PkgError> {
    let mut shelf = shelf::Shelf::load().map_err(|e| PkgError::Resolve(format!("shelf: {}", e)))?;
    let shelf_err = |e: shelf::ShelfError| PkgError::Resolve(e.to_string());
    let lock_entry = |dir: &Path, version: Option<String>, rev: Option<String>| LockedPackage {
        version,
        source: LockedSource::Shelf {
            shelf: sname.to_string(),
            rev,
            github: github.map(str::to_string),
            reference: ask.map(str::to_string),
        },
        checksum: cache::checksum_dir(dir).ok(),
        dependencies: sub_dependency_names(dir),
    };
    // rule 1: the lock's copy, while the manifest asks the same
    if ask.is_some()
        && !options.refresh
        && !options.upgrade.iter().any(|n| n == name)
        && let Some(prev) = previous
        && let LockedSource::Shelf {
            shelf: s,
            rev: Some(rev),
            github: g,
            reference,
        } = &prev.source
        && s == sname
        && g.as_deref() == github
        && reference.as_deref() == ask
        && let Some(dir) = shelf.held_copy(sname, rev)
    {
        return Ok((
            dir.clone(),
            lock_entry(&dir, prev.version.clone(), Some(rev.clone())),
        ));
    }
    let lib = shelf.library(sname);
    let found: Option<(PathBuf, Option<String>, Option<String>)> = match ask {
        None => lib
            .and_then(|l| l.pick())
            .map(|(p, _)| (p.path(), p.version(), p.commit().map(str::to_string))),
        Some(a) => {
            let (kind, value) = a.split_once(' ').unwrap_or((a, ""));
            let pick = lib.and_then(|l| match kind {
                "version" => l.select(value, github),
                "tag" | "branch" => l.by_ref(github, value).map(shelf::Pick::Copy),
                "rev" => l.by_commit(value).map(shelf::Pick::Copy),
                _ => None,
            });
            pick.map(|p| (p.path(), p.version(), p.commit().map(str::to_string)))
        }
    };
    if let Some((dir, version, rev)) = found {
        if !dir.exists() {
            return Err(PkgError::Resolve(format!(
                "shelf library '{}' points at '{}', which no longer exists \
                 (re-register it: `otc shelf add <path>`, or forget it: `otc shelf remove {} --yes`)",
                sname,
                dir.display(),
                sname
            )));
        }
        return Ok((dir.clone(), lock_entry(&dir, version, rev)));
    }
    // nothing held answers: fetch from GitHub when the dependency says where
    if let Some(repo) = github {
        if options.offline {
            return Err(PkgError::Fetch(format!(
                "'{name}' ({repo}) is not on this machine's shelf; it is fetched by `otc install`"
            )));
        }
        let gask = match ask.map(|a| a.split_once(' ').unwrap_or((a, ""))) {
            None => github::Ask::Newest,
            Some(("version", v)) => github::Ask::Version(v.to_string()),
            Some(("tag", t)) => github::Ask::Tag(t.to_string()),
            Some(("branch", b)) => github::Ask::Branch(b.to_string()),
            Some(("rev", r)) => github::Ask::Rev(r.to_string()),
            Some(_) => github::Ask::Newest,
        };
        let f = github::fetch(&mut shelf, repo, &gask, Some(sname), false).map_err(shelf_err)?;
        shelf.save().map_err(shelf_err)?;
        let dir = f.copy.path();
        return Ok((
            dir.clone(),
            lock_entry(&dir, Some(f.copy.version.clone()), f.copy.commit.clone()),
        ));
    }
    let held = lib
        .map(|l| {
            let mut v: Vec<String> = l.copies.iter().map(|c| c.version.clone()).collect();
            if l.local.is_some() {
                v.push("a local folder".to_string());
            }
            v.join(", ")
        })
        .unwrap_or_default();
    Err(PkgError::Resolve(match (lib, ask) {
        (None, _) => format!(
            "dependency '{}' names shelf library '{}', which is not on your shelf \
             (put it there: `otc shelf add <path>` or `otc shelf add <owner/repo>`)",
            name, sname
        ),
        (Some(_), Some(a)) => format!(
            "dependency '{name}' asks for {sname} {}, and the shelf holds {held}: \
             `otc shelf add <owner/repo>@<tag>` fetches another version, `otc shelf list {sname}` shows them",
            a.split_once(' ').map(|x| x.1).unwrap_or(a)
        ),
        (Some(_), None) => {
            format!("shelf library '{sname}' holds nothing (`otc shelf list {sname}`)")
        }
    }))
}

/// `resolve_dependency`, with the dependency's previous lock entry for
/// rule 1.
fn resolve_dependency_with(
    root: &Path,
    name: &str,
    dep: &Dependency,
    resolution: &resolver::Resolution,
    options: &InstallOptions,
    previous: Option<&LockedPackage>,
) -> Result<(PathBuf, LockedPackage), PkgError> {
    if let Some((sname, github, ask)) = shelf_ask(name, dep, options) {
        return resolve_shelf(
            name,
            &sname,
            github.as_deref(),
            ask.as_deref(),
            options,
            previous,
        );
    }
    match dep {
        Dependency::Path { path } => {
            let dir = normalize(root, path);
            // A path dependency pointing nowhere used to resolve silently:
            // checksum_dir(...).ok() swallowed the error, the lock recorded a
            // checksum of nothing, and the failure only surfaced much later
            // as an opaque "Cannot find module" at run time. Fail here,
            // naming the dependency and its bad path.
            if !dir.exists() {
                return Err(PkgError::Resolve(format!(
                    "dependency '{}' points at path '{}' ({}), which does not exist",
                    name,
                    path,
                    dir.display()
                )));
            }
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
            let fetched = cache::fetch_git(git, &git_ref, options.offline)
                .map_err(|e| PkgError::Fetch(e.to_string()))?;
            let checksum = cache::checksum_dir(&fetched.dir).ok();
            let deps = sub_dependency_names(&fetched.dir);
            Ok((
                fetched.dir.clone(),
                LockedPackage {
                    version: None,
                    source: LockedSource::Git {
                        git: git.clone(),
                        rev: fetched.rev.unwrap_or_default(),
                        reference: requested_git_ref(rev, tag, branch),
                    },
                    checksum,
                    dependencies: deps,
                },
            ))
        }
        Dependency::Shelf { .. } | Dependency::GitHub { .. } => {
            unreachable!("answered by the shelf above")
        }
        Dependency::Registry(_) | Dependency::RegistryExplicit { .. } => {
            let version = resolution.get(name).ok_or_else(|| {
                PkgError::Resolve(format!("registry dependency '{}' was not resolved", name))
            })?;
            let reg = open_registry(options, "no registry configured")?;
            let (dir, release) = fetch_release(&reg, name, version, options, true)?;
            Ok((
                dir,
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

/// The ref a manifest git dependency asks for, in the same precedence order
/// resolve_dependency fetches it (rev, then tag, then branch; None = HEAD).
/// Stored in the lock so a later install can tell whether the request changed.
fn requested_git_ref(
    rev: &Option<String>,
    tag: &Option<String>,
    branch: &Option<String>,
) -> Option<String> {
    rev.clone()
        .or_else(|| tag.clone())
        .or_else(|| branch.clone())
}

/// Try to satisfy the manifest exactly from the existing lockfile, fetching
/// the pinned sources without re-resolving. Returns Ok(None) when the lock
/// doesn't cover the manifest — no lockfile, a dependency added/removed, a
/// source or requested git ref changed, or a pinned registry version that no
/// longer satisfies its requirement — in which case the caller re-resolves.
fn replay_lock(
    root: &Path,
    manifest: &Manifest,
    options: &InstallOptions,
) -> Result<Option<DependencyMap>, PkgError> {
    let Ok(lock) = Lockfile::load(root) else {
        return Ok(None);
    };
    if lock.package.is_empty() {
        return Ok(None);
    }

    // A pin the lock's olang no longer satisfies (or one it never
    // recorded) re-resolves, so the lock says what answers it now.
    if let Some(req) = manifest.package.olang.as_deref()
        && !lock
            .olang
            .as_deref()
            .is_some_and(|v| toolchain::satisfies(req, v))
        && toolchain::lock_entry(Some(req), None).is_some()
    {
        return Ok(None);
    }

    // Every manifest dependency must be pinned compatibly.
    for (name, dep) in &manifest.dependencies {
        let Some(locked) = lock.package.get(name) else {
            return Ok(None);
        };
        let covered = match (dep, &locked.source) {
            (Dependency::Path { path }, LockedSource::Path { path: locked_path }) => {
                path == locked_path
            }
            (
                dep,
                LockedSource::Shelf {
                    shelf: locked,
                    rev: locked_rev,
                    github: locked_github,
                    reference,
                },
            ) if shelf_ask(name, dep, options).is_some() => {
                let (sname, github, ask) = shelf_ask(name, dep, options).expect("checked");
                &sname == locked
                    && github == *locked_github
                    && ask == *reference
                    && !options.upgrade.iter().any(|n| n == name)
                    // a bare name follows the shelf: a re-shelved one re-pins
                    && (ask.is_some()
                        || !options.repin_shelf
                        || !shelf_moved(&sname, locked_rev.as_deref()))
            }
            (
                Dependency::Git {
                    git,
                    tag,
                    rev,
                    branch,
                },
                LockedSource::Git {
                    git: locked_git,
                    reference,
                    ..
                },
            ) => git == locked_git && *reference == requested_git_ref(rev, tag, branch),
            (dep, LockedSource::Registry { .. }) if dep.is_registry() => {
                match (&locked.version, dep.version_req()) {
                    (Some(v), Some(req)) => {
                        match (semver::Version::parse(v), semver::VersionReq::parse(req)) {
                            (Ok(v), Ok(req)) => req.matches(&v),
                            _ => false,
                        }
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if !covered {
            return Ok(None);
        }
    }

    // Lock entries beyond the manifest: a path/git entry means a removed
    // dependency (stale lock); a registry entry is legitimate only while
    // another locked package still needs it transitively.
    for (name, locked) in &lock.package {
        if manifest.dependencies.contains_key(name) {
            continue;
        }
        // A path or shelf entry is a dependency's own (locked by
        // `install`), legitimate on the same terms.
        match &locked.source {
            LockedSource::Registry { .. }
            | LockedSource::Path { .. }
            | LockedSource::Shelf { .. } => {
                let needed = lock
                    .package
                    .values()
                    .any(|p| p.dependencies.iter().any(|d| d == name));
                if !needed {
                    return Ok(None);
                }
            }
            _ => return Ok(None),
        }
    }

    // Fetch exactly what the lock pins. Fetch failures propagate: a pinned
    // rev that can't be produced is a real problem, not a reason to silently
    // drift off the lock.
    let mut map = DependencyMap::new();
    for (name, locked) in &lock.package {
        // A lock that names a path this machine does not have (a clone of
        // a project whose lock was written elsewhere, a moved checkout) is
        // not a reason to fail: re-resolve from the manifest instead.
        // Not so a pinned shelf library: the lock says which commit the
        // name means, and that commit's snapshot is what runs — never
        // whatever the shelf holds now, the drift the pin exists to stop.
        // A snapshot is the same tree on every machine: its checksum holds.
        if matches!(locked.source, LockedSource::Shelf { rev: Some(_), .. }) {
            let dir = locked_dir(root, name, locked, options)?;
            verify_checksum(name, locked.checksum.as_deref(), &dir)?;
            map.insert(name.clone(), dir);
            continue;
        }
        if matches!(
            locked.source,
            LockedSource::Path { .. } | LockedSource::Shelf { .. }
        ) {
            match locked_dir(root, name, locked, options) {
                Ok(dir) if dir.exists() => {}
                _ => return Ok(None),
            }
        }
        let dir = locked_dir(root, name, locked, options)?;
        // An edited path or shelf library is development, not tampering —
        // but an install the user asked for re-pins it (and `--frozen`
        // fails on it), rather than keep a checksum that is no longer
        // true (OL-367). A run replays it as it stands.
        if options.explicit && library_edited(locked, &dir) {
            return Ok(None);
        }
        // Fetched sources must still match the checksum the lock recorded —
        // this is where the trust model's tamper detection actually bites.
        // Path and shelf deps are exempt: editing one is normal development.
        // A shelf library pinned at a commit is not: it is a snapshot, and
        // the same commit is the same tree on every machine.
        if !matches!(
            locked.source,
            LockedSource::Path { .. } | LockedSource::Shelf { rev: None, .. }
        ) {
            verify_checksum(name, locked.checksum.as_deref(), &dir)?;
        }
        map.insert(name.clone(), dir);
    }
    Ok(Some(map))
}

/// Whether a path or unpinned shelf library at `dir` is no longer the
/// tree its lock entry's checksum recorded: it was edited since.
fn library_edited(locked: &LockedPackage, dir: &Path) -> bool {
    matches!(
        locked.source,
        LockedSource::Path { .. } | LockedSource::Shelf { rev: None, .. }
    ) && locked
        .checksum
        .as_deref()
        .is_some_and(|recorded| cache::checksum_dir(dir).is_ok_and(|actual| actual != recorded))
}

/// The path and unpinned shelf libraries whose contents changed since
/// the lock recorded them, by name: `otc install` says these before it
/// re-pins them.
pub fn edited_libraries(root: &Path) -> Vec<String> {
    let Ok(lock) = Lockfile::load(root) else {
        return Vec::new();
    };
    lock.package
        .iter()
        .filter(|(name, locked)| {
            locked_dir(root, name, locked, &InstallOptions::default())
                .is_ok_and(|dir| dir.exists() && library_edited(locked, &dir))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// Whether this machine's shelf now holds `name` at a different revision
/// than a lock entry recorded (`locked_rev`; None = following its
/// directory). A name the shelf no longer holds is not a move: replaying
/// it reports the missing library.
fn shelf_moved(name: &str, locked_rev: Option<&str>) -> bool {
    let Ok(shelf) = shelf::Shelf::load() else {
        return false;
    };
    shelf.resolve(name).is_some() && shelf.rev_of(name) != locked_rev
}

/// A shelf library whose lock entry no longer matches the shelf: its name,
/// the rev the lock recorded, and the rev the shelf now holds (None =
/// following its directory). `otc install` reports these before it
/// re-pins them.
pub fn moved_shelf_pins(root: &Path) -> Vec<(String, Option<String>, Option<String>)> {
    let Ok(lock) = Lockfile::load(root) else {
        return Vec::new();
    };
    let Ok(shelf) = shelf::Shelf::load() else {
        return Vec::new();
    };
    lock.package
        .values()
        .filter_map(|locked| match &locked.source {
            // a bare name follows the shelf; one asked for a version keeps its copy
            LockedSource::Shelf {
                shelf: name,
                rev,
                reference: None,
                ..
            } if shelf.resolve(name).is_some() && shelf.rev_of(name) != rev.as_deref() => Some((
                name.clone(),
                rev.clone(),
                shelf.rev_of(name).map(str::to_string),
            )),
            _ => None,
        })
        .collect()
}

/// One line per shelf library whose lock entry is behind (or otherwise
/// off) the shelf's pin, for a doctor to say: information, not a fault —
/// the lock's commit is what runs, and `otc install` moves it. Says
/// whether the locked commit's snapshot is here, can be made again, or
/// neither.
pub fn lock_behind_shelf(root: &Path) -> Vec<String> {
    let Ok(shelf) = shelf::Shelf::load() else {
        return Vec::new();
    };
    let short = |r: &Option<String>| match r {
        Some(r) => format!("at {}", r.chars().take(12).collect::<String>()),
        None => "following its directory".to_string(),
    };
    moved_shelf_pins(root)
        .into_iter()
        .map(|(name, was, now)| {
            let held = match &was {
                Some(rev) if !shelf::Shelf::is_pin(&name, rev) => {
                    "; that is not a commit id".to_string()
                }
                Some(rev) if shelf.held_copy(&name, rev).is_some() => {
                    "; its snapshot is here and is what runs".to_string()
                }
                Some(_) if shelf.repo_of(&name).is_some_and(|r| r.exists()) => format!(
                    "; its snapshot is made again from {} when the project runs",
                    shelf.repo_of(&name).map(|r| r.display().to_string()).unwrap_or_default()
                ),
                Some(_) => "; its snapshot is not here, and the shelf knows no repository to make it from".to_string(),
                None => String::new(),
            };
            format!(
                "olang.lock has shelf library '{}' {}, the shelf has it {}{} (`otc install` moves the lock to the shelf)",
                name,
                short(&was),
                short(&now),
                held
            )
        })
        .collect()
}

/// Produce the on-disk directory a lock entry pins, fetching if needed.
fn locked_dir(
    root: &Path,
    name: &str,
    locked: &LockedPackage,
    options: &InstallOptions,
) -> Result<PathBuf, PkgError> {
    match &locked.source {
        LockedSource::Path { path } => Ok(normalize(root, path)),
        LockedSource::Shelf {
            shelf: shelf_name,
            rev,
            github,
            ..
        } => {
            let mut shelf =
                shelf::Shelf::load().map_err(|e| PkgError::Resolve(format!("shelf: {}", e)))?;
            // A lock that pins a revision is a promise about what the name
            // means: that commit's snapshot is used, whatever commit the
            // shelf holds the name at now (`otc install` moves the lock to
            // the shelf's pin). The shelf keeps its copies, makes a
            // missing folder snapshot again from its repository, and
            // fetches a missing GitHub commit from the repository the lock
            // names.
            if let Some(wanted) = rev {
                if let Some(repo) = github
                    && shelf::Shelf::is_pin(shelf_name, wanted)
                    && shelf.held_copy(shelf_name, wanted).is_none()
                    && !options.offline
                {
                    return github::copy_at(&mut shelf, shelf_name, repo, wanted, true)
                        .map_err(|e| PkgError::Fetch(e.to_string()))?
                        .ok_or_else(|| {
                            PkgError::Fetch(format!("{repo} at {wanted} could not be fetched"))
                        });
                }
                return pinned_shelf_dir(&mut shelf, shelf_name, wanted);
            }
            shelf.resolve(shelf_name).ok_or_else(|| {
                PkgError::Resolve(format!(
                    "the lock names shelf library '{}', which is not on this machine's shelf \
                     (put it there: `otc shelf add <path>`)",
                    shelf_name
                ))
            })
        }
        LockedSource::Git { git, rev, .. } => {
            Ok(
                cache::fetch_git(git, &cache::GitRef::Rev(rev.clone()), options.offline)
                    .map_err(|e| PkgError::Fetch(e.to_string()))?
                    .dir,
            )
        }
        LockedSource::Registry { .. } => {
            let version = locked.version.as_deref().ok_or_else(|| {
                PkgError::Lock(format!("locked registry package '{}' has no version", name))
            })?;
            let version =
                semver::Version::parse(version).map_err(|e| PkgError::Lock(e.to_string()))?;
            let reg = open_registry(
                options,
                "lockfile pins registry packages but no registry is configured",
            )?;
            Ok(fetch_release(&reg, name, &version, options, false)?.0)
        }
    }
}

/// Why `dir` cannot be the path dependency `name`, or None when it can:
/// it holds an `olang.toml`, or a root module `use name` would load
/// (`index.ol`, `mod.ol`, `<name>.ol`, `src/index.ol`). `otc add` refuses
/// what `olang doctor` flags, in the same words (OL-368).
pub fn path_dependency_problem(dir: &Path, name: &str) -> Option<String> {
    let roots = [
        dir.join("olang.toml"),
        dir.join("index.ol"),
        dir.join("mod.ol"),
        dir.join(format!("{name}.ol")),
        dir.join("src").join("index.ol"),
    ];
    if roots.iter().any(|p| p.is_file()) {
        return None;
    }
    Some(format!(
        "path {} has no olang.toml or root module (index.ol, mod.ol, {name}.ol or src/index.ol)",
        dir.display()
    ))
}

/// The directory a project's shelf dependency `name` (shelf library
/// `shelf_name`) reads from, for the tools that read a project without
/// installing it (a check, the language server, macro expansion): the
/// snapshot the project's lock pins, as a run uses, else what the shelf
/// holds now. None when neither is here.
pub fn shelf_dependency_dir(root: &Path, name: &str, shelf_name: &str) -> Option<PathBuf> {
    let shelf = shelf::Shelf::load().ok()?;
    let locked_rev = Lockfile::load(root).ok().and_then(|l| {
        l.package.get(name).and_then(|p| match &p.source {
            LockedSource::Shelf {
                shelf,
                rev: Some(rev),
                ..
            } if shelf == shelf_name => Some(rev.clone()),
            _ => None,
        })
    });
    match locked_rev {
        Some(rev) => shelf.held_copy(shelf_name, &rev),
        None => shelf.resolve(shelf_name),
    }
}

/// The snapshot of shelf library `name` at commit `wanted`, which a lock
/// pins: the shelf's pin when it is that commit, else the copy of that commit,
/// else one made again from the library's repository. Neither there nor
/// makeable is an error that names the library, both commits and the
/// commands that settle it.
fn pinned_shelf_dir(
    shelf: &mut shelf::Shelf,
    name: &str,
    wanted: &str,
) -> Result<PathBuf, PkgError> {
    if !shelf::Shelf::is_pin(name, wanted) {
        return Err(PkgError::Lock(format!(
            "the lock pins shelf library {:?} at {:?}, which is not a commit id \
             (run `otc install` to pin it again)",
            name, wanted
        )));
    }
    let short = |r: &str| r[..r.len().min(12)].to_string();
    let has = match shelf.rev_of(name) {
        Some(have) => format!("the shelf gives it at {}", short(have)),
        None if shelf.resolve(name).is_some() => {
            "the shelf has it following its directory".to_string()
        }
        None => "the shelf does not hold it".to_string(),
    };
    match shelf.pinned_copy(name, wanted) {
        Ok(Some(dir)) => Ok(dir),
        Ok(None) => Err(PkgError::Resolve(format!(
            "the lock pins shelf library '{name}' at {wanted_s}, which this machine does not hold \
             (no copy of that commit, and the shelf knows no repository to make it from); {has}. \
             Run `otc install` to move the lock to the shelf's pin, \
             or shelve that commit: `otc shelf add <path> --rev {wanted_s}`",
            wanted_s = short(wanted),
        ))),
        Err(e) => Err(PkgError::Resolve(format!(
            "the lock pins shelf library '{name}' at {wanted_s}, which this machine does not hold, \
             and it could not be made from {repo}: {e}; {has}. \
             Run `otc install` to move the lock to the shelf's pin",
            wanted_s = short(wanted),
            repo = shelf
                .repo_of(name)
                .map(|r| r.display().to_string())
                .unwrap_or_default(),
        ))),
    }
}

/// Compare a directory's content checksum against an expected value. A
/// missing expectation passes (old lockfiles and path deps have none); a
/// mismatch is an error naming both sums.
/// The registry an install uses: the one its options name, else
/// `OLANG_REGISTRY`, else the default; `missing` is said when there is none.
fn open_registry(options: &InstallOptions, missing: &str) -> Result<registry::Registry, PkgError> {
    let spec = registry::configured(options.registry.as_deref()).ok_or_else(|| {
        PkgError::Registry(format!(
            "{missing} (set OLANG_REGISTRY to an index's folder or its git address)"
        ))
    })?;
    registry::Registry::open(&spec, options.offline, options.refresh)
        .map_err(|e| PkgError::Registry(e.to_string()))
}

/// Fetch one registry release and answer the folder that holds the
/// package (the release's `subdir` inside the checkout, when it names one).
///
/// With `verify`, as a resolution does, the index is checked against
/// what was fetched: the release must carry a checksum and the source
/// match it, the package's olang.toml must say the entry's name and
/// version, and every registry dependency that olang.toml names must be
/// in the entry, since resolution reads the entry alone. A lock's replay
/// passes false: its own checksum is checked after.
fn fetch_release(
    reg: &registry::Registry,
    name: &str,
    version: &semver::Version,
    options: &InstallOptions,
    verify: bool,
) -> Result<(PathBuf, registry::Release), PkgError> {
    let release = reg
        .release(name, version)
        .map_err(|e| PkgError::Registry(e.to_string()))?;
    let fetched = cache::fetch_git(
        &release.git,
        &cache::GitRef::Rev(release.rev.clone()),
        options.offline,
    )
    .map_err(|e| PkgError::Fetch(e.to_string()))?;
    let dir = match release.subdir.as_deref() {
        None | Some("") | Some(".") => fetched.dir,
        Some(sub) => {
            let inside = Path::new(sub)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
            let dir = fetched.dir.join(sub);
            if !inside || !dir.is_dir() {
                return Err(PkgError::Registry(format!(
                    "the index entry for {name} {version} names the folder '{sub}', which is not in {} at {}",
                    release.git, release.rev
                )));
            }
            dir
        }
    };
    if !verify {
        return Ok((dir, release));
    }
    if release.checksum.is_none() {
        return Err(PkgError::Registry(format!(
            "the index entry for {name} {version} has no checksum: a release without one is not installed (`otc publish` writes it)"
        )));
    }
    verify_checksum(name, release.checksum.as_deref(), &dir)?;
    let manifest = Manifest::load(&dir).map_err(|e| {
        PkgError::Registry(format!(
            "{name} {version} from the registry has no readable olang.toml: {e}"
        ))
    })?;
    if manifest.package.name != name || &manifest.package.version != version {
        return Err(PkgError::Registry(format!(
            "the index entry says {name} {version}, but the source it points at is {} {}",
            manifest.package.name, manifest.package.version
        )));
    }
    for (dep, source) in &manifest.dependencies {
        if source.is_registry() && !release.dependencies.contains_key(dep) {
            return Err(PkgError::Registry(format!(
                "the index entry for {name} {version} leaves out '{dep}', which its olang.toml depends on: the entry is wrong, and {name} would not find '{dep}' when it runs"
            )));
        }
    }
    if release.withdrawn && options.explicit {
        eprintln!(
            "note: {name} {version} was withdrawn from the registry{}",
            release
                .reason
                .as_deref()
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        );
    }
    Ok((dir, release))
}

fn verify_checksum(name: &str, expected: Option<&str>, dir: &Path) -> Result<(), PkgError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = cache::checksum_dir(dir).map_err(|e| {
        PkgError::Fetch(format!(
            "cannot checksum '{}' at {}: {}",
            name,
            dir.display(),
            e
        ))
    })?;
    if actual != expected {
        return Err(PkgError::Fetch(format!(
            "checksum mismatch for '{}': the fetched source at {} does not match what was recorded (expected {}, got {}) — the upstream content changed after it was locked/published",
            name,
            dir.display(),
            expected,
            actual
        )));
    }
    Ok(())
}

/// How one lock entry fared under `verify`.
#[derive(Debug, PartialEq, Eq)]
pub enum VerifyStatus {
    /// Content matches the recorded checksum.
    Ok,
    /// Content differs. For git/registry sources this is tampering or an
    /// upstream rewrite; for a path dependency it usually just means the
    /// package was edited since the last install.
    Mismatch { expected: String, actual: String },
    /// The lock entry carries no checksum to compare against.
    NoChecksum,
}

/// Re-check every lock entry's content against its recorded checksum
/// (`otc pkg verify`). Does not modify the lockfile.
pub fn verify(
    root: &Path,
    options: &InstallOptions,
) -> Result<Vec<(String, LockedSource, VerifyStatus)>, PkgError> {
    let lock = Lockfile::load(root).map_err(PkgError::Lock)?;
    let mut report = Vec::new();
    for (name, locked) in &lock.package {
        let dir = locked_dir(root, name, locked, options)?;
        let status = match locked.checksum.as_deref() {
            None => VerifyStatus::NoChecksum,
            Some(expected) => {
                let actual = cache::checksum_dir(&dir).map_err(|e| {
                    PkgError::Fetch(format!(
                        "cannot checksum '{}' at {}: {}",
                        name,
                        dir.display(),
                        e
                    ))
                })?;
                if actual == expected {
                    VerifyStatus::Ok
                } else {
                    VerifyStatus::Mismatch {
                        expected: expected.to_string(),
                        actual,
                    }
                }
            }
        };
        report.push((name.clone(), locked.source.clone(), status));
    }
    Ok(report)
}

/// The direct dependency names of a sub-package (for the lock graph), read
/// from its own `olang.toml` if it has one.
fn sub_dependency_names(dir: &Path) -> Vec<String> {
    Manifest::load(dir)
        .map(|m| m.dependencies.keys().cloned().collect())
        .unwrap_or_default()
}

/// `target` as a path from `root` (`../ui/../tui` style, through
/// their common ancestor), for a lock that stays valid wherever the
/// workspace is checked out; absolute when they share nothing.
fn relative_to(root: &Path, target: &Path) -> String {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (root, target) = (canon(root), canon(target));
    let r: Vec<_> = root.components().collect();
    let t: Vec<_> = target.components().collect();
    let common = r.iter().zip(t.iter()).take_while(|(a, b)| a == b).count();
    if common <= 1 {
        return target.display().to_string();
    }
    let mut out = PathBuf::new();
    for _ in common..r.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c.as_os_str());
    }
    out.display().to_string()
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

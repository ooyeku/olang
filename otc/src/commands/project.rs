//! Project dependency management: `otc add`, `remove`, `list`,
//! `install`, `outdated`, `upgrade` (OL-582, OL-583, OL-584).
//!
//! These edit `olang.toml`, resolve into `olang.lock`, and report the
//! graph. A dependency is a folder (by path), a library on the shelf (by
//! name, perhaps with a version), or a GitHub repository held on the
//! shelf; every change ends by resolving, so the lockfile is never left
//! behind the manifest, and a failed one leaves both as they were.

use super::shelf::{Spec, capability_lines, read_spec, show_capabilities};
use super::{Changes, ctx, emit, fail};
use crate::say;
use olang::pkg::github;
use olang::pkg::lock::{LockedPackage, LockedSource, Lockfile};
use olang::pkg::manifest::{Dependency, Manifest, PackageMeta};
use olang::pkg::shelf::{Pick, Shelf};
use olang::pkg::{InstallOptions, install, shelf_ask};
use semver::Version;
use std::path::{Path, PathBuf};

/// The nearest enclosing project root, or None.
fn find_root() -> Option<PathBuf> {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| Manifest::find_root(&cwd))
}

fn require_root() -> anyhow::Result<PathBuf> {
    find_root().ok_or_else(|| {
        fail(
            format!(
                "no olang.toml here or above ({})",
                std::env::current_dir()
                    .map(|d| d.display().to_string())
                    .unwrap_or_default()
            ),
            "otc new <name> (or otc add <library>, which starts one here)",
        )
    })
}

/// A fresh `olang.toml` in `cwd` — `otc add` in a fresh directory
/// should start the project, not lecture about a missing init step.
/// Written only once the dependency checked out (OL-368).
fn init_manifest(cwd: &Path) -> anyhow::Result<()> {
    let name = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "my-project".to_string());
    let manifest = Manifest {
        package: PackageMeta {
            name: name.clone(),
            version: Version::new(0, 1, 0),
            description: None,
            authors: Vec::new(),
            license: None,
            assets: Vec::new(),
            public: Vec::new(),
            olang: None,
            lock: None,
        },
        dependencies: Default::default(),
        capabilities: None,
        check: None,
    };
    manifest.save(cwd).map_err(|e| anyhow::anyhow!("{}", e))?;
    say!("No olang.toml found — created one for '{}'", name);
    Ok(())
}

fn registry_from_env() -> Option<PathBuf> {
    std::env::var("OLANG_REGISTRY").ok().map(PathBuf::from)
}

fn options() -> InstallOptions {
    InstallOptions {
        registry: registry_from_env(),
        explicit: true,
        ..Default::default()
    }
}

/// The manifest and lock as they were, put back when a change fails.
struct Saved {
    root: PathBuf,
    manifest: Option<Vec<u8>>,
    lock: Option<Vec<u8>>,
}

impl Saved {
    fn take(root: &Path) -> Saved {
        Saved {
            root: root.to_path_buf(),
            manifest: std::fs::read(root.join("olang.toml")).ok(),
            lock: std::fs::read(root.join("olang.lock")).ok(),
        }
    }
    fn restore(&self) {
        for (file, was) in [("olang.toml", &self.manifest), ("olang.lock", &self.lock)] {
            let path = self.root.join(file);
            match was {
                Some(bytes) => {
                    let _ = std::fs::write(&path, bytes);
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

/// The registry's answer for a name the shelf does not settle: `name`
/// when the shelf has no such library, `name@1.2` when it holds none that
/// version allows. None with no registry configured, or when the shelf
/// answers: a library on this machine comes first.
fn from_registry(spec: &str) -> anyhow::Result<Option<(String, Dependency)>> {
    if olang::pkg::registry::configured(None).is_none() {
        return Ok(None);
    }
    let (name, version) = match spec.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (spec, None),
    };
    let shelf = super::shelf::load()?;
    let shelved = shelf.library(name).is_some_and(|lib| match version {
        Some(v) => lib.select(v, None).is_some(),
        None => lib.pick().is_some(),
    });
    if shelved {
        return Ok(None);
    }
    super::registry::dependency_for(spec)
}

/// `otc add <spec>`: one argument, three readings, said as understood:
///
///   otc add ../my-lib            a folder (anything that exists here, or `.`/`/`-led)
///   otc add ooyeku/geometry@v1   a GitHub repository, held on the shelf
///   otc add geometry[@1.2]       a library on your shelf, perhaps a version of it
///
/// `--path` forces the folder reading.
pub fn add(spec: &str, path_flag: Option<&str>, force: bool) -> anyhow::Result<()> {
    // Nothing is written until the dependency checks out: a failed add
    // leaves the project as it was, with no new olang.toml (OL-368).
    let found = find_root();
    let root = match &found {
        Some(r) => r.clone(),
        None => std::env::current_dir()?,
    };
    let dependencies = match &found {
        Some(r) => {
            Manifest::load(r)
                .map_err(|e| fail(e.to_string(), "otc doctor"))?
                .dependencies
        }
        None => Default::default(),
    };
    let mut changes = Changes::new("add");

    let as_path = path_flag.is_some()
        || spec.contains('\\')
        || spec.starts_with('.')
        || Path::new(spec).is_absolute()
        || (spec.contains('/') && Path::new(spec).exists());
    let (name, dep) = if as_path {
        let given = path_flag.unwrap_or(spec);
        say!("{given}: a folder");
        // The path the user typed is relative to where they stand; the
        // manifest records it relative to the project root, so the
        // project stays coherent no matter which subdirectory the
        // command ran from.
        let cwd = std::env::current_dir()?;
        let absolute = if Path::new(given).is_absolute() {
            PathBuf::from(given)
        } else {
            cwd.join(given)
        };
        let dir = std::fs::canonicalize(&absolute).map_err(|_| {
            fail(
                format!("'{}' is not a directory that exists", given),
                "otc add <folder | shelf name | owner/repo>",
            )
        })?;
        if !dir.is_dir() {
            return Err(fail(
                format!("'{}' is not a directory", given),
                "otc add <folder>",
            ));
        }
        let name = Manifest::load(&dir)
            .ok()
            .map(|m| m.package.name)
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
            .ok_or_else(|| {
                fail(
                    format!("cannot infer a name for '{}'", given),
                    "otc add <folder>",
                )
            })?;
        // `olang doctor` flags it and `use` cannot load it: refuse it now
        if let Some(why) = olang::pkg::path_dependency_problem(&dir, &name) {
            return Err(fail(
                format!(
                    "'{}' is not a package: {}\nPoint at the folder that holds the library's olang.toml or index.ol",
                    given, why
                ),
                "otc new <name> --lib (scaffolds a library)",
            ));
        }
        show_capabilities(&name, &dir);
        let recorded = pathdiff(&dir, &root).unwrap_or_else(|| dir.display().to_string());
        (name, Dependency::Path { path: recorded })
    } else if spec.contains('/') {
        let Spec::GitHub(repo, r) = read_spec(spec, None)? else {
            unreachable!("a folder that exists is read above")
        };
        say!(
            "{spec}: GitHub repository {repo}{}",
            r.as_deref().map(|r| format!(" at {r}")).unwrap_or_default()
        );
        let ask = github::Ask::from_ref(r.as_deref());
        if ctx().dry_run {
            let remote = github::remote(&repo).map_err(|e| {
                fail(
                    e.to_string(),
                    format!("git ls-remote {}", github::repo_url(&repo)),
                )
            })?;
            let (reference, commit, _) = remote
                .resolve(&repo, &ask)
                .map_err(|e| fail(e.to_string(), format!("otc add {repo}@<tag>")))?;
            changes.push(
                "fetch",
                format!("{repo} {reference}"),
                format!("commit {}", &commit[..commit.len().min(12)]),
            );
            changes.push(
                "add",
                repo.rsplit('/').next().unwrap_or(&repo).to_string(),
                "to olang.toml",
            );
            changes.finish(serde_json::json!({}));
            return Ok(());
        }
        let mut shelf = super::shelf::load()?;
        let f = github::fetch(&mut shelf, &repo, &ask, None, false).map_err(|e| {
            let t = e.to_string();
            let next = if t.contains("--accept-moved") {
                format!("otc shelf add {spec} --accept-moved")
            } else if t.contains("refused") {
                "otc new <name> --lib (an olang library has an olang.toml with [package])"
                    .to_string()
            } else {
                format!("git ls-remote {}", github::repo_url(&repo))
            };
            fail(t, next)
        })?;
        shelf
            .save()
            .map_err(|e| fail(e.to_string(), "otc doctor"))?;
        if f.held {
            say!("{} {} is already on the shelf", f.name, f.copy.version);
        } else {
            changes.push(
                "fetch",
                format!("{} {}", f.name, f.copy.version),
                format!(
                    "{repo} @{} into the shelf",
                    f.copy.short_commit().unwrap_or_default()
                ),
            );
        }
        show_capabilities(&f.name, &f.copy.path());
        let reference = f.copy.reference.clone().unwrap_or_default();
        let is_commit = f
            .copy
            .commit
            .as_deref()
            .is_some_and(|c| c.starts_with(&reference));
        let dep = match (&r, github::tag_version(&reference)) {
            _ if is_commit => Dependency::GitHub {
                github: repo.clone(),
                version: None,
                tag: None,
                branch: None,
                rev: f.copy.commit.clone(),
            },
            (_, Some(_)) => Dependency::GitHub {
                github: repo.clone(),
                version: Some(format!("^{}", f.copy.version)),
                tag: None,
                branch: None,
                rev: None,
            },
            (None, None) | (Some(_), None) => {
                let is_branch = github::remote(&repo)
                    .map(|rm| rm.branches.iter().any(|(b, _)| *b == reference))
                    .unwrap_or(true);
                Dependency::GitHub {
                    github: repo.clone(),
                    version: None,
                    tag: (!is_branch).then(|| reference.clone()),
                    branch: is_branch.then(|| reference.clone()),
                    rev: None,
                }
            }
        };
        (f.name, dep)
    } else if let Some(found) = from_registry(spec)? {
        found
    } else {
        // A bare name, perhaps with a version, is a shelf lookup, then the
        // registry's (above). Failing here with the fix named beats
        // recording a dependency that can never resolve.
        let (lib_name, version) = match spec.split_once('@') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (spec.to_string(), None),
        };
        let shelf = super::shelf::load()?;
        let Some(lib) = shelf.library(&lib_name) else {
            let shelved: Vec<&String> = shelf.libraries.keys().collect();
            let hint = if shelved.is_empty() {
                "your shelf is empty".to_string()
            } else {
                format!(
                    "on your shelf: {}",
                    shelved
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err(fail(
                format!(
                    "'{}' is not on your shelf ({}){}",
                    lib_name,
                    hint,
                    super::registry::not_found_hint(&lib_name)
                ),
                format!("otc shelf add <folder> (or otc add <owner>/{lib_name} from GitHub)"),
            ));
        };
        let gives = match &version {
            Some(v) => lib.select(v, None),
            None => lib.pick().map(|(p, _)| p),
        };
        let Some(gives) = gives else {
            return Err(fail(
                format!(
                    "the shelf holds no {lib_name} that version {} allows",
                    version.as_deref().unwrap_or("")
                ),
                format!("otc shelf list {lib_name}"),
            ));
        };
        say!(
            "{spec}: a library on your shelf (gives {}, {})",
            gives.version().unwrap_or_else(|| "?".to_string()),
            match gives {
                Pick::Local(p) => format!("the local folder {}", p.display()),
                Pick::Copy(c) => format!(
                    "a copy from {}",
                    c.from.clone().unwrap_or_else(|| c.source.clone())
                ),
            }
        );
        show_capabilities(&lib_name, &gives.path());
        (
            lib_name.clone(),
            Dependency::Shelf {
                shelf: lib_name,
                version,
            },
        )
    };

    if let Some(existing) = dependencies.get(&name)
        && !force
    {
        return Err(fail(
            format!(
                "'{}' is already a dependency ({})",
                name,
                describe_dependency(existing)
            ),
            format!("otc add {spec} --force (to change its source)"),
        ));
    }
    changes.push(
        "add",
        name.clone(),
        format!("{} in olang.toml", describe_dependency(&dep)),
    );
    if ctx().dry_run {
        changes.finish(serde_json::json!({}));
        return Ok(());
    }
    let saved = Saved::take(&root);
    if found.is_none() {
        init_manifest(&root)?;
    }
    // edited in place: the manifest's comments and unknown keys stay
    Manifest::write_dependency(&root, &name, &dep)
        .map_err(|e| fail(e.to_string(), "otc doctor"))?;

    // Resolve immediately: `add` means "usable now", not "usable after a
    // second command you may not know about".
    if let Err(e) = install(&root, &options()) {
        // put the manifest back as it was, or take away the one made here
        saved.restore();
        return Err(fail(e.to_string(), "otc list"));
    }
    say!("Added '{}' and updated olang.lock", name);
    lock_never_note(&root);
    if let Some(d) = dep_report(&root, &name) {
        say!("{name} is now {}", d.words());
    }
    changes.finish(deps_json(&root));
    Ok(())
}

/// An explicit otc command wrote the lock of a `lock = "never"` project:
/// say so, since runs, tests and the editor leave that lock alone.
fn lock_never_note(root: &Path) {
    if Manifest::load(root).is_ok_and(|m| m.lock_never()) {
        say!(
            "note: olang.toml sets lock = \"never\" — runs, tests and the editor leave olang.lock alone; this command wrote it because you asked"
        );
    }
}

fn describe_dependency(dep: &Dependency) -> String {
    match dep {
        Dependency::Path { path } => format!("path {}", path),
        Dependency::Git { git, .. } => format!("git {}", git),
        Dependency::Registry(req) => format!("version {}", req),
        Dependency::RegistryExplicit { version, .. } => format!("version {}", version),
        Dependency::Shelf { shelf, version } => match version {
            Some(v) => format!("shelf {shelf} {v}"),
            None => format!("shelf {shelf}"),
        },
        Dependency::GitHub {
            github,
            version,
            tag,
            branch,
            rev,
        } => {
            let what = version
                .as_deref()
                .map(|v| format!(" {v}"))
                .or_else(|| tag.as_deref().map(|t| format!(" tag {t}")))
                .or_else(|| branch.as_deref().map(|b| format!(" branch {b}")))
                .or_else(|| {
                    rev.as_deref()
                        .map(|r| format!(" rev {}", &r[..r.len().min(12)]))
                })
                .unwrap_or_default();
            format!("github {github}{what}")
        }
    }
}

pub fn remove(name: &str) -> anyhow::Result<()> {
    let root = require_root()?;
    let manifest = Manifest::load(&root).map_err(|e| fail(e.to_string(), "otc doctor"))?;
    if !manifest.dependencies.contains_key(name) {
        let have: Vec<&String> = manifest.dependencies.keys().collect();
        return Err(fail(
            format!(
                "'{}' is not a dependency (this project has: {})",
                name,
                if have.is_empty() {
                    "none".to_string()
                } else {
                    have.iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ),
            "otc list",
        ));
    }
    let mut changes = Changes::new("remove");
    changes.push(
        "remove",
        name.to_string(),
        "from olang.toml and olang.lock (the shelf keeps its copies)",
    );
    if ctx().dry_run {
        changes.finish(deps_json(&root));
        return Ok(());
    }
    let saved = Saved::take(&root);
    Manifest::remove_dependency(&root, name).map_err(|e| fail(e.to_string(), "otc doctor"))?;
    // Re-resolve so the lock drops the entry too.
    if let Err(e) = install(&root, &options()) {
        saved.restore();
        return Err(fail(e.to_string(), "otc list"));
    }
    say!("Removed '{}' and updated olang.lock", name);
    lock_never_note(&root);
    changes.finish(deps_json(&root));
    Ok(())
}

/// What one dependency got, and why.
pub(crate) struct DepReport {
    pub name: String,
    /// `path`, `shelf`, `github`, `git`, `registry`.
    pub source: &'static str,
    /// What the manifest says.
    pub spec: String,
    pub version: Option<String>,
    pub dir: Option<PathBuf>,
    pub rev: Option<String>,
    /// The rule that chose it: `locked`, `asked`, `default`, `local`,
    /// `remote` (or `path`, `git`, `registry`), or `unresolved`.
    pub rule: &'static str,
    pub dependencies: Vec<String>,
    /// A problem to say: drift, a missing copy.
    pub problem: Option<String>,
}

impl DepReport {
    pub fn words(&self) -> String {
        let v = self
            .version
            .as_deref()
            .map(|v| format!("{v} "))
            .unwrap_or_default();
        let at = self
            .dir
            .as_ref()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| "(unresolved — run otc install)".to_string());
        let why = match self.rule {
            "locked" => "locked: olang.lock pins it".to_string(),
            "asked" => format!("asked: olang.toml asks for {}", self.spec),
            "default" => "default: this machine's `otc shelf use`".to_string(),
            "local" => "local: the shelf's local folder".to_string(),
            "remote" => "remote: the newest copy the shelf holds".to_string(),
            other => other.to_string(),
        };
        format!("{v}{at} ({why})")
    }
}

/// Each dependency of the project at `root`: what it got and which rule
/// chose it.
pub(crate) fn dep_reports(root: &Path) -> Vec<DepReport> {
    let Ok(manifest) = Manifest::load(root) else {
        return Vec::new();
    };
    let lock = Lockfile::load(root).unwrap_or_default();
    let shelf = Shelf::load().ok();
    let opts = options();
    manifest
        .dependencies
        .iter()
        .map(|(name, dep)| {
            report_one(
                root,
                name,
                dep,
                lock.package.get(name),
                shelf.as_ref(),
                &opts,
            )
        })
        .collect()
}

fn dep_report(root: &Path, name: &str) -> Option<DepReport> {
    dep_reports(root).into_iter().find(|d| d.name == name)
}

fn report_one(
    root: &Path,
    name: &str,
    dep: &Dependency,
    locked: Option<&LockedPackage>,
    shelf: Option<&Shelf>,
    opts: &InstallOptions,
) -> DepReport {
    let deps = locked.map(|p| p.dependencies.clone()).unwrap_or_default();
    let base = |source, spec: String| DepReport {
        name: name.to_string(),
        source,
        spec,
        version: locked.and_then(|p| p.version.clone()),
        dir: None,
        rev: None,
        rule: source,
        dependencies: deps.clone(),
        problem: None,
    };
    if let Some((sname, github, ask)) = shelf_ask(name, dep, opts) {
        let source = if github.is_some() { "github" } else { "shelf" };
        let spec = ask
            .clone()
            .map(|a| a.split_once(' ').map(|x| x.1.to_string()).unwrap_or(a))
            .unwrap_or_else(|| sname.clone());
        let mut r = base(
            source,
            match &github {
                Some(g) => format!("{g} {spec}").trim().to_string(),
                None => spec,
            },
        );
        let lib = shelf.and_then(|s| s.library(&sname));
        // what the rules would give now, and why
        let now: Option<(PathBuf, Option<String>, &'static str)> = lib.and_then(|l| match &ask {
            None => l
                .pick()
                .map(|(p, why)| (p.path(), p.commit().map(str::to_string), why.word())),
            Some(a) => {
                let (kind, value) = a.split_once(' ').unwrap_or((a, ""));
                let p = match kind {
                    "version" => l.select(value, github.as_deref()),
                    "tag" | "branch" => l.by_ref(github.as_deref(), value).map(Pick::Copy),
                    "rev" => l.by_commit(value).map(Pick::Copy),
                    _ => None,
                };
                p.map(|p| (p.path(), p.commit().map(str::to_string), "asked"))
            }
        });
        match locked.map(|p| &p.source) {
            Some(LockedSource::Shelf { rev: Some(rev), .. }) => {
                r.rev = Some(rev.clone());
                r.dir = lib.and_then(|l| l.by_commit(rev)).map(|c| c.path());
                if r.dir.is_none() {
                    r.problem = Some(format!(
                        "the lock pins {} at {}, which the shelf does not hold (`otc install` fetches or re-pins it)",
                        sname,
                        &rev[..rev.len().min(12)]
                    ));
                }
                r.rule = match &now {
                    Some((_, Some(c), why)) if c == rev => why,
                    _ => "locked",
                };
            }
            Some(LockedSource::Shelf { rev: None, .. }) => {
                r.dir = now.as_ref().map(|n| n.0.clone());
                r.rule = now.as_ref().map(|n| n.2).unwrap_or("unresolved");
                if r.version.is_none() {
                    r.version = r.dir.as_deref().and_then(olang::pkg::shelf::folder_version);
                }
            }
            _ => {
                r.rule = "unresolved";
                r.problem = Some("not resolved yet (`otc install` resolves it)".to_string());
            }
        }
        return r;
    }
    match dep {
        Dependency::Path { path } => {
            let mut r = base("path", path.clone());
            let dir = root.join(path);
            r.version = olang::pkg::shelf::folder_version(&dir);
            r.dir = Some(std::path::absolute(&dir).unwrap_or(dir));
            r
        }
        Dependency::Git { git, .. } => {
            let mut r = base("git", git.clone());
            if let Some(LockedSource::Git { rev, .. }) = locked.map(|p| &p.source) {
                r.rev = Some(rev.clone());
            }
            r
        }
        Dependency::Registry(v) => base("registry", v.clone()),
        Dependency::RegistryExplicit { version, .. } => base("registry", version.clone()),
        Dependency::Shelf { .. } | Dependency::GitHub { .. } => unreachable!("answered above"),
    }
}

fn report_json(d: &DepReport) -> serde_json::Value {
    serde_json::json!({
        "name": d.name, "source": d.source, "spec": d.spec, "version": d.version,
        "dir": d.dir.as_ref().map(|p| p.display().to_string()), "rev": d.rev,
        "rule": d.rule, "dependencies": d.dependencies, "problem": d.problem,
    })
}

/// `#{ format: 1, kind: "deps", package, version, root, olang, dependencies }`.
pub(crate) fn deps_json(root: &Path) -> serde_json::Value {
    let manifest = Manifest::load(root).ok();
    serde_json::json!({
        "format": 1,
        "kind": "deps",
        "package": manifest.as_ref().map(|m| m.package.name.clone()),
        "version": manifest.as_ref().map(|m| m.package.version.to_string()),
        "root": std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf()).display().to_string(),
        "olang": olang::pkg::toolchain::lock_pin(root),
        "dependencies": dep_reports(root).iter().map(report_json).collect::<Vec<_>>(),
    })
}

/// `otc list` — the project, each dependency, the copy it got and the
/// rule that chose it, and one level of what they pull in.
pub fn list() -> anyhow::Result<()> {
    let root = require_root()?;
    let manifest = Manifest::load(&root).map_err(|e| fail(e.to_string(), "otc doctor"))?;
    if ctx().json {
        emit(deps_json(&root));
        return Ok(());
    }
    println!("{} {}", manifest.package.name, manifest.package.version);
    let reports = dep_reports(&root);
    if reports.is_empty() {
        println!("  (no dependencies — add one: otc add <folder | shelf name | owner/repo>)");
        return Ok(());
    }
    for (i, d) in reports.iter().enumerate() {
        let last = i + 1 == reports.len();
        let branch = if last { "└──" } else { "├──" };
        println!("{} {} {}", branch, d.name, d.words());
        let child_prefix = if last { "    " } else { "│   " };
        if let Some(p) = &d.problem {
            println!("{child_prefix}! {p}");
        }
        for (j, child) in d.dependencies.iter().enumerate() {
            let cbranch = if j + 1 == d.dependencies.len() {
                "└──"
            } else {
                "├──"
            };
            println!("{}{} {}", child_prefix, cbranch, child);
        }
    }
    Ok(())
}

/// One dependency's place: what it has, the newest its requirement
/// allows, the newest there is.
struct Outdated {
    name: String,
    has: Option<String>,
    allowed: Option<String>,
    newest: Option<String>,
    /// For a folder snapshot: commits in its repository since the pin.
    commits_since: Option<u64>,
    /// The tag (GitHub) or copy that `allowed` is, to fetch.
    allowed_tag: Option<String>,
    newest_tag: Option<String>,
    github: Option<String>,
    note: Option<String>,
}

fn max_version(vs: impl Iterator<Item = String>) -> Option<String> {
    vs.max_by(|a, b| olang::pkg::shelf::version_order(a, b))
}

fn outdated_rows(root: &Path, offline: bool) -> anyhow::Result<Vec<Outdated>> {
    let manifest = Manifest::load(root).map_err(|e| fail(e.to_string(), "otc doctor"))?;
    let lock = Lockfile::load(root).unwrap_or_default();
    let shelf = Shelf::load().ok();
    let opts = options();
    let mut rows = Vec::new();
    for (name, dep) in &manifest.dependencies {
        let Some((sname, github, ask)) = shelf_ask(name, dep, &opts) else {
            continue;
        };
        let req = ask
            .as_deref()
            .and_then(|a| a.strip_prefix("version "))
            .map(str::to_string);
        let pinned_other = ask.as_deref().is_some_and(|a| !a.starts_with("version "));
        let lib = shelf.as_ref().and_then(|s| s.library(&sname));
        let locked = lock.package.get(name);
        let has = locked.and_then(|p| p.version.clone());
        let mut held: Vec<String> = lib
            .map(|l| {
                l.copies
                    .iter()
                    .filter(|c| {
                        github
                            .as_deref()
                            .is_none_or(|g| olang::pkg::shelf::same_origin(c.from.as_deref(), g))
                    })
                    .map(|c| c.version.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut tags: Vec<(String, String)> = Vec::new();
        let mut note = None;
        if let Some(g) = &github
            && !offline
        {
            match github::remote(g) {
                Ok(r) => {
                    for (t, v, _) in r.releases() {
                        tags.push((t, v.to_string()));
                    }
                }
                Err(e) => note = Some(format!("{e}")),
            }
        }
        held.extend(tags.iter().map(|(_, v)| v.clone()));
        let allows = |v: &String| match &req {
            Some(r) => olang::pkg::shelf::version_matches(r, v),
            None => !pinned_other,
        };
        let allowed = max_version(held.iter().filter(|v| allows(v)).cloned());
        let newest = max_version(
            held.iter()
                .filter(|v| semver::Version::parse(v).is_ok_and(|x| x.pre.is_empty()))
                .cloned(),
        )
        .or_else(|| max_version(held.iter().cloned()));
        let tag_of = |v: &Option<String>| {
            v.as_ref()
                .and_then(|v| tags.iter().find(|(_, tv)| tv == v).map(|(t, _)| t.clone()))
        };
        // a folder snapshot: commits since, in its repository
        let mut commits_since = None;
        if let Some(LockedSource::Shelf { rev: Some(rev), .. }) = locked.map(|p| &p.source)
            && let Some(c) = lib.and_then(|l| l.by_commit(rev))
            && c.source == "folder"
            && let Some(from) = &c.from
        {
            commits_since = std::process::Command::new("git")
                .arg("-C")
                .arg(from)
                .args(["rev-list", "--count", &format!("{rev}..HEAD")])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok());
        }
        rows.push(Outdated {
            name: name.clone(),
            has,
            allowed_tag: tag_of(&allowed),
            newest_tag: tag_of(&newest),
            allowed,
            newest,
            commits_since,
            github,
            note,
        });
    }
    Ok(rows)
}

fn behind(has: &Option<String>, other: &Option<String>) -> bool {
    matches!((has, other), (Some(h), Some(o)) if olang::pkg::shelf::version_order(o, h) == std::cmp::Ordering::Greater)
}

/// `otc outdated [--offline]`.
pub fn outdated(offline: bool) -> anyhow::Result<()> {
    let root = require_root()?;
    let rows = outdated_rows(&root, offline)?;
    if ctx().json {
        emit(serde_json::json!({
            "format": 1, "kind": "outdated", "scope": "project",
            "root": root.display().to_string(),
            "dependencies": rows.iter().map(|r| serde_json::json!({
                "name": r.name, "has": r.has, "allowed": r.allowed, "newest": r.newest,
                "commits_since": r.commits_since, "github": r.github,
                "behind": behind(&r.has, &r.allowed), "major_behind": behind(&r.allowed, &r.newest),
                "note": r.note,
            })).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    if rows.is_empty() {
        println!("no dependency here comes from the shelf or GitHub, so none can be behind");
        return Ok(());
    }
    println!(
        "{:<16} {:<10} {:<10} {:<10}",
        "dependency", "has", "allowed", "newest"
    );
    let mut any = false;
    let mut major = false;
    for r in &rows {
        let d = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".to_string());
        let mut mark = String::new();
        if behind(&r.has, &r.allowed) {
            mark.push_str("  ← upgrade");
            any = true;
        } else if behind(&r.allowed, &r.newest) {
            mark.push_str("  ← newer past the requirement");
            major = true;
        }
        if let Some(n) = r.commits_since
            && n > 0
        {
            mark.push_str(&format!(
                "  ({n} commit{} since the pin)",
                if n == 1 { "" } else { "s" }
            ));
        }
        println!(
            "{:<16} {:<10} {:<10} {:<10}{}",
            r.name,
            d(&r.has),
            d(&r.allowed),
            d(&r.newest),
            mark
        );
        if let Some(n) = &r.note {
            println!("  ({n})");
        }
    }
    if any {
        println!("next: otc upgrade --dry-run (shows the change), then otc upgrade");
    } else if major {
        println!("next: otc upgrade --major --dry-run (moves past the requirement)");
    }
    Ok(())
}

/// One planned upgrade: name, from, to, the tag, the GitHub repository.
type Planned = (
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);

/// `otc upgrade [name…] [--major] [--yes]`: move the lock (and with
/// --major the manifest) to the newest allowed, saying each change before
/// writing; a new capability asked needs --yes; a failure leaves both as
/// they were.
pub fn upgrade(names: &[String], major: bool, yes: bool) -> anyhow::Result<()> {
    let root = require_root()?;
    let manifest = Manifest::load(&root).map_err(|e| fail(e.to_string(), "otc doctor"))?;
    for n in names {
        if !manifest.dependencies.contains_key(n) {
            return Err(fail(
                format!("'{n}' is not a dependency of this project"),
                "otc list",
            ));
        }
    }
    let rows = outdated_rows(&root, false)?;
    let before = dep_reports(&root);
    let mut changes = Changes::new("upgrade");
    // (name, from, to, tag, github repository)
    let mut plan: Vec<Planned> = Vec::new();
    for r in rows
        .iter()
        .filter(|r| names.is_empty() || names.contains(&r.name))
    {
        let (to, tag) = if major {
            (&r.newest, &r.newest_tag)
        } else {
            (&r.allowed, &r.allowed_tag)
        };
        if !behind(&r.has, to) {
            continue;
        }
        let to = to.clone().expect("behind");
        plan.push((
            r.name.clone(),
            r.has.clone(),
            to,
            tag.clone(),
            r.github.clone(),
        ));
    }
    if plan.is_empty() {
        say!(
            "every dependency is at the newest {} allows",
            if major { "there is" } else { "its requirement" }
        );
        if !major && rows.iter().any(|r| behind(&r.allowed, &r.newest)) {
            say!("next: otc upgrade --major (newer versions are past the requirement)");
        }
        changes.finish(deps_json(&root));
        return Ok(());
    }
    let mut shelf = super::shelf::load()?;
    let mut new_caps = Vec::new();
    let mut planned: Vec<(String, String)> = Vec::new();
    for (name, from, to, tag, github) in &plan {
        // the new copy: fetched (to the git cache only, on a dry run)
        let dir: Option<PathBuf> = match (github, tag) {
            (Some(g), Some(t)) if ctx().dry_run => olang::pkg::cache::fetch_git(
                &github::repo_url(g),
                &olang::pkg::cache::GitRef::Tag(t.clone()),
                false,
            )
            .ok()
            .map(|f| f.dir),
            (Some(g), Some(t)) => {
                let f = github::fetch(
                    &mut shelf,
                    g,
                    &github::Ask::Tag(t.clone()),
                    Some(name),
                    false,
                )
                .map_err(|e| fail(e.to_string(), format!("otc shelf add {g}@{t}")))?;
                Some(f.copy.path())
            }
            _ => shelf
                .library(name)
                .and_then(|l| l.select(&format!("={to}"), None))
                .map(|p| p.path()),
        };
        let old_dir = before
            .iter()
            .find(|d| &d.name == name)
            .and_then(|d| d.dir.clone());
        let old_caps = old_dir.as_deref().map(capability_lines).unwrap_or_default();
        let caps = dir.as_deref().map(capability_lines).unwrap_or_default();
        let added: Vec<String> = caps
            .iter()
            .filter(|c| !old_caps.contains(c))
            .cloned()
            .collect();
        let mut detail = format!("{} → {to}", from.as_deref().unwrap_or("-"));
        if let Some(t) = tag {
            detail.push_str(&format!(" ({t})"));
        }
        if let Some(note) = dir
            .as_deref()
            .and_then(|d| Manifest::load(d).ok().and_then(|m| m.package.description))
        {
            detail.push_str(&format!(": {note}"));
        }
        if ctx().dry_run {
            changes.push("upgrade", name.clone(), detail);
        } else {
            say!("{name}: {detail}");
            planned.push((name.clone(), detail));
        }
        for a in added.iter().filter(|a| !a.starts_with("declares no")) {
            say!("  {name} {to} asks for a new capability: {a}");
            new_caps.push(format!("{name}: {a}"));
        }
    }
    if !new_caps.is_empty() && !yes && !ctx().dry_run {
        return Err(fail(
            format!(
                "the upgrade asks for new capabilities ({}); nothing was written",
                new_caps.join("; ")
            ),
            "otc upgrade --yes (after reading them)",
        ));
    }
    if ctx().dry_run {
        changes.finish(deps_json(&root));
        return Ok(());
    }
    shelf
        .save()
        .map_err(|e| fail(e.to_string(), "otc doctor"))?;
    let saved = Saved::take(&root);
    let result = (|| -> anyhow::Result<()> {
        if major {
            for (name, _, to, _, _) in &plan {
                let mut dep = manifest.dependencies.get(name).cloned().expect("planned");
                match &mut dep {
                    Dependency::Shelf { version, .. } | Dependency::GitHub { version, .. } => {
                        *version = Some(format!("^{to}"))
                    }
                    Dependency::Registry(v) => *v = format!("^{to}"),
                    _ => {}
                }
                Manifest::write_dependency(&root, name, &dep)
                    .map_err(|e| fail(e.to_string(), "otc doctor"))?;
            }
        }
        let mut opts = options();
        opts.upgrade = plan.iter().map(|p| p.0.clone()).collect();
        install(&root, &opts).map_err(|e| fail(e.to_string(), "otc list"))?;
        Ok(())
    })();
    if let Err(e) = result {
        saved.restore();
        return Err(fail(
            format!("{e}; olang.toml and olang.lock are as they were"),
            "otc outdated",
        ));
    }
    for (name, detail) in planned {
        changes.push("upgrade", name, detail);
    }
    for d in dep_reports(&root)
        .iter()
        .filter(|d| plan.iter().any(|p| p.0 == d.name))
    {
        say!("{} is now {}", d.name, d.words());
    }
    changes.finish(deps_json(&root));
    Ok(())
}

/// `otc install` — make the lockfile true. `--update` re-resolves
/// everything to the newest satisfying sources; `--frozen` fails instead
/// of changing the lock (CI's reproducibility gate).
pub fn do_install(frozen: bool, update: bool, verbose: bool) -> anyhow::Result<()> {
    let root = require_root()?;
    let lock_before = std::fs::read_to_string(root.join("olang.lock")).ok();
    let opts = InstallOptions {
        frozen,
        registry: registry_from_env(),
        refresh: update,
        repin_shelf: true,
        explicit: true,
        offline: false,
        upgrade: Vec::new(),
    };
    if ctx().dry_run {
        say!("would resolve every dependency and write olang.lock (dry run: nothing was changed)");
        emit(
            serde_json::json!({ "format": 1, "kind": "change", "command": "install", "dry_run": true, "changes": [], "now": {} }),
        );
        return Ok(());
    }
    // A shelf library re-shelved since the lock was written (another
    // commit, or pinned/unpinned) is re-pinned to what the shelf holds now:
    // say so, as a moved path is announced, so the lock change is expected.
    // Under --frozen the install below refuses the drift instead.
    if !frozen {
        let short = |rev: &Option<String>| match rev {
            Some(r) => format!("at {}", &r[..r.len().min(12)]),
            None => "following its directory".to_string(),
        };
        // An edited path or shelf library: its new contents are pinned.
        for name in olang::pkg::edited_libraries(&root) {
            say!(
                "library '{}' changed since the lock was written — re-pinning",
                name
            );
        }
        for (name, was, now) in olang::pkg::moved_shelf_pins(&root) {
            say!(
                "shelf library '{}' moved: the lock had it {}, the shelf has it {} — re-pinning",
                name,
                short(&was),
                short(&now)
            );
        }
    }
    // A lock written on another machine can name paths this one does not
    // have; say so before re-resolving, so the change to the lock is not
    // a surprise.
    if let Ok(lock) = Lockfile::load(&root) {
        for (name, locked) in &lock.package {
            if let LockedSource::Path { path } = &locked.source {
                let dir = if std::path::Path::new(path).is_absolute() {
                    std::path::PathBuf::from(path)
                } else {
                    root.join(path)
                };
                if !dir.exists() {
                    say!(
                        "the lock names a path that is not here ({} -> {}) — re-resolving",
                        name,
                        path
                    );
                }
            }
        }
    }
    let map = install(&root, &opts)
        .map_err(|e| fail(e.to_string(), "otc list (what each dependency resolves to)"))?;
    // a pin nothing installed satisfies: said, with the command (never run)
    if let Some(pin) = olang::pkg::toolchain::manifest_pin(&root)
        && olang::pkg::toolchain::lock_pin(&root)
            .is_none_or(|v| !olang::pkg::toolchain::satisfies(&pin, &v))
    {
        say!(
            "olang.toml pins olang {}, and no installed olang satisfies it — `{}` installs one",
            pin,
            olang::pkg::toolchain::install_hint(&pin)
        );
    }
    // A starter library shelved by an older toolchain lags the copy this
    // one ships (the shelf is seeded once): say so, with the command that
    // refreshes it, so a release note about `validate` reaches the shelf.
    for (name, dir) in &map {
        if olang::pkg::starter::get(name).is_none() {
            continue;
        }
        let shelved = std::fs::read_to_string(dir.join("olang.toml"))
            .ok()
            .and_then(|t| {
                t.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("version = ")
                        .map(|v| v.trim_matches('"').to_string())
                })
            });
        if let Some(v) = shelved
            && v != olang::VERSION
        {
            say!(
                "shelf library '{}' is {} — this olang ships {}: `otc shelf restore {}` refreshes it",
                name,
                v,
                olang::VERSION,
                name
            );
        }
    }
    say!(
        "Resolved {} dependenc{}",
        map.len(),
        if map.len() == 1 { "y" } else { "ies" }
    );
    if verbose {
        for (name, dir) in &map {
            say!("  {} -> {}", name, dir.display());
        }
    }
    let lock_after = std::fs::read_to_string(root.join("olang.lock")).ok();
    if lock_before == lock_after {
        say!("olang.lock unchanged");
    } else {
        say!("Wrote olang.lock");
        lock_never_note(&root);
    }
    emit(serde_json::json!({
        "format": 1, "kind": "change", "command": "install", "dry_run": false,
        "changes": if lock_before == lock_after { vec![] } else {
            vec![serde_json::json!({ "action": "write", "what": "olang.lock", "detail": "" })]
        },
        "now": deps_json(&root),
    }));
    Ok(())
}

/// `to` expressed relative to `from` when the walk is simple (shared
/// prefix, then up-and-over); None falls back to the absolute path.
fn pathdiff(to: &Path, from: &Path) -> Option<String> {
    let to: Vec<_> = to.components().collect();
    let from: Vec<_> = from.components().collect();
    let common = to
        .iter()
        .zip(from.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common == 0 {
        return None;
    }
    let mut parts: Vec<String> =
        std::iter::repeat_n("..".to_string(), from.len() - common).collect();
    for c in &to[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        return Some(".".to_string());
    }
    Some(parts.join("/"))
}

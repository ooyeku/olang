//! The shelf: `otc shelf list | info | add | use | outdated | upgrade |
//! remove | prune | restore` (OL-581, OL-583, OL-584).
//!
//! The shelf holds every library this machine has, by name, as many
//! versions of each as needed (`olang::pkg::shelf`). These commands are
//! its whole interface: what is there, where each copy came from, what
//! uses it, and what can go. Nothing here runs a library's code.

use super::{Changes, ctx, emit, fail};
use crate::say;
use anyhow::Result;
use clap::Subcommand;
use olang::pkg::github;
use olang::pkg::shelf::{Library, Pick, Shelf, ShelfCopy};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum ShelfCommand {
    /// The libraries held: each version, where it came from, its size and
    /// what uses it.
    ///
    /// Example: otc shelf list geometry
    List {
        /// One library, with every project that uses each version
        name: Option<String>,
    },
    /// One library in full: versions, users, the capabilities it asks
    /// for, its docs' first line.
    ///
    /// Example: otc shelf info geometry
    Info { name: String },
    /// Put a library on the shelf without adding it to a project: a
    /// folder (referenced), a folder at a commit, or owner/repo[@ref].
    ///
    /// Example: otc shelf add ooyeku/geometry@v1.2.0
    Add {
        /// A folder, or a GitHub repository (owner/repo, @tag, @branch, @commit)
        spec: String,
        /// Hold it under this name (default: the library's package name)
        #[arg(long)]
        name: Option<String>,
        /// A folder at this commit of its repository (a copy the shelf owns)
        #[arg(long, value_name = "COMMIT")]
        rev: Option<String>,
        /// Accept a tag that now names another commit than the copy held
        #[arg(long)]
        accept_moved: bool,
    },
    /// Set the machine's default for a bare name: name@version, or
    /// name@local; a bare name goes back to the rule (local, else newest).
    ///
    /// Example: otc shelf use geometry@1.2.0
    #[command(alias = "default")]
    Use { spec: String },
    /// For each GitHub library, the newest held and the newest released;
    /// for a folder snapshot, the commits since (network).
    ///
    /// Example: otc shelf outdated
    Outdated { names: Vec<String> },
    /// Fetch the newest release of GitHub libraries; older copies stay
    /// until pruned (network).
    ///
    /// Example: otc shelf upgrade geometry
    Upgrade {
        names: Vec<String>,
        #[arg(long)]
        accept_moved: bool,
    },
    /// Take a library (name) or one version (name@version, name@local)
    /// off the shelf; says what uses it first. A local folder is only
    /// forgotten, never deleted.
    ///
    /// Example: otc shelf remove geometry@1.1.0
    Remove {
        spec: String,
        /// Remove it although known projects use it
        #[arg(long)]
        yes: bool,
    },
    /// Remove the copies nothing uses (says what and how much first;
    /// --yes removes).
    ///
    /// Example: otc shelf prune --yes
    Prune {
        #[arg(long)]
        yes: bool,
    },
    /// Put back a starter library (or all of them), or refresh it to this
    /// olang's copy.
    ///
    /// Example: otc shelf restore textkit
    Restore { name: Option<String> },
}

impl ShelfCommand {
    pub fn execute(self) -> Result<()> {
        match self {
            ShelfCommand::List { name } => list(name.as_deref()),
            ShelfCommand::Info { name } => info(&name),
            ShelfCommand::Add {
                spec,
                name,
                rev,
                accept_moved,
            } => add(&spec, name.as_deref(), rev.as_deref(), accept_moved).map(|_| ()),
            ShelfCommand::Use { spec } => use_default(&spec),
            ShelfCommand::Outdated { names } => outdated(&names),
            ShelfCommand::Upgrade {
                names,
                accept_moved,
            } => upgrade(&names, accept_moved),
            ShelfCommand::Remove { spec, yes } => remove(&spec, yes),
            ShelfCommand::Prune { yes } => prune(yes, true).map(|_| ()),
            ShelfCommand::Restore { name } => restore(name.as_deref()),
        }
    }
}

pub(crate) fn load() -> Result<Shelf> {
    Shelf::load_or_seed().map_err(|e| {
        let text = e.to_string();
        if text.contains("otc doctor --fix") {
            fail(text, "otc doctor --fix")
        } else {
            fail(text, "otc doctor")
        }
    })
}

fn save(shelf: &Shelf) -> Result<()> {
    shelf.save().map_err(|e| fail(e.to_string(), "otc doctor"))
}

/// Who uses each folder the shelf gives: a copy's or a local folder's
/// path, to the projects (and held libraries) whose lock names it.
pub(crate) fn usage(shelf: &Shelf) -> BTreeMap<PathBuf, Vec<String>> {
    use olang::pkg::lock::{LockedSource, Lockfile};
    let mut users: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    let mut note = |dir: PathBuf, who: String| {
        let list = users.entry(dir).or_default();
        if !list.contains(&who) {
            list.push(who);
        }
    };
    let mut scan = |root: &Path, who: String| {
        let Ok(lock) = Lockfile::load(root) else {
            return;
        };
        for locked in lock.package.values() {
            let LockedSource::Shelf { shelf: n, rev, .. } = &locked.source else {
                continue;
            };
            let dir = match rev {
                Some(r) => shelf
                    .library(n)
                    .and_then(|l| l.by_commit(r))
                    .map(ShelfCopy::path),
                None => shelf.resolve(n),
            };
            if let Some(d) = dir {
                note(d, who.clone());
            }
        }
    };
    for root in olang::pkg::projects::known() {
        scan(&root, root.display().to_string());
    }
    // a held library's own lock pins what it depends on
    for (name, lib) in &shelf.libraries {
        for c in &lib.copies {
            scan(&c.path(), format!("library {name} {}", c.version));
        }
        if let Some(l) = &lib.local {
            scan(l, format!("library {name} (local)"));
        }
    }
    users
}

fn source_words(c: &ShelfCopy) -> String {
    let mut w = c.source.clone();
    if let Some(f) = &c.from {
        w.push(' ');
        w.push_str(f);
    }
    if let Some(r) = &c.reference
        && c.commit
            .as_deref()
            .is_none_or(|h| !h.starts_with(r.as_str()))
    {
        w.push(' ');
        w.push_str(r);
    }
    if let Some(h) = c.short_commit() {
        w.push_str(&format!(" @{}", &h[..h.len().min(8)]));
    }
    w
}

fn local_status(dir: &Path, name: &str) -> Option<String> {
    if !dir.exists() {
        Some(format!(
            "missing — the folder is gone: `otc shelf add <its new place>` re-registers it, \
             `otc shelf remove {name}@local` forgets it{}",
            if olang::pkg::starter::get(name).is_some() {
                format!(", `otc shelf restore {name}` brings the starter back")
            } else {
                String::new()
            }
        ))
    } else if olang::pkg::shelf::check_library(dir).is_err() {
        Some(
            "no longer looks like a library (no olang.toml with [package], no index.ol)"
                .to_string(),
        )
    } else {
        None
    }
}

fn library_json(
    name: &str,
    lib: &Library,
    users: &BTreeMap<PathBuf, Vec<String>>,
) -> serde_json::Value {
    let gives = lib.pick().map(|(p, why)| {
        serde_json::json!({
            "version": p.version(), "dir": p.path().display().to_string(),
            "why": why.word(), "commit": p.commit(),
        })
    });
    let copies: Vec<serde_json::Value> = lib
        .copies
        .iter()
        .map(|c| {
            serde_json::json!({
                "version": c.version, "source": c.source, "from": c.from, "ref": c.reference,
                "commit": c.commit, "checksum": c.checksum, "dir": c.path().display().to_string(),
                "fetched": c.fetched, "size": super::doctor::dir_size(&c.path()),
                "present": c.path().is_dir(),
                "users": users.get(&c.path()).cloned().unwrap_or_default(),
            })
        })
        .collect();
    serde_json::json!({
        "name": name,
        "gives": gives,
        "default": lib.default,
        "local": lib.local.as_ref().map(|l| serde_json::json!({
            "dir": l.display().to_string(),
            "version": olang::pkg::shelf::folder_version(l),
            "status": local_status(l, name).map(|_| "missing").unwrap_or("ok"),
            "problem": local_status(l, name),
            "users": users.get(l).cloned().unwrap_or_default(),
        })),
        "copies": copies,
    })
}

fn users_words(n: usize) -> String {
    match n {
        0 => "unused".to_string(),
        1 => "1 user".to_string(),
        n => format!("{n} users"),
    }
}

/// `otc shelf list [name]`.
fn list(only: Option<&str>) -> Result<()> {
    let shelf = load()?;
    let users = usage(&shelf);
    if let Some(n) = only
        && shelf.library(n).is_none()
    {
        return Err(fail(
            format!("'{n}' is not on your shelf"),
            "otc shelf list",
        ));
    }
    let libs: Vec<(&String, &Library)> = shelf
        .libraries
        .iter()
        .filter(|(n, _)| only.is_none_or(|o| o == n.as_str()))
        .collect();
    if ctx().json {
        emit(serde_json::json!({
            "format": 1,
            "kind": "shelf",
            "shelf": std::path::absolute(Shelf::file()).unwrap_or_else(|_| Shelf::file()).display().to_string(),
            "home": Shelf::home().display().to_string(),
            "size": super::doctor::dir_size(&Shelf::home()),
            "libraries": libs.iter().map(|(n, l)| library_json(n, l, &users)).collect::<Vec<_>>(),
        }));
        return Ok(());
    }
    if libs.is_empty() {
        println!("Your shelf is empty.");
        println!("next: otc shelf add <folder> (or otc shelf add <owner/repo>)");
        return Ok(());
    }
    for (name, lib) in &libs {
        let gives = match lib.pick() {
            Some((Pick::Local(_), why)) => format!("gives the local folder ({})", why.word()),
            Some((Pick::Copy(c), why)) => format!("gives {} ({})", c.version, why.word()),
            None => "gives nothing".to_string(),
        };
        println!("{name}  {gives}");
        if let Some(l) = &lib.local {
            let n = users.get(l).map_or(0, Vec::len);
            let v = olang::pkg::shelf::folder_version(l).unwrap_or_else(|| "-".to_string());
            println!(
                "  local  {v:<10} {}  (referenced, not copied) · {}",
                l.display(),
                users_words(n)
            );
            if let Some(p) = local_status(l, name) {
                println!("         ({p})");
            }
            if only.is_some() {
                for u in users.get(l).into_iter().flatten() {
                    println!("         used by {u}");
                }
            }
        }
        for c in &lib.copies {
            let n = users.get(&c.path()).map_or(0, Vec::len);
            let present = if c.path().is_dir() {
                ""
            } else {
                " (its folder is gone: `otc shelf remove` it)"
            };
            println!(
                "  copy   {:<10} {}  {} · {}{}",
                c.version,
                source_words(c),
                super::doctor::human(super::doctor::dir_size(&c.path())),
                users_words(n),
                present
            );
            if only.is_some() {
                println!("         {}", c.path().display());
                for u in users.get(&c.path()).into_iter().flatten() {
                    println!("         used by {u}");
                }
            }
        }
    }
    println!(
        "{} on the shelf ({})",
        super::doctor::human(super::doctor::dir_size(&Shelf::home())),
        Shelf::home().display()
    );
    Ok(())
}

/// A library's `[capabilities]`, as lines (`fs = "read"`), or one line
/// saying it declares none.
pub(crate) fn capability_lines(dir: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join("olang.toml")).unwrap_or_default();
    let caps = toml::from_str::<toml::Table>(&text)
        .ok()
        .and_then(|t| t.get("capabilities").cloned());
    match caps {
        Some(toml::Value::Table(t)) if !t.is_empty() => t
            .iter()
            .filter(|(k, _)| k.as_str() != "dependencies")
            .map(|(k, v)| format!("{k} = {v}"))
            .collect(),
        _ => vec![
            "declares no [capabilities]: it may use what the project allows \
             (limit it with [capabilities.dependencies.<name>] in the project's olang.toml)"
                .to_string(),
        ],
    }
}

/// The first line of a library's docs: its description, else the first
/// `///` line of its root module.
fn doc_line(dir: &Path) -> Option<String> {
    if let Ok(m) = olang::pkg::manifest::Manifest::load(dir)
        && let Some(d) = m.package.description
    {
        return Some(d);
    }
    let text = std::fs::read_to_string(dir.join("index.ol")).ok()?;
    text.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("//!").or_else(|| l.strip_prefix("///")))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

fn info(name: &str) -> Result<()> {
    let shelf = load()?;
    let Some(lib) = shelf.library(name) else {
        return Err(fail(
            format!("'{name}' is not on your shelf"),
            "otc shelf list",
        ));
    };
    let users = usage(&shelf);
    let gives = lib.pick().map(|(p, _)| p.path());
    let caps = gives.as_deref().map(capability_lines).unwrap_or_default();
    let doc = gives.as_deref().and_then(doc_line);
    if ctx().json {
        let mut v = library_json(name, lib, &users);
        v["format"] = 1.into();
        v["kind"] = "library".into();
        v["capabilities"] = serde_json::json!(caps);
        v["doc"] = serde_json::json!(doc);
        emit(v);
        return Ok(());
    }
    println!(
        "{name}{}",
        doc.map(|d| format!(" — {d}")).unwrap_or_default()
    );
    match lib.pick() {
        Some((p, why)) => println!(
            "a project that asks for no version gets {} ({}: {})",
            p.version().unwrap_or_else(|| "?".to_string()),
            why.word(),
            p.path().display()
        ),
        None => println!("holds nothing"),
    }
    if let Some(d) = &lib.default {
        println!("this machine's default: {d} (`otc shelf use {name}` goes back to the rule)");
    }
    println!("versions:");
    if let Some(l) = &lib.local {
        println!(
            "  local {}  {}",
            olang::pkg::shelf::folder_version(l).unwrap_or_else(|| "-".to_string()),
            l.display()
        );
        for u in users.get(l).into_iter().flatten() {
            println!("      used by {u}");
        }
    }
    for c in &lib.copies {
        println!("  {}  {}", c.version, source_words(c));
        for u in users.get(&c.path()).into_iter().flatten() {
            println!("      used by {u}");
        }
    }
    println!("capabilities it asks for:");
    for c in caps {
        println!("  {c}");
    }
    Ok(())
}

/// What `otc add` and `otc shelf add` understood a spec as.
pub(crate) enum Spec {
    Folder(PathBuf),
    GitHub(String, Option<String>),
}

/// A folder when one exists there (or `--rev` asks for one), else
/// `owner/repo[@ref]`.
pub(crate) fn read_spec(spec: &str, rev: Option<&str>) -> Result<Spec> {
    let p = Path::new(spec);
    if p.exists() || rev.is_some() || spec.starts_with('.') || p.is_absolute() {
        return Ok(Spec::Folder(p.to_path_buf()));
    }
    if let Some((repo, r)) = github::parse_spec(spec) {
        return Ok(Spec::GitHub(repo, r));
    }
    Err(fail(
        format!("'{spec}' is neither a folder here nor a GitHub repository (owner/repo)"),
        "otc shelf add <folder> (or otc shelf add <owner/repo>)",
    ))
}

/// Say a library's capabilities before it is added.
pub(crate) fn show_capabilities(name: &str, dir: &Path) {
    say!("{name} asks for:");
    for line in capability_lines(dir) {
        say!("  {line}");
    }
}

/// `otc shelf add`: returns the name held and the copy (none for a
/// local folder).
pub(crate) fn add(
    spec: &str,
    name: Option<&str>,
    rev: Option<&str>,
    accept_moved: bool,
) -> Result<(String, Option<ShelfCopy>)> {
    let mut shelf = load()?;
    let mut changes = Changes::new("shelf add");
    let result = match read_spec(spec, rev)? {
        Spec::Folder(dir) => {
            say!("{spec}: a folder");
            if ctx().dry_run {
                olang::pkg::shelf::check_library(&dir)
                    .map_err(|e| fail(format!("'{spec}': {e}"), "otc new <name> --lib"))?;
                changes.push("add", format!("{}", dir.display()), "to the shelf");
                (name.unwrap_or(spec).to_string(), None)
            } else {
                match rev {
                    Some(rev) => {
                        let (n, sha) = shelf
                            .add_at_rev(&dir, name, rev)
                            .map_err(|e| fail(e.to_string(), "otc shelf list"))?;
                        let copy = shelf.library(&n).and_then(|l| l.by_commit(&sha)).cloned();
                        show_capabilities(&n, &copy.as_ref().map(ShelfCopy::path).unwrap_or(dir));
                        changes.push(
                            "add",
                            format!("{n} at {}", &sha[..sha.len().min(12)]),
                            "a copy the shelf owns; the name gives it now",
                        );
                        (n, copy)
                    }
                    None => {
                        let n = shelf
                            .add(&dir, name)
                            .map_err(|e| fail(e.to_string(), "otc new <name> --lib"))?;
                        show_capabilities(&n, &dir);
                        changes.push(
                            "add",
                            n.clone(),
                            format!(
                                "the local folder {} (referenced, not copied)",
                                dir.display()
                            ),
                        );
                        (n, None)
                    }
                }
            }
        }
        Spec::GitHub(repo, r) => {
            say!(
                "{spec}: GitHub repository {repo}{}",
                r.as_deref().map(|r| format!(" at {r}")).unwrap_or_default()
            );
            let ask = github::Ask::from_ref(r.as_deref());
            if ctx().dry_run {
                let remote =
                    github::remote(&repo).map_err(|e| fail(e.to_string(), "otc status"))?;
                let (reference, commit, _) = remote
                    .resolve(&repo, &ask)
                    .map_err(|e| fail(e.to_string(), format!("otc shelf add {repo}@<tag>")))?;
                changes.push(
                    "fetch",
                    format!("{repo} {reference}"),
                    format!("commit {}", &commit[..commit.len().min(12)]),
                );
                (name.unwrap_or(&repo).to_string(), None)
            } else {
                let f = github::fetch(&mut shelf, &repo, &ask, name, accept_moved).map_err(|e| {
                    let t = e.to_string();
                    let next = if t.contains("--accept-moved") {
                        format!("otc shelf add {spec} --accept-moved")
                    } else if t.contains("refused") {
                        "otc new <name> --lib (an olang library has an olang.toml with [package])".to_string()
                    } else {
                        format!("git ls-remote {}", github::repo_url(&repo))
                    };
                    fail(t, next)
                })?;
                show_capabilities(&f.name, &f.copy.path());
                if f.held {
                    say!("{} {} is already on the shelf", f.name, f.copy.version);
                } else {
                    changes.push(
                        "fetch",
                        format!("{} {}", f.name, f.copy.version),
                        format!(
                            "{repo} {} @{}, checksum {}",
                            f.copy.reference.clone().unwrap_or_default(),
                            f.copy.short_commit().unwrap_or_default(),
                            f.copy.checksum.as_deref().map(|c| &c[..12]).unwrap_or("-")
                        ),
                    );
                }
                (f.name, Some(f.copy))
            }
        }
    };
    if !ctx().dry_run {
        save(&shelf)?;
        say!("use it from any project:  otc add {}", result.0);
    }
    let gives = shelf
        .pick(&result.0)
        .map(|(p, why)| serde_json::json!({ "version": p.version(), "why": why.word(), "dir": p.path().display().to_string() }));
    changes.finish(serde_json::json!({ "name": result.0, "gives": gives }));
    Ok(result)
}

fn use_default(spec: &str) -> Result<()> {
    let mut shelf = load()?;
    let (name, label) = match spec.split_once('@') {
        Some((n, l)) => (n.to_string(), Some(l.to_string())),
        None => (spec.to_string(), None),
    };
    let Some(lib) = shelf.libraries.get_mut(&name) else {
        return Err(fail(
            format!("'{name}' is not on your shelf"),
            "otc shelf list",
        ));
    };
    match &label {
        Some(l) if l == "local" && lib.local.is_none() => {
            return Err(fail(
                format!("'{name}' has no local folder on the shelf"),
                format!("otc shelf add <folder> --name {name}"),
            ));
        }
        Some(l) if l != "local" && !lib.copies.iter().any(|c| c.answers(l)) => {
            return Err(fail(
                format!("'{name}' holds no version {l}"),
                format!("otc shelf list {name}"),
            ));
        }
        _ => {}
    }
    let mut changes = Changes::new("shelf use");
    match &label {
        Some(l) => changes.push(
            "default",
            name.clone(),
            format!("{l} (a bare `{name}` gives it on this machine)"),
        ),
        None => changes.push(
            "default",
            name.clone(),
            "the rule again: the local folder, else the newest copy",
        ),
    }
    if !ctx().dry_run {
        lib.default = label.clone();
        save(&shelf)?;
    }
    let gives = shelf
        .pick(&name)
        .map(|(p, why)| serde_json::json!({ "version": p.version(), "why": why.word(), "dir": p.path().display().to_string() }));
    if let Some(g) = &gives {
        say!(
            "a bare `{name}` now gives {} ({})",
            g["version"].as_str().unwrap_or("?"),
            g["why"].as_str().unwrap_or("")
        );
    }
    changes.finish(serde_json::json!({ "name": name, "gives": gives }));
    Ok(())
}

/// The GitHub repositories the shelf holds copies of, by library.
fn github_libraries<'a>(
    shelf: &'a Shelf,
    names: &[String],
) -> Vec<(&'a String, String, Vec<&'a ShelfCopy>)> {
    let mut out = Vec::new();
    for (name, lib) in &shelf.libraries {
        if !names.is_empty() && !names.contains(name) {
            continue;
        }
        let mut repos: Vec<String> = Vec::new();
        for c in &lib.copies {
            if c.source == "github"
                && let Some(f) = &c.from
                && !repos.iter().any(|r| r.eq_ignore_ascii_case(f))
            {
                repos.push(f.clone());
            }
        }
        for repo in repos {
            let copies = lib
                .copies
                .iter()
                .filter(|c| olang::pkg::shelf::same_origin(c.from.as_deref(), &repo))
                .collect();
            out.push((name, repo, copies));
        }
    }
    out
}

fn outdated(names: &[String]) -> Result<()> {
    let shelf = load()?;
    for n in names {
        if shelf.library(n).is_none() {
            return Err(fail(
                format!("'{n}' is not on your shelf"),
                "otc shelf list",
            ));
        }
    }
    let mut rows = Vec::new();
    for (name, repo, copies) in github_libraries(&shelf, names) {
        let newest_held = copies
            .iter()
            .map(|c| c.version.clone())
            .max_by(|a, b| olang::pkg::shelf::version_order(a, b));
        let remote = github::remote(&repo);
        let (newest, error) = match &remote {
            Ok(r) => (r.newest_release().map(|(_, v, _)| v.to_string()), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let behind = matches!((&newest_held, &newest), (Some(h), Some(n))
            if olang::pkg::shelf::version_order(n, h) == std::cmp::Ordering::Greater);
        rows.push(serde_json::json!({
            "name": name, "source": "github", "from": repo, "held": newest_held,
            "newest": newest, "behind": behind, "error": error,
        }));
    }
    // folder snapshots: the commits since, in the repository they came from
    for (name, lib) in &shelf.libraries {
        if !names.is_empty() && !names.contains(name) {
            continue;
        }
        for c in lib.copies.iter().filter(|c| c.source == "folder") {
            let (Some(from), Some(commit)) = (&c.from, &c.commit) else {
                continue;
            };
            let since = std::process::Command::new("git")
                .arg("-C")
                .arg(from)
                .args(["rev-list", "--count", &format!("{commit}..HEAD")])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .trim()
                        .parse::<u64>()
                        .ok()
                });
            rows.push(serde_json::json!({
                "name": name, "source": "folder", "from": from, "held": c.version,
                "commit": commit, "commits_since": since, "behind": since.is_some_and(|n| n > 0),
            }));
        }
    }
    if ctx().json {
        emit(
            serde_json::json!({ "format": 1, "kind": "outdated", "scope": "shelf", "libraries": rows }),
        );
        return Ok(());
    }
    if rows.is_empty() {
        println!(
            "nothing on the shelf comes from GitHub or a folder at a commit, so nothing can be behind"
        );
        return Ok(());
    }
    let mut any = false;
    for r in &rows {
        let name = r["name"].as_str().unwrap_or("");
        if let Some(e) = r["error"].as_str() {
            println!("{name}: {e}");
            continue;
        }
        match r["source"].as_str() {
            Some("github") => {
                let held = r["held"].as_str().unwrap_or("-");
                let newest = r["newest"].as_str().unwrap_or("-");
                let mark = if r["behind"].as_bool() == Some(true) {
                    any = true;
                    "  ← newer released"
                } else {
                    ""
                };
                println!(
                    "{name}  held {held}, newest {newest} ({}){mark}",
                    r["from"].as_str().unwrap_or("")
                );
            }
            _ => {
                let since = r["commits_since"].as_u64();
                println!(
                    "{name}  {} at {}: {}",
                    r["held"].as_str().unwrap_or(""),
                    r["commit"]
                        .as_str()
                        .map(|c| &c[..c.len().min(12)])
                        .unwrap_or(""),
                    match since {
                        Some(0) => "its repository has no commits since".to_string(),
                        Some(n) => {
                            any = true;
                            format!(
                                "{n} commit{} since in {}",
                                if n == 1 { "" } else { "s" },
                                r["from"].as_str().unwrap_or("")
                            )
                        }
                        None => "its repository cannot say".to_string(),
                    }
                );
            }
        }
    }
    if any {
        println!("next: otc shelf upgrade (fetches the newest releases; older copies stay)");
    }
    Ok(())
}

fn upgrade(names: &[String], accept_moved: bool) -> Result<()> {
    let mut shelf = load()?;
    for n in names {
        if shelf.library(n).is_none() {
            return Err(fail(
                format!("'{n}' is not on your shelf"),
                "otc shelf list",
            ));
        }
    }
    let targets: Vec<(String, String)> = github_libraries(&shelf, names)
        .into_iter()
        .map(|(n, r, _)| (n.clone(), r))
        .collect();
    let mut changes = Changes::new("shelf upgrade");
    if targets.is_empty() {
        say!("nothing on the shelf comes from GitHub, so there is nothing to upgrade");
    }
    for (name, repo) in targets {
        if ctx().dry_run {
            let remote =
                github::remote(&repo).map_err(|e| fail(e.to_string(), "otc shelf outdated"))?;
            if let Some((tag, v, commit)) = remote.newest_release() {
                let held = shelf
                    .library(&name)
                    .is_some_and(|l| l.by_commit(&commit).is_some());
                if held {
                    say!("{name} {v} is the newest, and held");
                } else {
                    changes.push("fetch", format!("{name} {v}"), format!("{repo} {tag}"));
                }
            }
            continue;
        }
        let f = github::fetch(
            &mut shelf,
            &repo,
            &github::Ask::Newest,
            Some(&name),
            accept_moved,
        )
        .map_err(|e| {
            let t = e.to_string();
            let next = if t.contains("--accept-moved") {
                format!("otc shelf upgrade {name} --accept-moved")
            } else {
                "otc shelf outdated".to_string()
            };
            fail(t, next)
        })?;
        if f.held {
            say!("{name} {} is the newest, and held", f.copy.version);
        } else {
            changes.push(
                "fetch",
                format!("{name} {}", f.copy.version),
                format!(
                    "{repo} {} (older copies stay until `otc shelf prune`)",
                    f.copy.reference.clone().unwrap_or_default()
                ),
            );
        }
    }
    if !ctx().dry_run {
        save(&shelf)?;
    }
    changes.finish(serde_json::json!({}));
    Ok(())
}

fn remove(spec: &str, yes: bool) -> Result<()> {
    let mut shelf = load()?;
    let (name, label) = match spec.split_once('@') {
        Some((n, l)) => (n, Some(l)),
        None => (spec, None),
    };
    let Some(lib) = shelf.library(name) else {
        return Err(fail(
            format!("'{name}' is not on your shelf"),
            "otc shelf list",
        ));
    };
    let users = usage(&shelf);
    // the folders this removal takes, and who uses them
    let mut dirs: Vec<PathBuf> = Vec::new();
    match label {
        None => {
            dirs.extend(lib.copies.iter().map(ShelfCopy::path));
            dirs.extend(lib.local.clone());
        }
        Some("local") => dirs.extend(lib.local.clone()),
        Some(l) => dirs.extend(
            lib.copies
                .iter()
                .filter(|c| c.answers(l))
                .map(ShelfCopy::path),
        ),
    }
    let using: Vec<String> = dirs
        .iter()
        .flat_map(|d| users.get(d).cloned().unwrap_or_default())
        .collect();
    if !using.is_empty() {
        say!("{spec} is used by:");
        for u in &using {
            say!("  {u}");
        }
        if !yes && !ctx().dry_run {
            return Err(fail(
                format!(
                    "{} user(s) of {spec} would need it again (their next install says what is missing)",
                    using.len()
                ),
                format!("otc shelf remove {spec} --yes"),
            ));
        }
    }
    let mut changes = Changes::new("shelf remove");
    if ctx().dry_run {
        let mut probe = shelf.clone();
        // a dry run removes from a copy of the record, never a folder
        if let Some(l) = probe.libraries.get_mut(name) {
            match label {
                None => {
                    for c in &l.copies {
                        changes.push(
                            "remove",
                            format!("{name} {}", c.version),
                            c.path().display().to_string(),
                        );
                    }
                    if let Some(p) = &l.local {
                        changes.push(
                            "forget",
                            format!("{name} local folder"),
                            p.display().to_string(),
                        );
                    }
                }
                Some("local") => {
                    if let Some(p) = &l.local {
                        changes.push(
                            "forget",
                            format!("{name} local folder"),
                            p.display().to_string(),
                        );
                    }
                }
                Some(lab) => {
                    for c in l.copies.iter().filter(|c| c.answers(lab)) {
                        changes.push(
                            "remove",
                            format!("{name} {}", c.version),
                            c.path().display().to_string(),
                        );
                    }
                }
            }
        }
    } else {
        let gone = shelf
            .remove(name, label)
            .map_err(|e| fail(e.to_string(), format!("otc shelf list {name}")))?;
        for g in gone {
            changes.push("remove", g, "");
        }
        save(&shelf)?;
        if label.is_none() && olang::pkg::starter::get(name).is_some() {
            say!("it is a starter library: `otc shelf restore {name}` brings it back");
        }
    }
    changes.finish(serde_json::json!({ "name": name }));
    Ok(())
}

/// `otc shelf prune [--yes]`: the copies nothing uses and that are not
/// what their name gives. Returns the bytes freed (or that would be).
pub(crate) fn prune(yes: bool, report: bool) -> Result<u64> {
    let mut shelf = load()?;
    let users = usage(&shelf);
    let dry = ctx().dry_run || !yes;
    let mut doomed: Vec<(String, ShelfCopy)> = Vec::new();
    for (name, lib) in &shelf.libraries {
        let gives = lib.pick().map(|(p, _)| p.path());
        for c in &lib.copies {
            if users.get(&c.path()).is_some_and(|u| !u.is_empty())
                || gives.as_ref() == Some(&c.path())
            {
                continue;
            }
            doomed.push((name.clone(), c.clone()));
        }
    }
    let mut changes = Changes::new("shelf prune");
    let mut freed = 0u64;
    for (name, c) in &doomed {
        let size = super::doctor::dir_size(&c.path());
        freed += size;
        if dry {
            say!(
                "would remove {name} {} ({}, {})",
                c.version,
                source_words(c),
                super::doctor::human(size)
            );
        } else {
            changes.push(
                "remove",
                format!("{name} {}", c.version),
                super::doctor::human(size),
            );
            let label = c.short_commit().unwrap_or_else(|| c.version.clone());
            let _ = shelf.remove(name, Some(&label));
        }
    }
    if doomed.is_empty() {
        say!("nothing to prune: every copy is used, or is what its name gives");
    } else if dry {
        say!(
            "{} in {} cop{} nothing uses: `otc shelf prune --yes` removes them",
            super::doctor::human(freed),
            doomed.len(),
            if doomed.len() == 1 { "y" } else { "ies" }
        );
    } else {
        save(&shelf)?;
    }
    if report {
        changes.finish(serde_json::json!({ "freed": freed, "dry_run": dry }));
    }
    Ok(freed)
}

fn restore(name: Option<&str>) -> Result<()> {
    let mut shelf = load()?;
    let mut changes = Changes::new("shelf restore");
    let names: Vec<&str> = match name {
        Some(n) => vec![n],
        None => olang::pkg::starter::STARTERS
            .iter()
            .map(|s| s.name)
            .collect(),
    };
    for n in names {
        if olang::pkg::starter::get(n).is_none() {
            return Err(fail(
                format!(
                    "'{n}' is not a starter library (they are: {})",
                    olang::pkg::starter::STARTERS
                        .iter()
                        .map(|s| s.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                "otc shelf list",
            ));
        }
        if ctx().dry_run {
            changes.push(
                "add",
                n.to_string(),
                format!("the starter shipped with olang {}", olang::VERSION),
            );
            continue;
        }
        let dir = shelf
            .restore_starter(n)
            .map_err(|e| fail(e.to_string(), "otc doctor"))?;
        changes.push("add", n.to_string(), dir.display().to_string());
    }
    if !ctx().dry_run {
        save(&shelf)?;
    }
    changes.finish(serde_json::json!({}));
    Ok(())
}

//! The registry's commands: `otc search`, `otc publish`, `otc withdraw`,
//! and the registry half of `otc add <name>`.
//!
//! A registry is a git repository of `<name>.toml` index entries
//! (olang's src/pkg/registry.rs). Installing reads it from wherever
//! `OLANG_REGISTRY` names; `publish` and `withdraw` write into a
//! checkout of it on this machine and stop there: committing the entry
//! and sending it to the index's repository are the person's.

use olang::pkg::cache::{self, GitRef};
use olang::pkg::manifest::{Dependency, Manifest};
use olang::pkg::registry::{self, Registry, Release};
use semver::{Version, VersionReq};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The registry reads go to, brought up to date, or None when none is
/// configured.
fn open_for_reading() -> anyhow::Result<Option<Registry>> {
    let Some(spec) = registry::configured(None) else {
        return Ok(None);
    };
    Registry::open(&spec, false, true)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("{}", e))
}

/// The checkout of the index that `publish` and `withdraw` write into:
/// `--index`, else `OLANG_REGISTRY` when it names a folder.
fn index_for_writing(index: Option<&str>) -> Result<PathBuf, String> {
    let spec = index
        .map(str::to_string)
        .or_else(|| std::env::var("OLANG_REGISTRY").ok())
        .filter(|s| !s.trim().is_empty());
    match spec {
        Some(s) if !registry::is_remote(&s) && Path::new(&s).is_dir() => Ok(PathBuf::from(s)),
        Some(s) if registry::is_remote(&s) => Err(format!(
            "the entry is written into a checkout of the index, and {s} is its address: clone it and pass --index <folder>"
        )),
        Some(s) => Err(format!("the index folder '{s}' does not exist")),
        None => Err(
            "no index to write into: pass --index <folder>, a checkout of the registry's repository"
                .to_string(),
        ),
    }
}

/// `name` or `name@requirement`, for a registry lookup.
fn split_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.split_once('@') {
        Some((name, req)) => (name, Some(req)),
        None => (spec, None),
    }
}

/// The registry dependency `otc add <spec>` means, when the registry
/// holds the name: `name` takes the newest release (`^x.y.z`),
/// `name@1.2` that requirement. None when no registry is configured or
/// it has no such package.
pub fn dependency_for(spec: &str) -> anyhow::Result<Option<(String, Dependency)>> {
    let (name, req) = split_spec(spec);
    let Some(reg) = open_for_reading()? else {
        if req.is_some() {
            anyhow::bail!(
                "'{spec}' asks the registry for a version, and no registry is configured (set OLANG_REGISTRY to an index's folder or its git address)"
            );
        }
        return Ok(None);
    };
    let mut versions = match reg.versions(name) {
        Ok(v) => v,
        Err(registry::RegistryError::NotFound(_)) => return Ok(None),
        Err(e) => anyhow::bail!("{}", e),
    };
    versions.sort();
    let (chosen, written) = match req {
        Some(req) => {
            let parsed = VersionReq::parse(req).map_err(|_| {
                anyhow::anyhow!("'{req}' is not a version requirement (^1.0, >=1.2, 1.4.0)")
            })?;
            let chosen = versions.iter().rev().find(|v| parsed.matches(v)).cloned();
            (chosen, req.to_string())
        }
        None => {
            // the newest release, a pre-release only when there is nothing else
            let chosen = versions
                .iter()
                .rev()
                .find(|v| v.pre.is_empty())
                .or(versions.last())
                .cloned();
            let written = chosen.as_ref().map(|v| format!("^{v}")).unwrap_or_default();
            (chosen, written)
        }
    };
    let Some(chosen) = chosen else {
        anyhow::bail!(
            "the registry has no release of '{name}' that fits{} (published: {})",
            req.map(|r| format!(" '{r}'")).unwrap_or_default(),
            if versions.is_empty() {
                "none that is not withdrawn".to_string()
            } else {
                versions
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
    };
    println!("  {name} {chosen} (from the registry {})", reg.name());
    Ok(Some((name.to_string(), Dependency::Registry(written))))
}

/// What to say about the registry when a bare name was found nowhere.
pub fn not_found_hint(name: &str) -> String {
    match registry::configured(None) {
        Some(spec) => format!("\nThe registry {spec} has no '{name}' either (otc search)"),
        None => String::new(),
    }
}

/// `otc search [words…]`: the registry's packages whose name or
/// description holds every word; all of them with no words.
pub fn search(words: &[String]) -> anyhow::Result<()> {
    let Some(reg) = open_for_reading()? else {
        anyhow::bail!(
            "no registry is configured (set OLANG_REGISTRY to an index's folder or its git address)"
        );
    };
    let words: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for name in reg.names() {
        let Ok(entry) = reg.entry(&name) else {
            continue;
        };
        let about = entry.description.clone().unwrap_or_default();
        let hay = format!("{name} {about}").to_lowercase();
        if !words.iter().all(|w| hay.contains(w)) {
            continue;
        }
        let latest = entry
            .releases
            .iter()
            .filter(|r| !r.withdrawn)
            .map(|r| &r.version)
            .max()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "withdrawn".to_string());
        rows.push((name, latest, about));
    }
    if rows.is_empty() {
        println!("Nothing in {} matches.", reg.name());
        return Ok(());
    }
    let wide = rows.iter().map(|r| r.0.len()).max().unwrap_or(0);
    let wide_v = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
    for (name, latest, about) in rows {
        println!("  {name:<wide$}  {latest:<wide_v$}  {about}");
    }
    Ok(())
}

/// The olang beside this otc, else `$OLANG`, else the one on PATH.
fn olang_binary() -> PathBuf {
    if let Ok(p) = std::env::var("OLANG") {
        return PathBuf::from(p);
    }
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("olang"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("olang"))
}

pub struct PublishArgs {
    pub index: Option<String>,
    pub git: Option<String>,
    pub dry_run: bool,
    pub force: bool,
    pub no_test: bool,
}

/// `otc publish`: check the package in this folder and write its
/// release into a checkout of the index.
///
/// Everything it finds wrong is said together, and nothing is written
/// until all of it passes. It commits nothing and sends nothing.
pub fn publish(args: PublishArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let root = Manifest::find_root(&cwd).ok_or_else(|| {
        anyhow::anyhow!("no olang.toml here or above: publish runs in a package's folder")
    })?;
    let manifest = Manifest::load(&root).map_err(|e| anyhow::anyhow!("{}", e))?;
    let name = manifest.package.name.clone();
    let version = manifest.package.version.clone();
    let mut problems: Vec<String> = Vec::new();

    if !registry::valid_name(&name) {
        problems.push(format!(
            "the name '{name}' cannot be a registry name (letters, digits, '-' and '_')"
        ));
    }
    if manifest
        .package
        .description
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        problems.push("olang.toml has no description: [package] description says what the library is, in a line".into());
    }
    if manifest
        .package
        .license
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        problems.push(
            "olang.toml names no license: [package] license = \"MIT\" (or the one you chose)"
                .into(),
        );
    }
    let has = |stems: &[&str]| {
        std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                let file = e.file_name().to_string_lossy().to_lowercase();
                stems
                    .iter()
                    .any(|s| file == *s || file.starts_with(&format!("{s}.")))
            })
    };
    if !has(&["license", "licence", "copying"]) {
        problems.push("there is no LICENSE file".into());
    }
    if !has(&["readme"]) {
        problems.push("there is no README".into());
    }

    // A registry release depends on registry releases: a path, a git
    // address or another machine's shelf is nothing a stranger can install.
    let mut dependencies = std::collections::BTreeMap::new();
    for (dep, source) in &manifest.dependencies {
        match source {
            Dependency::Registry(req) | Dependency::RegistryExplicit { version: req, .. } => {
                dependencies.insert(dep.clone(), req.clone());
            }
            Dependency::Shelf { shelf, .. }
                if olang::pkg::starter::get(shelf).is_some() || *shelf == name => {}
            Dependency::Shelf { shelf, .. } => problems.push(format!(
                "'{dep}' comes from your shelf ('{shelf}'), which only this machine has: publish it and depend on a version ({dep} = \"^1.0\")"
            )),
            Dependency::Path { path } => problems.push(format!(
                "'{dep}' is a path dependency ({path}): publish it and depend on a version ({dep} = \"^1.0\")"
            )),
            Dependency::Git { git, .. } => problems.push(format!(
                "'{dep}' is a git dependency ({git}): publish it and depend on a version ({dep} = \"^1.0\")"
            )),
            Dependency::GitHub { github, .. } => problems.push(format!(
                "'{dep}' is a GitHub dependency ({github}): publish it and depend on a version ({dep} = \"^1.0\")"
            )),
        }
    }

    // The commit that is published, and where a stranger fetches it.
    let top = git(&root, &["rev-parse", "--show-toplevel"]).map(PathBuf::from);
    let head = git(&root, &["rev-parse", "HEAD"]);
    if top.is_none() || head.is_none() {
        problems.push(
            "this folder is not in a git repository with a commit: a release is a commit".into(),
        );
    } else if !git(
        &root,
        &["status", "--porcelain", "--untracked-files=no", "--", "."],
    )
    .unwrap_or_default()
    .is_empty()
    {
        // tracked files only: a file git does not track is not in the
        // commit, so it is not in the release either
        problems.push(
            "there are uncommitted changes here: a release is a commit, so commit them first"
                .into(),
        );
    }
    let url = args
        .git
        .clone()
        .or_else(|| git(&root, &["remote", "get-url", "origin"]))
        .filter(|u| !u.is_empty());
    if url.is_none() && top.is_some() {
        problems.push("the repository has no `origin` to fetch the release from: add one, or pass --git <address>".into());
    }
    let index = match index_for_writing(args.index.as_deref()) {
        Ok(dir) => Some(Registry::at(dir)),
        Err(e) => {
            problems.push(e);
            None
        }
    };
    if let Some(reg) = &index {
        for (dep, req) in &dependencies {
            let fits = VersionReq::parse(req).ok().and_then(|r| {
                reg.versions(dep)
                    .ok()
                    .map(|vs| vs.iter().any(|v| r.matches(v)))
            });
            if fits != Some(true) {
                problems.push(format!(
                    "the index has no release of '{dep}' that fits '{req}': publish {dep} first"
                ));
            }
        }
        if !args.force
            && reg
                .entry(&name)
                .map(|e| e.releases.iter().any(|r| r.version == version))
                .unwrap_or(false)
        {
            problems.push(format!(
                "{name} {version} is already in the index, and a release is never rewritten: raise the version in olang.toml"
            ));
        }
    }
    if !problems.is_empty() {
        anyhow::bail!(
            "{name} {version} is not ready to publish:\n{}",
            problems
                .iter()
                .map(|p| format!("  - {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    let (top, head, url, index) = (top.unwrap(), head.unwrap(), url.unwrap(), index.unwrap());

    // Its own dependencies, fetched as a stranger's install will fetch
    // them: `olang check` reads a project and fetches nothing.
    olang::pkg::install(
        &root,
        &olang::pkg::InstallOptions {
            explicit: true,
            ..Default::default()
        },
    )
    .map_err(|e| anyhow::anyhow!("{name}'s own dependencies do not install: {e}"))?;

    if !args.no_test {
        let olang = olang_binary();
        for step in [vec!["check", "."], vec!["test"]] {
            println!("  olang {}", step.join(" "));
            let out = Command::new(&olang)
                .current_dir(&root)
                .args(&step)
                .output()?;
            if !out.status.success() {
                let said = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                let tail: Vec<&str> = said.lines().rev().take(20).collect();
                anyhow::bail!(
                    "`olang {}` fails here, so nothing was published:\n{}",
                    step.join(" "),
                    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
                );
            }
        }
    }

    // Fetched the way an install will: this proves the commit is at the
    // address, and the checksum is of what a stranger receives.
    let fetched = cache::fetch_git(&url, &GitRef::Rev(head.clone()), false).map_err(|e| {
        anyhow::anyhow!(
            "commit {} is not at {url} ({e}): push it first, since that address is where the release is fetched from",
            &head[..head.len().min(12)]
        )
    })?;
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let subdir = canonical(&root)
        .strip_prefix(canonical(&top))
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .filter(|s| !s.is_empty());
    let dir = match &subdir {
        Some(sub) => fetched.dir.join(sub),
        None => fetched.dir.clone(),
    };
    let checksum = cache::checksum_dir(&dir).map_err(|e| anyhow::anyhow!("{}", e))?;

    let release = Release {
        version: version.clone(),
        git: url.clone(),
        rev: head.clone(),
        subdir: subdir.clone(),
        checksum: Some(checksum.clone()),
        withdrawn: false,
        reason: None,
        dependencies,
    };
    println!("  {name} {version}");
    println!(
        "    from      {url}{}",
        subdir
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default()
    );
    println!("    commit    {head}");
    println!("    checksum  {checksum}");
    let tags = git(&root, &["tag", "--points-at", "HEAD"]).unwrap_or_default();
    if !tags
        .lines()
        .any(|t| t == version.to_string() || t == format!("v{version}"))
    {
        println!(
            "    note: no tag v{version} points at this commit; the entry pins the commit either way"
        );
    }
    if args.dry_run {
        println!("Dry run: nothing was written.");
        return Ok(());
    }
    index
        .publish(&name, release, args.force)
        .and_then(|_| {
            index.set_about(
                &name,
                manifest.package.description.clone(),
                manifest.package.license.clone(),
            )
        })
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let file = index.root().join(format!("{name}.toml"));
    println!("Wrote {}", file.display());
    println!(
        "It is published once that file is in the index's repository: commit it there and send it (a pull request, for an index that is not yours)."
    );
    Ok(())
}

/// `otc withdraw name@version`: mark a release withdrawn in a checkout
/// of the index (or, with `--restore`, put it back).
pub fn withdraw(
    spec: &str,
    reason: Option<String>,
    restore: bool,
    index: Option<&str>,
) -> anyhow::Result<()> {
    let (name, version) = spec.split_once('@').ok_or_else(|| {
        anyhow::anyhow!("name the release as name@version (otc withdraw textkit@1.0.0)")
    })?;
    let version = Version::parse(version)
        .map_err(|_| anyhow::anyhow!("'{version}' is not a version (1.0.0)"))?;
    let reg = Registry::at(index_for_writing(index).map_err(|e| anyhow::anyhow!("{}", e))?);
    reg.set_withdrawn(name, &version, !restore, reason)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let file = reg.root().join(format!("{name}.toml"));
    if restore {
        println!(
            "{name} {version} is no longer withdrawn in {}",
            file.display()
        );
    } else {
        println!("{name} {version} is withdrawn in {}", file.display());
        println!("No new install chooses it; a project whose lock names it still installs it.");
    }
    println!("It takes effect once that file is in the index's repository.");
    Ok(())
}

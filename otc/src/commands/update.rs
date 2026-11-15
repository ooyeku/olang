//! The olang versions on this machine: `otc use`, `otc update` and `otc
//! toolchain list | add | remove | prune | link | unlink | which`
//! (OL-579, OL-580).
//!
//! Releases install side by side under `~/.olang/toolchains/<version>/`
//! (`releases.rs` downloads and verifies them); the shims in
//! `~/.olang/bin/` are symlinks to the default, so switching is one atomic
//! relink and a running olang is never overwritten. A linked local build
//! is a checkout recorded by name in `~/.olang/toolchains.toml`, its olang
//! always the checkout's current `target/release/olang`. Installations
//! owned by other package managers (Homebrew, cargo) are never touched.

use super::releases::{self, Fetch, normalize, toolchains_dir};
use super::{Changes, ctx, fail};
use crate::say;
use anyhow::{Context, Result};
use clap::Subcommand;
use olang::pkg::toolchain::{Linked, Toolchains};
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum ToolchainCommand {
    /// Every olang on this machine, which one runs and why; with
    /// --available, every release too.
    ///
    /// Example: otc toolchain list --available
    List {
        /// Also every release on GitHub (cached an hour)
        #[arg(long, alias = "remote")]
        available: bool,
        /// With --available: leave prereleases out
        #[arg(long)]
        stable: bool,
        /// With --available: the releases as last fetched, no network
        #[arg(long)]
        offline: bool,
    },
    /// Install a version if needed and make it the default (as `otc use`).
    ///
    /// Example: otc toolchain use 0.88
    Use {
        /// A version, a prefix (1.0), latest, stable, rc, a linked
        /// build's name, or - for the one before
        version: String,
        /// Pin this project (olang.toml and olang.lock) instead
        #[arg(long)]
        project: bool,
        /// With --project: allow a linked build's name in olang.lock
        #[arg(long)]
        local: bool,
    },
    /// Install a release without switching to it.
    ///
    /// Example: otc toolchain add 1.0.0-rc.8
    #[command(alias = "install")]
    Add {
        /// A version, a prefix, latest, stable or rc
        version: String,
    },
    /// Remove an installed release (never the one in use).
    ///
    /// Example: otc toolchain remove 0.80.0
    Remove {
        version: String,
        /// Remove it even though known projects pin it
        #[arg(long)]
        yes: bool,
    },
    /// Remove the releases no known project pins and that are not the
    /// default (says what first; --yes removes).
    ///
    /// Example: otc toolchain prune --yes
    Prune {
        #[arg(long)]
        yes: bool,
    },
    /// Make a local checkout's build a toolchain, named by its branch.
    ///
    /// Example: otc toolchain link ../olang
    Link {
        /// The checkout (its target/release/olang is the toolchain)
        path: String,
        /// The name (default: the checkout's branch)
        #[arg(long)]
        name: Option<String>,
    },
    /// Forget a linked build (the checkout is not touched).
    ///
    /// Example: otc toolchain unlink main
    Unlink { name: String },
    /// Print the olang a project runs.
    ///
    /// Example: otc toolchain which
    Which {
        /// The project's folder (default: the one this is run in)
        dir: Option<String>,
    },
    /// Install the newest release and make it the default (as `otc update`).
    ///
    /// Example: otc toolchain upgrade
    Upgrade {
        /// Only say what the newest is
        #[arg(long)]
        check: bool,
    },
}

pub fn toolchain(cmd: ToolchainCommand) -> Result<()> {
    match cmd {
        ToolchainCommand::List {
            available,
            stable,
            offline,
        } => super::toolchains::list(available, stable, offline),
        ToolchainCommand::Use {
            version,
            project,
            local,
        } => use_version(&version, project, local),
        ToolchainCommand::Add { version } => add(&version),
        ToolchainCommand::Remove { version, yes } => remove(&version, yes),
        ToolchainCommand::Prune { yes } => prune(yes),
        ToolchainCommand::Link { path, name } => link(&path, name.as_deref()),
        ToolchainCommand::Unlink { name } => unlink(&name),
        ToolchainCommand::Which { dir } => which(dir),
        ToolchainCommand::Upgrade { check } => update(check),
    }
}

/// What a version argument names.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// An installed release.
    Installed(String),
    /// A release to download first.
    Release(String),
    /// A linked local build.
    Linked(String),
}

impl Target {
    pub fn name(&self) -> &str {
        match self {
            Target::Installed(v) | Target::Release(v) | Target::Linked(v) => v,
        }
    }
}

/// Whether `v` is in the series `prefix` names (`1.0` is 1.0.x and its
/// prereleases; `1` every 1.x).
fn in_series(prefix: &str, v: &str) -> bool {
    let Ok(v) = semver::Version::parse(v) else {
        return false;
    };
    let parts: Vec<&str> = prefix.split('.').collect();
    let n: Vec<Option<u64>> = parts.iter().map(|p| p.parse().ok()).collect();
    if n.iter().any(Option::is_none) || n.is_empty() || n.len() > 3 {
        return false;
    }
    let want = [v.major, v.minor, v.patch];
    n.iter().enumerate().all(|(i, x)| *x == Some(want[i]))
}

/// The installed releases, newest first.
pub fn installed() -> Vec<String> {
    olang::pkg::toolchain::installed()
        .into_iter()
        .map(|(v, _)| v)
        .collect()
}

/// Resolve a version argument: an installed match answers without the
/// network; otherwise the releases list is read (cached for an hour).
pub fn resolve(spec: &str) -> Result<Target> {
    let spec = spec.trim();
    let tc = Toolchains::load();
    if spec == "-" {
        return match tc.previous {
            Some(p) => resolve(&p),
            None => Err(fail(
                "there is no olang before this one to go back to",
                "otc toolchain list",
            )),
        };
    }
    if tc.linked.contains_key(spec) {
        return Ok(Target::Linked(spec.to_string()));
    }
    let have = installed();
    let v = normalize(spec);
    let exact = semver::Version::parse(&v).is_ok();
    if exact && have.contains(&v) {
        return Ok(Target::Installed(v));
    }
    let series = !exact && v.split('.').all(|p| p.parse::<u64>().is_ok()) && !v.is_empty();
    if series && let Some(found) = have.iter().find(|h| in_series(&v, h)) {
        return Ok(Target::Installed(found.clone()));
    }
    let word = matches!(spec, "latest" | "stable" | "rc");
    if !exact && !series && !word {
        let linked: Vec<String> = tc.linked.keys().cloned().collect();
        return Err(fail(
            format!(
                "'{spec}' is not a version, a series (1.0), latest, stable, rc, or a linked build{}",
                if linked.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", linked.join(", "))
                }
            ),
            "otc toolchain list --available",
        ));
    }
    let listing = releases::list(Fetch::Cached)?;
    if let Some(note) = &listing.note {
        say!("note: {note} ({})", releases::age(listing.age_s));
    }
    let found = listing.releases.iter().find(|r| match spec {
        "latest" => true,
        "stable" => !r.prerelease,
        "rc" => r.prerelease,
        _ if exact => r.version == v,
        _ => in_series(&v, &r.version),
    });
    match found {
        Some(r) if have.contains(&r.version) => Ok(Target::Installed(r.version.clone())),
        Some(r) => Ok(Target::Release(r.version.clone())),
        None => Err(fail(
            format!("no release of olang answers '{spec}'"),
            "otc toolchain list --available",
        )),
    }
}

fn bin_dir() -> Result<PathBuf> {
    olang::home::bin().context("no home directory could be determined (set HOME or OLANG_HOME)")
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The default toolchain: where the olang shim points, as a release's
/// version or a linked build's name.
pub fn default_toolchain() -> Result<Option<String>> {
    let shim = bin_dir()?.join(exe("olang"));
    let Ok(target) = std::fs::read_link(&shim) else {
        return Ok(None);
    };
    let tc = Toolchains::load();
    if let Some((name, _)) = tc.linked.iter().find(|(_, l)| l.olang() == target) {
        return Ok(Some(name.clone()));
    }
    // .../toolchains/<version>/bin/olang
    Ok(target
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .map(|v| v.to_string_lossy().into_owned()))
}

/// Repoint the shims at `target` and record it as the default, the old
/// one as `previous`. Returns a note when the otc shim was left alone.
fn set_default(target: &Target) -> Result<Option<String>> {
    let bins = bin_dir()?;
    std::fs::create_dir_all(&bins)?;
    let tc = Toolchains::load();
    let (olang_bin, otc_bin) = match target {
        Target::Linked(name) => {
            let l = tc.linked.get(name).expect("resolved");
            (l.olang(), l.otc())
        }
        Target::Installed(v) | Target::Release(v) => {
            let dir = toolchains_dir()?.join(v).join("bin");
            (dir.join(exe("olang")), dir.join(exe("otc")))
        }
    };
    if !olang_bin.is_file() {
        return Err(fail(
            format!("{} has no olang at {}", target.name(), olang_bin.display()),
            match target {
                Target::Linked(_) => "cargo build --release (in the checkout)".to_string(),
                _ => format!("otc toolchain add {}", target.name()),
            },
        ));
    }
    let mut note = None;
    for (name, bin) in [("olang", &olang_bin), ("otc", &otc_bin)] {
        if !bin.is_file() {
            note = Some(format!(
                "{} has no otc built ({}), so the otc shim still runs the one it did",
                target.name(),
                bin.display()
            ));
            continue;
        }
        let shim = bins.join(exe(name));
        let staged = bins.join(format!(".{}.new", name));
        let _ = std::fs::remove_file(&staged);
        link_file(bin, &staged).with_context(|| format!("linking {}", shim.display()))?;
        std::fs::rename(&staged, &shim)?;
    }
    let before = default_from_record_or_shim(&tc);
    let mut tc = Toolchains::load();
    if before.as_deref() != Some(target.name()) {
        tc.previous = before;
    }
    tc.default = Some(target.name().to_string());
    tc.save().context("writing toolchains.toml")?;
    Ok(note)
}

fn default_from_record_or_shim(tc: &Toolchains) -> Option<String> {
    tc.default
        .clone()
        .or_else(|| default_toolchain().ok().flatten())
}

/// A symlink on Unix. Windows lets only administrators and developer
/// mode make symlinks, so there the file is copied instead.
fn link_file(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(target, link).map(|_| ())
    }
}

/// What `olang --version` answers now, the binary that answered, and the
/// line that says something else on PATH shadows the default.
pub fn now_running() -> serde_json::Value {
    let home_bin = olang::home::bin().unwrap_or_default();
    let toolchains = olang::home::toolchains().unwrap_or_default();
    let rows = super::toolchains::rows(&home_bin, &toolchains);
    let active = rows.iter().find(|r| r.active);
    let shadow = super::toolchains::shadow_note(&rows, &home_bin);
    serde_json::json!({
        "default": default_toolchain().ok().flatten(),
        "olang_version": active.and_then(|r| r.version.clone()),
        "olang": active.map(|r| r.path.display().to_string()),
        "shadowed": shadow.as_ref().map(|(s, _)| s),
        "fix": shadow.as_ref().map(|(_, f)| f),
    })
}

fn say_now(now: &serde_json::Value) {
    match (now["olang_version"].as_str(), now["olang"].as_str()) {
        (Some(v), Some(at)) => say!("`olang --version` now answers olang {v} ({at})"),
        _ => say!("no olang is on PATH, so `olang` runs nothing yet"),
    }
    if let (Some(why), Some(fix)) = (now["shadowed"].as_str(), now["fix"].as_str()) {
        say!("{why}");
        say!("fix: {fix}");
    } else if now["olang"].is_null() {
        say!(
            "fix: export PATH=\"{}:$PATH\"",
            olang::home::bin().unwrap_or_default().display()
        );
    }
}

/// `otc use <version> [--project [--local]]`.
pub fn use_version(spec: &str, project: bool, local: bool) -> Result<()> {
    let target = resolve(spec)?;
    let mut changes = Changes::new("use");
    if let Target::Release(v) = &target {
        changes.push(
            "install",
            format!("olang {v}"),
            "downloaded from GitHub, checksum verified",
        );
        if !ctx().dry_run {
            releases::install_version(v)
                .map_err(|e| fail(format!("{e:#}"), "otc toolchain list --available"))?;
        }
    }
    if project {
        return pin_project(&target, local, changes);
    }
    if default_toolchain()?.as_deref() == Some(target.name())
        && !matches!(target, Target::Release(_))
    {
        say!("olang {} is already the default", target.name());
    } else {
        changes.push(
            "switch",
            format!("olang {}", target.name()),
            "the shims in ~/.olang/bin point at it",
        );
        if !ctx().dry_run
            && let Some(note) = set_default(&target)?
        {
            say!("note: {note}");
        }
    }
    let now = now_running();
    if !ctx().dry_run {
        say_now(&now);
    }
    changes.finish(now);
    Ok(())
}

/// `otc use <v> --project`: the project's `[package] olang` and lock.
fn pin_project(target: &Target, local: bool, mut changes: Changes) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let root = olang::pkg::manifest::Manifest::find_root(&cwd).ok_or_else(|| {
        fail(
            format!("no olang.toml at or above {}", cwd.display()),
            "otc new <name> (or run this inside a project)",
        )
    })?;
    let name = target.name().to_string();
    let linked = matches!(target, Target::Linked(_));
    if linked && !local {
        return Err(fail(
            format!(
                "'{name}' is a local build: its name means nothing on another machine, so it goes \
                 only into this project's olang.lock, and only when asked"
            ),
            format!("otc use {name} --project --local"),
        ));
    }
    if !linked {
        changes.push(
            "pin",
            "olang.toml",
            format!("[package] olang = \"{name}\" (this olang or newer)"),
        );
    }
    changes.push("pin", "olang.lock", format!("olang = \"{name}\""));
    if !ctx().dry_run {
        if !linked {
            olang::pkg::manifest::Manifest::set_package_key(&root, "olang", &name)
                .map_err(|e| fail(e.to_string(), "otc doctor"))?;
        }
        let mut lock = olang::pkg::lock::Lockfile::load(&root)
            .map_err(|e| fail(e, "otc install (or remove olang.lock to resolve afresh)"))?;
        lock.olang = Some(name.clone());
        lock.save(&root).map_err(|e| fail(e, "otc doctor"))?;
        olang::pkg::projects::record(&root);
        say!("this project ({}) now runs olang {name}", root.display());
    }
    changes.finish(serde_json::json!({ "project": root.display().to_string(), "olang": name }));
    Ok(())
}

/// `otc update [--check]`: `otc use latest`.
pub fn update(check: bool) -> Result<()> {
    if check {
        let listing = releases::list(Fetch::Cached)?;
        let latest = listing.releases.first().map(|r| r.version.clone());
        let current = default_toolchain()?.unwrap_or_else(|| olang::VERSION.to_string());
        say!("default: {current}");
        say!("latest:  {}", latest.as_deref().unwrap_or("?"));
        match &latest {
            Some(l) if *l == current => say!("up to date."),
            Some(l) => say!("`otc use latest` (or `otc update`) installs {l} and switches to it."),
            None => {}
        }
        super::emit(serde_json::json!({
            "format": 1, "kind": "update-check", "default": current, "latest": latest,
        }));
        return Ok(());
    }
    // Never down: "latest" is whatever the release server says, and an
    // older release is signed as well as a newer one, so a server (or a
    // mistake) that named an old release made it the default, with the
    // bugs this one fixed. `otc use <version>` takes an older one on
    // purpose.
    let latest = resolve("latest")?;
    let current = default_toolchain()?.unwrap_or_else(|| olang::VERSION.to_string());
    if releases::is_older(latest.name(), &current) {
        return Err(fail(
            format!(
                "the newest release the server names, {}, is older than this olang ({current}); \
                 nothing was changed",
                latest.name()
            ),
            format!("otc use {} (if you mean to go back)", latest.name()),
        ));
    }
    use_version("latest", false, false)
}

fn add(spec: &str) -> Result<()> {
    let target = resolve(spec)?;
    let mut changes = Changes::new("toolchain add");
    match &target {
        Target::Release(v) => {
            changes.push("install", format!("olang {v}"), "");
            if !ctx().dry_run {
                releases::install_version(v)
                    .map_err(|e| fail(format!("{e:#}"), "otc toolchain list --available"))?;
                say!("`otc use {v}` switches to it");
            }
        }
        Target::Installed(v) => {
            say!("olang {v} is already installed (`otc use {v}` switches to it)")
        }
        Target::Linked(n) => say!("'{n}' is a linked build (`otc use {n}` switches to it)"),
    }
    changes.finish(now_running());
    Ok(())
}

/// The known projects whose lock runs toolchain `name`, or whose pin
/// resolves to it.
pub fn projects_using(name: &str) -> Vec<PathBuf> {
    use olang::pkg::toolchain::{Resolved, for_project};
    olang::pkg::projects::known()
        .into_iter()
        .filter(|root| {
            olang::pkg::toolchain::lock_pin(root).as_deref() == Some(name)
                || matches!(for_project(root), Resolved::Installed { version, .. } if version == name)
        })
        .collect()
}

fn remove(spec: &str, yes: bool) -> Result<()> {
    // one folder's name under the toolchains folder: `..` named its
    // parent, `~/.olang`, whole
    let v = releases::toolchain_name(spec)
        .map_err(|e| fail(format!("{e:#}"), "otc toolchain list"))?;
    if Toolchains::load().linked.contains_key(&v) {
        return Err(fail(
            format!("'{v}' is a linked build"),
            format!("otc toolchain unlink {v}"),
        ));
    }
    if default_toolchain()?.as_deref() == Some(v.as_str()) {
        return Err(fail(
            format!("olang {v} is the default, the one in use; switch to another first"),
            "otc use <another version> (otc toolchain list shows them)",
        ));
    }
    let dir = toolchains_dir()?.join(&v);
    if !dir.exists() {
        return Err(fail(
            format!("olang {v} is not installed"),
            "otc toolchain list",
        ));
    }
    let users = projects_using(&v);
    if !users.is_empty() {
        say!("olang {v} is pinned by:");
        for u in &users {
            say!("  {}", u.display());
        }
        if !yes && !ctx().dry_run {
            return Err(fail(
                format!(
                    "{} known project(s) pin olang {v}; they would need it again",
                    users.len()
                ),
                format!("otc toolchain remove {v} --yes"),
            ));
        }
    }
    let mut changes = Changes::new("toolchain remove");
    changes.push("remove", format!("olang {v}"), dir.display().to_string());
    if !ctx().dry_run {
        olang::pkg::shelf::remove_tree(&dir);
    }
    changes.finish(now_running());
    Ok(())
}

fn prune(yes: bool) -> Result<()> {
    let default = default_toolchain()?;
    let dry = ctx().dry_run || !yes;
    let mut changes = Changes::new("toolchain prune");
    let mut kept = Vec::new();
    let mut freed = 0u64;
    for v in installed() {
        if default.as_deref() == Some(v.as_str()) {
            kept.push((v, "the default".to_string()));
            continue;
        }
        let users = projects_using(&v);
        if !users.is_empty() {
            kept.push((v, format!("pinned by {}", users[0].display())));
            continue;
        }
        let dir = toolchains_dir()?.join(&v);
        let size = super::doctor::dir_size(&dir);
        freed += size;
        if dry {
            say!("would remove olang {v} ({})", super::doctor::human(size));
        } else {
            changes.push("remove", format!("olang {v}"), super::doctor::human(size));
            olang::pkg::shelf::remove_tree(&dir);
        }
    }
    for (v, why) in &kept {
        say!("keeping olang {v} ({why})");
    }
    if dry && freed > 0 {
        say!(
            "{} would be freed: `otc toolchain prune --yes` removes them",
            super::doctor::human(freed)
        );
    } else if freed == 0 {
        say!("nothing to prune");
    }
    changes.finish(serde_json::json!({ "freed": freed, "dry_run": dry }));
    Ok(())
}

/// A checkout's branch, if git can say.
fn branch_of(checkout: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !b.is_empty() && b != "HEAD").then_some(b)
}

fn link(path: &str, name: Option<&str>) -> Result<()> {
    let given = PathBuf::from(path);
    let given = given.canonicalize().map_err(|_| {
        fail(
            format!("{path} does not exist"),
            "otc toolchain link <checkout of olang>",
        )
    })?;
    // a checkout, or its target/release/olang
    let checkout = if given.is_dir() {
        given
    } else {
        given
            .ancestors()
            .find(|a| a.join("Cargo.toml").is_file())
            .map(Path::to_path_buf)
            .ok_or_else(|| {
                fail(
                    format!("{} is not inside an olang checkout", given.display()),
                    "otc toolchain link <checkout of olang>",
                )
            })?
    };
    if !checkout.join("Cargo.toml").is_file() {
        return Err(fail(
            format!(
                "{} is not an olang checkout (it has no Cargo.toml)",
                checkout.display()
            ),
            "otc toolchain link <checkout of olang>",
        ));
    }
    let name = match name {
        Some(n) => n.to_string(),
        None => branch_of(&checkout).ok_or_else(|| {
            fail(
                format!(
                    "{} has no branch checked out to name it by",
                    checkout.display()
                ),
                format!("otc toolchain link {path} --name <name>"),
            )
        })?,
    };
    let reserved = matches!(name.as_str(), "latest" | "stable" | "rc" | "-")
        || semver::Version::parse(&name).is_ok()
        || name.split('.').all(|p| p.parse::<u64>().is_ok())
        || name.contains(['/', '\\', ' ']);
    if reserved {
        return Err(fail(
            format!("'{name}' would read as a version; a linked build needs a name of its own"),
            format!("otc toolchain link {path} --name <name>"),
        ));
    }
    let linked = Linked {
        checkout: checkout.clone(),
    };
    let mut changes = Changes::new("toolchain link");
    changes.push(
        "link",
        format!("'{name}'"),
        format!("{}", linked.olang().display()),
    );
    if !ctx().dry_run {
        let mut tc = Toolchains::load();
        tc.linked.insert(name.clone(), linked.clone());
        tc.save().context("writing toolchains.toml")?;
        let info = super::toolchains::linked_info(&linked);
        say!("{name} is {}", info.describe());
        if let Some(stale) = &info.stale {
            say!("note: {stale}");
        }
        say!("`otc use {name}` switches to it");
    }
    changes.finish(serde_json::json!({ "name": name, "checkout": checkout.display().to_string() }));
    Ok(())
}

fn unlink(name: &str) -> Result<()> {
    let tc = Toolchains::load();
    if !tc.linked.contains_key(name) {
        return Err(fail(
            format!("no linked build is named '{name}'"),
            "otc toolchain list",
        ));
    }
    if default_toolchain()?.as_deref() == Some(name) {
        return Err(fail(
            format!("'{name}' is the default, the one in use; switch to another first"),
            "otc use <version>",
        ));
    }
    let mut changes = Changes::new("toolchain unlink");
    changes.push("unlink", format!("'{name}'"), "its checkout is not touched");
    if !ctx().dry_run {
        let mut tc = tc;
        tc.linked.remove(name);
        if tc.previous.as_deref() == Some(name) {
            tc.previous = None;
        }
        tc.save().context("writing toolchains.toml")?;
    }
    changes.finish(now_running());
    Ok(())
}

fn which(dir: Option<String>) -> Result<()> {
    let start = match dir {
        Some(d) => PathBuf::from(d),
        None => std::env::current_dir()?,
    };
    let root = olang::pkg::manifest::Manifest::find_root(&start).ok_or_else(|| {
        fail(
            format!("no olang.toml at or above {}", start.display()),
            "otc toolchain list (the olang outside a project)",
        )
    })?;
    use olang::pkg::toolchain::{Resolved, for_project, install_hint};
    let (bin, why) = match for_project(&root) {
        Resolved::Installed { bin, version, pin } => {
            (bin, format!("its pin {pin} resolves to {version}"))
        }
        Resolved::Running { pin } => (
            std::env::current_exe()?.with_file_name(exe("olang")),
            format!(
                "its pin {pin} is met by the running olang {}",
                olang::VERSION
            ),
        ),
        Resolved::Unpinned => (
            std::env::current_exe()?.with_file_name(exe("olang")),
            "it pins no olang".to_string(),
        ),
        Resolved::Missing { pin } => {
            return Err(fail(
                format!("olang.toml pins olang {pin}, and no installed olang satisfies it"),
                install_hint(&pin),
            ));
        }
    };
    if ctx().json {
        super::emit(serde_json::json!({
            "format": 1, "kind": "which", "root": root.display().to_string(),
            "olang": bin.display().to_string(), "why": why,
        }));
    } else {
        println!("{}", bin.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::in_series;

    #[test]
    fn a_series_is_a_prefix_of_the_version() {
        assert!(in_series("1.0", "1.0.0-rc.8"));
        assert!(in_series("1.0", "1.0.3"));
        assert!(!in_series("1.0", "1.1.0"));
        assert!(in_series("0.88", "0.88.0"));
        assert!(in_series("1", "1.4.0"));
        assert!(!in_series("0.8", "0.88.0"));
    }
}

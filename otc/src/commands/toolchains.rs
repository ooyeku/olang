//! `otc toolchain list` (and a bare `otc toolchain`): every olang this
//! machine has, which one runs and why (OL-577), and with `--available`
//! every release on GitHub beside them (OL-578).
//!
//! An olang is found four ways: a managed release under
//! `~/.olang/toolchains`, a linked local build (`toolchains.toml`), the
//! shims in `~/.olang/bin` (the default), and every `olang` on PATH
//! (cargo, Homebrew, a checkout's build, anything else). Each binary is one row however many ways reach
//! it, and its version is the binary's own answer to `--version`, never a
//! folder's name. The **active** olang is the first on PATH, the one a
//! bare `olang` runs; the **default** is the shims'. When they differ,
//! one line says which shadows which and the command that fixes it (the
//! newer of the two is never the one to remove, OL-638); in a project,
//! one more says what its pin resolves to.

use super::doctor::path_dirs;
use super::{ctx, emit};
use anyhow::{Context, Result};
use olang::pkg::toolchain::{Linked, Toolchains};
use std::path::{Path, PathBuf};

/// One olang binary on this machine.
pub(crate) struct Row {
    /// What the binary says it is (`olang --version`), `None` when it
    /// does not answer.
    pub(crate) version: Option<String>,
    /// `managed`, `linked`, `shims`, `cargo`, `homebrew`, `checkout`, `path`.
    pub(crate) kind: &'static str,
    /// Where it was found (a toolchain's `bin/olang`, a PATH entry's).
    pub(crate) path: PathBuf,
    /// The file itself, every link followed: one row per binary.
    pub(crate) real: PathBuf,
    /// The toolchain folder's name, for a managed or linked one.
    pub(crate) folder: Option<String>,
    pub(crate) active: bool,
    pub(crate) default: bool,
    pub(crate) on_path: bool,
    /// For a linked build: its checkout's state.
    pub(crate) linked: Option<LinkedInfo>,
}

/// What a linked build's checkout says of itself.
#[derive(Debug, Clone, Default)]
pub(crate) struct LinkedInfo {
    pub(crate) checkout: PathBuf,
    pub(crate) branch: Option<String>,
    pub(crate) commit: Option<String>,
    /// The last release tag reachable, and the commits since it.
    pub(crate) last_tag: Option<String>,
    pub(crate) ahead: Option<u64>,
    /// The build's own version (`olang --version`).
    pub(crate) version: Option<String>,
    /// Why the build is not there, or is behind its source.
    pub(crate) missing: Option<String>,
    pub(crate) stale: Option<String>,
}

impl LinkedInfo {
    /// `1.0.0-rc.8+39e27324 (main, 41 commits past v1.0.0-rc.8)`.
    pub(crate) fn describe(&self) -> String {
        if let Some(m) = &self.missing {
            return format!("missing: {m}");
        }
        let mut v = self.version.clone().unwrap_or_else(|| "?".to_string());
        if let Some(c) = &self.commit {
            v = format!("{v}+{}", &c[..c.len().min(8)]);
        }
        let mut about = Vec::new();
        if let Some(b) = &self.branch {
            about.push(b.clone());
        }
        match (&self.last_tag, self.ahead) {
            (Some(t), Some(0)) => about.push(format!("at {t}")),
            (Some(t), Some(n)) => about.push(format!(
                "{n} commit{} past {t}",
                if n == 1 { "" } else { "s" }
            )),
            _ => {}
        }
        if about.is_empty() {
            v
        } else {
            format!("{v} ({})", about.join(", "))
        }
    }
}

fn git_in(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// A linked build's state: its branch, commit, distance from the last
/// release, its build's version, and whether the build is missing or
/// older than the last commit.
pub(crate) fn linked_info(l: &Linked) -> LinkedInfo {
    let mut info = LinkedInfo {
        checkout: l.checkout.clone(),
        ..Default::default()
    };
    if !l.checkout.is_dir() {
        info.missing = Some(format!(
            "{} is gone (`otc toolchain link <its new place>` relinks it, `otc toolchain unlink <name>` forgets it)",
            l.checkout.display()
        ));
        return info;
    }
    info.branch =
        git_in(&l.checkout, &["rev-parse", "--abbrev-ref", "HEAD"]).filter(|b| b != "HEAD");
    info.commit = git_in(&l.checkout, &["rev-parse", "HEAD"]);
    info.last_tag = git_in(
        &l.checkout,
        &["describe", "--tags", "--abbrev=0", "--match", "v*"],
    );
    if let Some(t) = &info.last_tag {
        info.ahead = git_in(&l.checkout, &["rev-list", "--count", &format!("{t}..HEAD")])
            .and_then(|n| n.parse().ok());
    }
    let bin = l.olang();
    if !bin.is_file() {
        info.missing = Some(format!(
            "{} has no build at {} (`cargo build --release` in the checkout)",
            l.checkout.display(),
            bin.display()
        ));
        return info;
    }
    info.version = version_of(&bin);
    let built = std::fs::metadata(&bin)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    let committed =
        git_in(&l.checkout, &["log", "-1", "--format=%ct"]).and_then(|t| t.parse::<u64>().ok());
    if let (Some(b), Some(c)) = (built, committed)
        && b + 60 < c
    {
        let behind = c - b;
        let span = if behind >= 86_400 {
            format!(
                "{} day{}",
                behind / 86_400,
                if behind / 86_400 == 1 { "" } else { "s" }
            )
        } else if behind >= 3600 {
            format!("{} h", behind / 3600)
        } else {
            format!("{} min", behind / 60)
        };
        info.stale = Some(format!(
            "built {span} before the last commit: `cargo build --release` in {}",
            l.checkout.display()
        ));
    }
    info
}

/// A binary's version: the second word of `olang --version`, given five
/// seconds (a wedged binary is a `?`, never a hung list).
fn version_of(bin: &Path) -> Option<String> {
    let mut child = std::process::Command::new(bin)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(5) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut words = text.split_whitespace();
    match (words.next(), words.next()) {
        (Some("olang"), Some(v)) => Some(v.to_string()),
        _ => None,
    }
}

/// What put an olang outside `~/.olang` where it is.
fn outside_kind(p: &Path) -> &'static str {
    let s = p.to_string_lossy();
    if s.contains("/.cargo/") {
        "cargo"
    } else if s.contains("/Cellar/") || s.contains("/homebrew/") || s.contains("/linuxbrew/") {
        "homebrew"
    } else if s.contains("/target/release/") || s.contains("/target/debug/") || s.contains("/.bin/")
    {
        "checkout"
    } else {
        "path"
    }
}

fn olang_name() -> &'static str {
    if cfg!(windows) { "olang.exe" } else { "olang" }
}

/// Every olang found: PATH order first (so the active one leads), then
/// the shims, then the toolchains newest first.
pub(crate) fn rows(home_bin: &Path, toolchains: &Path) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    let mut add = |path: PathBuf, kind: &'static str, folder: Option<String>, on_path: bool| {
        let real = path.canonicalize().unwrap_or_else(|_| path.clone());
        if let Some(r) = rows.iter_mut().find(|r| r.real == real) {
            r.on_path |= on_path;
            // a toolchain reached through the shims or PATH is that toolchain
            if folder.is_some() && r.folder.is_none() {
                r.kind = kind;
                r.path = path;
                r.folder = folder;
            }
            return;
        }
        rows.push(Row {
            version: if real.is_file() {
                version_of(&real)
            } else {
                None
            },
            kind,
            path,
            real,
            folder,
            active: false,
            default: false,
            on_path,
            linked: None,
        });
    };
    let mut first = true;
    let mut active_real = None;
    for dir in path_dirs() {
        let cand = dir.join(olang_name());
        if !cand.is_file() {
            continue;
        }
        if first {
            active_real = Some(cand.canonicalize().unwrap_or_else(|_| cand.clone()));
            first = false;
        }
        let kind = if dir == home_bin {
            "shims"
        } else {
            outside_kind(&cand)
        };
        add(cand, kind, None, true);
    }
    let shim = home_bin.join(olang_name());
    let default_real = shim
        .is_file()
        .then(|| shim.canonicalize().unwrap_or_else(|_| shim.clone()));
    if shim.is_file() {
        add(shim, "shims", None, false);
    }
    for (folder, bin) in olang::pkg::toolchain::installed_in(toolchains) {
        let linked = std::fs::symlink_metadata(&bin).is_ok_and(|m| m.file_type().is_symlink());
        add(
            bin,
            if linked { "linked" } else { "managed" },
            Some(folder),
            false,
        );
    }
    for (name, l) in &Toolchains::load().linked {
        add(l.olang(), "linked", Some(name.clone()), false);
    }
    for r in &mut rows {
        r.active = active_real.as_ref() == Some(&r.real);
        r.default = default_real.as_ref() == Some(&r.real);
        if r.kind == "linked"
            && let Some(name) = &r.folder
            && let Some(l) = Toolchains::load().linked.get(name)
        {
            r.linked = Some(linked_info(l));
        }
    }
    rows
}

/// Whether version `a` is newer than `b`, as two binaries gave them
/// (semver; false when either does not read as one).
fn is_newer(a: Option<&str>, b: Option<&str>) -> bool {
    let read = |v: Option<&str>| semver::Version::parse(v?.trim_start_matches('v')).ok();
    matches!((read(a), read(b)), (Some(a), Some(b)) if a > b)
}

/// The line that says the active olang shadows the default, and the fix.
///
/// The fix never costs the machine its newest olang (OL-638). When the
/// shadowing one is the newer, the fix brings the default up to it.
/// Otherwise the fix puts the shims first, and removing the shadowing
/// one is offered only when the default would then run (the shims are
/// on PATH), with how to get olang back: `cargo install olang` installs
/// another project's crate of that name from crates.io.
pub(crate) fn shadow_note(rows: &[Row], home_bin: &Path) -> Option<(String, String)> {
    let default = rows.iter().find(|r| r.default)?;
    let fix = format!("export PATH=\"{}:$PATH\"", home_bin.display());
    let shown = |r: &Row| r.version.clone().unwrap_or_else(|| "?".to_string());
    match rows.iter().find(|r| r.active) {
        Some(active) if !active.default => {
            let why = format!(
                "{} ({}, {}) is ahead of the shims on PATH, so `olang` runs it, not the default {}",
                active.path.display(),
                active.kind,
                shown(active),
                shown(default)
            );
            if is_newer(active.version.as_deref(), default.version.as_deref()) {
                return Some((
                    why,
                    "otc update (it is the newer olang, so keep it: this installs the latest \
                     release and makes it the default)"
                        .to_string(),
                ));
            }
            let remove = match active.kind {
                _ if !default.on_path => String::new(),
                "cargo" => format!(
                    " (or remove it: cargo uninstall olang; the default {} then runs. To get \
                     olang back later use the install script, or `cargo install --path .` in a \
                     checkout. `cargo install olang` is not olang: on crates.io the name is \
                     another project's)",
                    shown(default)
                ),
                "homebrew" => format!(
                    " (or remove it: brew uninstall olang; the default {} then runs)",
                    shown(default)
                ),
                _ => String::new(),
            };
            Some((why, format!("{fix}{remove}")))
        }
        Some(_) => None,
        None => Some((
            format!(
                "the shims ({}) are not on PATH, so `olang` does not run the default {}",
                home_bin.display(),
                shown(default)
            ),
            fix,
        )),
    }
}

/// What the project here pins, and the olang that answers it.
pub(crate) fn project_note() -> Option<serde_json::Value> {
    let cwd = std::env::current_dir().ok()?;
    let root = olang::pkg::manifest::Manifest::find_root(&cwd)?;
    use olang::pkg::toolchain::{Resolved, for_project, install_hint};
    let (pin, version, olang, missing) = match for_project(&root) {
        Resolved::Unpinned => (None, None, None, None),
        Resolved::Installed { pin, version, bin } => (
            Some(pin),
            Some(version),
            Some(bin.display().to_string()),
            None,
        ),
        Resolved::Running { pin } => (Some(pin), Some(olang::VERSION.to_string()), None, None),
        Resolved::Missing { pin } => {
            let hint = install_hint(&pin);
            (Some(pin), None, None, Some(hint))
        }
    };
    Some(serde_json::json!({
        "root": root.display().to_string(), "pin": pin, "version": version,
        "olang": olang, "install": missing,
    }))
}

fn row_json(r: &Row) -> serde_json::Value {
    serde_json::json!({
        "version": r.version, "olang": r.path.display().to_string(),
        "real": r.real.display().to_string(), "kind": r.kind, "folder": r.folder,
        "active": r.active, "default": r.default, "on_path": r.on_path,
        "linked": r.linked.as_ref().map(|l| serde_json::json!({
            "checkout": l.checkout.display().to_string(), "branch": l.branch, "commit": l.commit,
            "last_tag": l.last_tag, "ahead": l.ahead, "missing": l.missing, "stale": l.stale,
            "describe": l.describe(),
        })),
    })
}

/// `otc toolchain list [--available [--stable] [--offline]]`.
pub fn list(available: bool, stable: bool, offline: bool) -> Result<()> {
    let home_bin = olang::home::bin().context("no home directory could be determined")?;
    let toolchains = olang::home::toolchains().context("no home directory could be determined")?;
    let rows = rows(&home_bin, &toolchains);
    let shadow = shadow_note(&rows, &home_bin);
    let project = project_note();
    let default_name = super::update::default_toolchain()?;
    let listing = if available {
        let how = if offline {
            super::releases::Fetch::Offline
        } else {
            super::releases::Fetch::Cached
        };
        Some(super::releases::list(how)?)
    } else {
        None
    };
    let active_version = rows
        .iter()
        .find(|r| r.active)
        .and_then(|r| r.version.clone());
    let installed: Vec<String> = olang::pkg::toolchain::installed()
        .into_iter()
        .map(|(v, _)| v)
        .collect();
    let releases: Vec<serde_json::Value> = listing
        .as_ref()
        .map(|l| {
            l.releases
                .iter()
                .filter(|r| !stable || !r.prerelease)
                .map(|r| {
                    let newer = active_version.as_deref().is_some_and(|a| {
                        super::releases::version_cmp(&r.version, a) == std::cmp::Ordering::Greater
                    });
                    serde_json::json!({
                        "version": r.version, "date": r.date, "prerelease": r.prerelease,
                        "installed": installed.contains(&r.version),
                        "default": default_name.as_deref() == Some(r.version.as_str()),
                        "active": active_version.as_deref() == Some(r.version.as_str()),
                        "newer": newer,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if ctx().json {
        // `#{ format: 1, kind: "toolchains", dir, default, active, shadowed,
        // fix, project, toolchains, available, available_age_s, available_note }`
        emit(serde_json::json!({
            "format": 1,
            "kind": "toolchains",
            "dir": toolchains.display().to_string(),
            "default": default_name,
            "active": rows.iter().find(|r| r.active).map(row_json),
            "shadowed": shadow.as_ref().map(|(s, _)| s),
            "fix": shadow.as_ref().map(|(_, f)| f),
            "project": project,
            "toolchains": rows.iter().map(row_json).collect::<Vec<_>>(),
            "available": listing.as_ref().map(|_| releases.clone()),
            "available_age_s": listing.as_ref().map(|l| l.age_s),
            "available_note": listing.as_ref().and_then(|l| l.note.clone()),
        }));
        return Ok(());
    }
    if rows.is_empty() {
        println!(
            "no olang found: none on PATH, none under {}",
            toolchains.display()
        );
        println!("next: otc use latest (installs the newest release)");
    }
    let width = rows
        .iter()
        .map(|r| r.version.as_deref().unwrap_or("?").len())
        .max()
        .unwrap_or(1);
    if !rows.is_empty() && listing.is_some() {
        println!("on this machine:");
    }
    for r in &rows {
        let mut marks = Vec::new();
        if r.active {
            marks.push("active");
        }
        if r.default {
            marks.push("default");
        }
        let folder = match (&r.folder, &r.version, r.kind) {
            (Some(f), _, "linked") => format!(" ({f})"),
            (Some(f), Some(v), _) if f != v => format!(" (folder {f})"),
            (Some(f), None, _) => format!(" (folder {f})"),
            _ => String::new(),
        };
        println!(
            "  {} {:<width$}  {:<8}  {:<15} {}{}",
            if r.active { "→" } else { " " },
            r.version.as_deref().unwrap_or("?"),
            r.kind,
            marks.join(", "),
            r.path.display(),
            folder,
        );
        if let Some(l) = &r.linked {
            println!("      {}", l.describe());
            if let Some(stale) = &l.stale {
                println!("      {stale}");
            }
        }
    }
    if let Some((why, fix)) = &shadow {
        println!("{why}: {fix}");
    }
    if let Some(p) = &project {
        let root = p["root"].as_str().unwrap_or("");
        match (
            p["pin"].as_str(),
            p["version"].as_str(),
            p["install"].as_str(),
        ) {
            (None, _, _) => println!("this project ({root}) pins no olang: it runs the active one"),
            (Some(pin), Some(v), _) => {
                let at = p["olang"]
                    .as_str()
                    .map(|b| format!(" ({b})"))
                    .unwrap_or_default();
                println!("this project ({root}) pins olang {pin}: it runs {v}{at}")
            }
            (Some(pin), None, Some(hint)) => println!(
                "this project ({root}) pins olang {pin}: no olang here satisfies it ({hint} installs one)"
            ),
            _ => {}
        }
    }
    if let Some(l) = &listing {
        let when = if l.cached {
            format!(", as fetched {}", super::releases::age(l.age_s))
        } else {
            String::new()
        };
        println!();
        println!("released ({} on GitHub{when}):", releases.len());
        if let Some(note) = &l.note {
            println!("  ({note})");
        }
        for r in &releases {
            let mut marks = Vec::new();
            if r["active"].as_bool() == Some(true) {
                marks.push("active");
            }
            if r["default"].as_bool() == Some(true) {
                marks.push("default");
            }
            if r["installed"].as_bool() == Some(true) {
                marks.push("installed");
            } else if r["newer"].as_bool() == Some(true) {
                marks.push("newer");
            }
            println!(
                "  {:<14} {:<10}  {:<10}  {}",
                r["version"].as_str().unwrap_or(""),
                r["date"].as_str().unwrap_or(""),
                if r["prerelease"].as_bool() == Some(true) {
                    "prerelease"
                } else {
                    "stable"
                },
                marks.join(", ")
            );
        }
        println!("`otc use <version>` installs one and switches to it");
    }
    Ok(())
}

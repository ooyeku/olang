//! `otc toolchain list` (and a bare `otc toolchain`): every olang this
//! machine has, which one runs and why (OL-577).
//!
//! An olang is found three ways: a managed toolchain or a linked local
//! build under `~/.olang/toolchains`, the shims in `~/.olang/bin` (the
//! default), and every `olang` on PATH (cargo, Homebrew, a checkout's
//! build, anything else). Each binary is one row however many ways reach
//! it, and its version is the binary's own answer to `--version`, never a
//! folder's name. The **active** olang is the first on PATH, the one a
//! bare `olang` runs; the **default** is the shims'. When they differ,
//! one line says which shadows which and the command that fixes it (the
//! newer of the two is never the one to remove, OL-638); in a project,
//! one more says what its pin resolves to.

use super::doctor::path_dirs;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// One olang binary on this machine.
struct Row {
    /// What the binary says it is (`olang --version`), `None` when it
    /// does not answer.
    version: Option<String>,
    /// `managed`, `linked`, `shims`, `cargo`, `homebrew`, `checkout`, `path`.
    kind: &'static str,
    /// Where it was found (a toolchain's `bin/olang`, a PATH entry's).
    path: PathBuf,
    /// The file itself, every link followed: one row per binary.
    real: PathBuf,
    /// The toolchain folder's name, for a managed or linked one.
    folder: Option<String>,
    active: bool,
    default: bool,
    on_path: bool,
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
fn rows(home_bin: &Path, toolchains: &Path) -> Vec<Row> {
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
            version: version_of(&real),
            kind,
            path,
            real,
            folder,
            active: false,
            default: false,
            on_path,
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
    for r in &mut rows {
        r.active = active_real.as_ref() == Some(&r.real);
        r.default = default_real.as_ref() == Some(&r.real);
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
fn shadow_note(rows: &[Row], home_bin: &Path) -> Option<(String, String)> {
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
fn project_note() -> Option<serde_json::Value> {
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

pub fn list(json: bool, default_folder: Option<String>) -> Result<()> {
    let home_bin = olang::home::bin().context("no home directory could be determined")?;
    let toolchains = olang::home::toolchains().context("no home directory could be determined")?;
    let rows = rows(&home_bin, &toolchains);
    let shadow = shadow_note(&rows, &home_bin);
    let project = project_note();
    if json {
        // `#{ format: 1, kind: "toolchains", dir, default, active, shadowed,
        // fix, project, toolchains }`, each row `#{ version, olang, kind,
        // folder, active, default, on_path }`
        let row_json = |r: &Row| {
            serde_json::json!({
                "version": r.version, "olang": r.path.display().to_string(),
                "real": r.real.display().to_string(), "kind": r.kind, "folder": r.folder,
                "active": r.active, "default": r.default, "on_path": r.on_path,
            })
        };
        println!(
            "{}",
            serde_json::json!({
                "format": 1,
                "kind": "toolchains",
                "dir": toolchains.display().to_string(),
                "default": default_folder,
                "active": rows.iter().find(|r| r.active).map(row_json),
                "shadowed": shadow.as_ref().map(|(s, _)| s),
                "fix": shadow.as_ref().map(|(_, f)| f),
                "project": project,
                "toolchains": rows.iter().map(row_json).collect::<Vec<_>>(),
            })
        );
        return Ok(());
    }
    if rows.is_empty() {
        println!(
            "no olang found: none on PATH, none under {}",
            toolchains.display()
        );
        println!("(`otc update` installs the latest release)");
        return Ok(());
    }
    let width = rows
        .iter()
        .map(|r| r.version.as_deref().unwrap_or("?").len())
        .max()
        .unwrap_or(1);
    for r in &rows {
        let mut marks = Vec::new();
        if r.active {
            marks.push("active");
        }
        if r.default {
            marks.push("default");
        }
        let folder = match (&r.folder, &r.version) {
            (Some(f), Some(v)) if f != v => format!(" (folder {f})"),
            (Some(f), None) => format!(" (folder {f})"),
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
    Ok(())
}

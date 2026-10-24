//! Developer tooling shipped inside the `olang` binary: the test runner
//! (`olang test`), the formatter (`olang fmt`), and the static checker
//! (`olang check`).

pub mod allow; // `// allow: <class>` — one line's advisories silenced
pub mod bench; // `olang bench` — reproducible timings with baseline compare
pub mod check; // `olang check` — provable annotation violations, pre-run
pub mod coverage; // `olang test --coverage` — line coverage from the runner
pub mod doc; // `olang doc` — API reference from /// comments
#[cfg(feature = "native")]
pub mod doctor; // `olang doctor` — audit a project
pub mod fields; // what completes inside data: a Frame's columns, a map's keys
pub mod fix; // `olang fix maps` — the migration to `.` and `??`
pub mod fix_idioms; // `olang fix guards / defaults / first-last / units` — the 1.0 idioms
pub mod fmt;
pub mod keys; // the checker's key analysis: a `.key` read no construction has
#[cfg(feature = "native")]
pub mod lsp; // `olang lsp` — the language server over stdio
#[cfg(feature = "native")]
pub mod rename_across; // a shared name renamed in every file that imports it
#[cfg(feature = "native")]
pub mod scope_refs; // a name's places by its binding: references and rename
pub mod semantic; // the language server's semantic tokens
pub mod test_events; // `olang test --format json` — the runner's events for an editor
pub mod test_runner;

use std::path::{Path, PathBuf};

/// Every program olang reads at or below `path`: `.ol` files and `.olmd`
/// notebooks, sorted, skipping what `discover_ol_files` skips. A file
/// path returns just itself. What `olang test`, `check` and `fmt` walk.
pub fn discover_sources(path: &Path) -> Vec<PathBuf> {
    discover(path, true)
}

/// Every `.ol` file at or below `path`, sorted. A walk keeps what a
/// checkout keeps: the `.gitignore`s (at every level, and above `path`)
/// and `.git/info/exclude` respected, a repository or not; hidden folders
/// and the build and cache folders (`SKIPPED_DIRS`) left out even with no
/// `.gitignore`. Inside a project, a folder below `path` with an
/// `olang.toml` of its own is another project (a fixture, a vendored
/// package) and is left out too: it is walked when named itself. A file
/// path returns just itself. Public: the CLI's watch
/// mode polls it from the binary crate.
pub fn discover_ol_files(path: &Path) -> Vec<PathBuf> {
    discover(path, false)
}

/// Folders a walk never enters: build output, dependencies, caches.
pub const SKIPPED_DIRS: &[&str] = &["target", "node_modules", "dist"];

fn wanted(name: &str, notebooks: bool) -> bool {
    name.ends_with(".ol") || (notebooks && name.ends_with(".olmd"))
}

fn skipped_dir(name: &str) -> bool {
    name.starts_with('.') || SKIPPED_DIRS.contains(&name)
}

/// Whether `path` is in a project: it, or a folder above it, has an
/// `olang.toml`. Only then is a nested `olang.toml` another project's.
fn in_project(path: &Path) -> bool {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    abs.ancestors().any(|d| d.join("olang.toml").is_file())
}

/// A folder below the walk's start that is a project of its own.
fn own_project(dir: &Path) -> bool {
    dir.join("olang.toml").is_file()
}

#[cfg(feature = "native")]
fn discover(path: &Path, notebooks: bool) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path.to_path_buf()];
    }
    let nested = in_project(path);
    let mut b = ignore::WalkBuilder::new(path);
    b.hidden(false)
        .parents(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .ignore(false)
        .require_git(false)
        .follow_links(true)
        .filter_entry(move |e| {
            e.depth() == 0
                || !e.file_type().is_some_and(|t| t.is_dir())
                || !(skipped_dir(&e.file_name().to_string_lossy())
                    || (nested && own_project(e.path())))
        });
    let mut found: Vec<PathBuf> = b
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| wanted(&e.file_name().to_string_lossy(), notebooks))
        .map(|e| e.into_path())
        .collect();
    found.sort();
    found
}

#[cfg(not(feature = "native"))]
fn discover(path: &Path, notebooks: bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if path.is_file() {
        found.push(path.to_path_buf());
        return found;
    }
    let nested = in_project(path);
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                if !skipped_dir(&name) && !(nested && own_project(&p)) {
                    stack.push(p);
                }
            } else if wanted(&name, notebooks) {
                found.push(p);
            }
        }
    }
    found.sort();
    found
}

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

/// Every `.ol` file at or below `path`, sorted, skipping VCS/build dirs and
/// hidden directories. A file path returns just itself. Public: the CLI's
/// watch mode polls it from the binary crate.
pub fn discover_ol_files(path: &Path) -> Vec<PathBuf> {
    discover(path, false)
}

fn discover(path: &Path, notebooks: bool) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if path.is_file() {
        found.push(path.to_path_buf());
        return found;
    }
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
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    stack.push(p);
                }
            } else if name.ends_with(".ol") || (notebooks && name.ends_with(".olmd")) {
                found.push(p);
            }
        }
    }
    found.sort();
    found
}

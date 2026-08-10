//! Developer tooling shipped inside the `olang` binary: the test runner
//! (`olang test`) and the formatter (`olang fmt`).

pub mod fmt;
#[cfg(feature = "native")]
pub mod lsp; // `olang lsp` — the language server over stdio
pub mod test_runner;

use std::path::{Path, PathBuf};

/// Every `.ol` file at or below `path`, sorted, skipping VCS/build dirs and
/// hidden directories. A file path returns just itself.
pub(crate) fn discover_ol_files(path: &Path) -> Vec<PathBuf> {
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
            } else if name.ends_with(".ol") {
                found.push(p);
            }
        }
    }
    found.sort();
    found
}

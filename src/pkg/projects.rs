//! The projects otc has installed for: `~/.olang/state/projects.json`.
//!
//! What lets `otc` answer "what uses this" for a toolchain or a shelf
//! copy: an explicit install (`otc install`, `add`, `remove`, `upgrade`,
//! `use --project`) records the project's root. A project whose
//! `olang.toml` is gone drops out when the list is next read. Regenerable
//! (`otc clean --state` forgets it; the next install in each project
//! records it again).

use std::path::{Path, PathBuf};

fn file() -> Option<PathBuf> {
    crate::home::projects()
}

fn read() -> Vec<PathBuf> {
    let Some(f) = file() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(f) else {
        return Vec::new();
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("projects").cloned())
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

fn write(roots: &[PathBuf]) {
    let Some(f) = file() else { return };
    if let Some(parent) = f.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let list: Vec<String> = roots.iter().map(|p| p.display().to_string()).collect();
    let text = serde_json::json!({ "format": 1, "projects": list }).to_string();
    let staged = f.with_extension("json.new");
    if std::fs::write(&staged, text).is_ok() {
        let _ = std::fs::rename(&staged, &f);
    }
}

/// Record `root` as a known project.
pub fn record(root: &Path) {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut roots = read();
    if roots.contains(&root) {
        return;
    }
    roots.push(root);
    roots.retain(|r| r.join("olang.toml").is_file());
    roots.sort();
    write(&roots);
}

/// The known projects that are still there, sorted.
pub fn known() -> Vec<PathBuf> {
    let all = read();
    let kept: Vec<PathBuf> = all
        .iter()
        .filter(|r| r.join("olang.toml").is_file())
        .cloned()
        .collect();
    if kept.len() != all.len() {
        write(&kept);
    }
    kept
}

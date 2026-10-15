//! The project manifest, `olang.toml`.
//!
//! A manifest declares the package's identity and its dependencies. Because
//! olang packages are source (no build step, no ABI), a dependency is just a
//! pointer to a directory of `.ol` files: a local path, a git repository, or
//! a named registry version.

use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The parsed `olang.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub package: PackageMeta,
    #[serde(default)]
    pub dependencies: BTreeMap<String, Dependency>,
    /// The `[capabilities]` block: what this package (and, attenuated,
    /// each dependency) is allowed to touch. Absent = full capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<crate::caps::CapsConfig>,
    /// The `[check]` block: how `olang check` treats this project's
    /// advisories. `promote = ["exhaustiveness", "shape"]` turns those
    /// warning classes into errors that fail the check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<CheckConfig>,
}

/// `[check]` in olang.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CheckConfig {
    /// Warning classes promoted to errors: `exhaustiveness` (a `match`
    /// over an enum or a literal union missing a case), `shape` (a
    /// literal key a declared record shape does not carry), `result` (a
    /// discarded `Result`), `all`.
    #[serde(default)]
    pub promote: Vec<String>,
    /// Functions pinned to native code: a name, a `*` pattern, or
    /// `path:pattern` (the path relative to the project's root). `olang
    /// check --tier` fails when the tier refuses one, and a run keeping
    /// tier statistics names one that falls back (src/tier_stats.rs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageMeta {
    pub name: String,
    pub version: Version,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    /// Files and directories (relative to the package) an application
    /// built with `olang build --app` carries for `asset.read`: fonts,
    /// images, data.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
    /// The oldest olang this package runs on, as `rust-version` is Rust's:
    /// `"0.88"` is 0.88.0 or newer (1.x included), `"1"` 1.0.0 or newer.
    /// `otc install` records the toolchain that resolved it in olang.lock;
    /// an editor runs that toolchain for the project; an older olang
    /// running it warns (src/pkg/toolchain.rs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub olang: Option<String>,
    /// Who may write this project's `olang.lock`: `"auto"` (the default)
    /// lets any install refresh it; `"never"` keeps every incidental
    /// install (a run, `olang test`, the REPL, the language server, a
    /// check) from creating or rewriting it — the existing lock is still
    /// replayed, and without one resolution happens in memory. Only an
    /// explicit `otc install` / `add` / `remove` writes it. For test
    /// fixtures whose committed lock must not drift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<String>,
}

/// The sections olang reads from olang.toml.
pub const TOP_SECTIONS: &[&str] = &["package", "dependencies", "capabilities", "check"];

/// The keys `[package]` takes.
pub const PACKAGE_KEYS: &[&str] = &[
    "name",
    "version",
    "description",
    "authors",
    "license",
    "assets",
    "olang",
    "lock",
];

/// The keys `[check]` takes.
pub const CHECK_KEYS: &[&str] = &["promote", "native"];

/// The advisory classes `[check] promote` names, each a stable name for
/// one kind of finding. `all` is every one of these: the classes the
/// running olang knows, so a later olang's new class is promoted by `all`
/// only once the project runs on that olang. `unused` (a name nothing
/// reads) is a hint's class, never a finding, and is not promoted.
pub const PROMOTE_CLASSES: &[&str] = &[
    "exhaustiveness",
    "shape",
    "keys",
    "result",
    "copy",
    "shadow",
    "tier",
];

/// The known name `word` is a likely misspelling of: case aside, at most
/// two edits away, or the singular or plural of it.
fn near_miss<'a>(word: &str, known: &[&'a str]) -> Option<&'a str> {
    let w = word.to_ascii_lowercase();
    known.iter().copied().find(|k| {
        let k = k.to_ascii_lowercase();
        w == k
            || crate::interpreter::IntuitiveErrorFormatter::levenshtein_distance(&w, &k) <= 2
            || (w.len() >= 4 && (k.starts_with(&w) || w.starts_with(&k)))
            || k.trim_end_matches("ies") == w.trim_end_matches('y')
    })
}

/// The values `[package] lock` accepts.
pub const LOCK_VALUES: &[&str] = &["auto", "never"];

/// A dependency's source. `toml` distinguishes these by which key is present,
/// so the enum is untagged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Dependency {
    /// `dep = "1.2.0"` — a registry dependency by version requirement.
    Registry(String),
    /// `dep = { path = "../thing" }` — a local directory.
    Path { path: String },
    /// `dep = { git = "url", tag/rev/branch = "..." }` — a git repository.
    Git {
        git: String,
        #[serde(default)]
        tag: Option<String>,
        #[serde(default)]
        rev: Option<String>,
        #[serde(default)]
        branch: Option<String>,
    },
    /// `dep = { version = "1.2", registry = "..." }` — explicit registry form.
    RegistryExplicit {
        version: String,
        #[serde(default)]
        registry: Option<String>,
    },
    /// `dep = { shelf = "my-lib" }` — a library registered on the user's
    /// local shelf (`otc lib add`). The manifest records only the name,
    /// which keeps it free of machine-specific paths; the shelf supplies
    /// the directory at install time and the lockfile pins what resolved.
    Shelf { shelf: String },
}

impl Dependency {
    /// The version requirement, for registry dependencies (used by the
    /// resolver). Path and git deps resolve without version selection.
    pub fn version_req(&self) -> Option<&str> {
        match self {
            Dependency::Registry(v) => Some(v),
            Dependency::RegistryExplicit { version, .. } => Some(version),
            _ => None,
        }
    }

    pub fn is_registry(&self) -> bool {
        matches!(
            self,
            Dependency::Registry(_) | Dependency::RegistryExplicit { .. }
        )
    }
}

/// The keys a capability table takes; `dependencies` only at the top.
const CAPABILITY_KEYS: &[&str] = &["fs", "net", "proc", "db", "env"];

fn check_capability_keys(caps: &toml::Value) -> Result<(), ManifestError> {
    let unknown = |table: &toml::value::Table, at: &str, deps: bool| {
        for key in table.keys() {
            if !(CAPABILITY_KEYS.contains(&key.as_str()) || (deps && key == "dependencies")) {
                return Err(ManifestError::Parse(format!(
                    "{} has no capability '{}' (known: {}); an unknown capability is refused, never ignored",
                    at,
                    key,
                    CAPABILITY_KEYS.join(", ")
                )));
            }
        }
        Ok(())
    };
    let toml::Value::Table(t) = caps else {
        return Err(ManifestError::Parse(
            "[capabilities] must be a table".into(),
        ));
    };
    unknown(t, "[capabilities]", true)?;
    if let Some(deps) = t.get("dependencies") {
        let toml::Value::Table(deps) = deps else {
            return Err(ManifestError::Parse(
                "[capabilities.dependencies] must be a table".into(),
            ));
        };
        for (name, spec) in deps {
            let toml::Value::Table(spec) = spec else {
                return Err(ManifestError::Parse(format!(
                    "[capabilities.dependencies.{}] must be a table",
                    name
                )));
            };
            unknown(
                spec,
                &format!("[capabilities.dependencies.{}]", name),
                false,
            )?;
        }
    }
    Ok(())
}

/// Errors reading or writing a manifest.
#[derive(Debug)]
pub enum ManifestError {
    Io(std::io::Error),
    Parse(String),
    NotFound(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(e) => write!(f, "{}", e),
            ManifestError::Parse(e) => write!(f, "invalid olang.toml: {}", e),
            ManifestError::NotFound(p) => write!(f, "no olang.toml found at {}", p),
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    /// Parse a manifest from TOML text. A key `[capabilities]` does not
    /// know is refused, never ignored: a misspelled or newer capability
    /// must not leave its grant wide open.
    pub fn from_toml(text: &str) -> Result<Self, ManifestError> {
        if let Ok(toml::Value::Table(t)) = toml::from_str::<toml::Value>(text)
            && let Some(caps) = t.get("capabilities")
        {
            check_capability_keys(caps)?;
        }
        toml::from_str(text).map_err(|e| ManifestError::Parse(e.to_string()))
    }

    /// Whether manifest text declares a `[capabilities]` block — read
    /// even when the text is not a valid manifest, so a broken one that
    /// meant to restrict is refused rather than run with no grant at all.
    pub fn declares_capabilities(text: &str) -> bool {
        match toml::from_str::<toml::Value>(text) {
            Ok(toml::Value::Table(t)) => t.contains_key("capabilities"),
            _ => text.contains("capabilities"),
        }
    }

    /// `load`, refusing a manifest that declares `[capabilities]` but
    /// cannot be read: `Ok(None)` when there is no usable manifest and
    /// nothing was restricted, `Err` when a restriction was asked for and
    /// cannot be honoured.
    pub fn load_for_capabilities(dir: &Path) -> Result<Option<Self>, ManifestError> {
        match Self::load(dir) {
            Ok(m) => Ok(Some(m)),
            Err(ManifestError::NotFound(_)) => Ok(None),
            Err(e) => {
                let text = crate::vfs::read_to_string(&dir.join("olang.toml")).unwrap_or_default();
                if Self::declares_capabilities(&text) {
                    Err(e)
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Load `olang.toml` from a directory.
    pub fn load(dir: &Path) -> Result<Self, ManifestError> {
        let path = dir.join("olang.toml");
        if !crate::vfs::exists(&path) {
            return Err(ManifestError::NotFound(path.display().to_string()));
        }
        let text = crate::vfs::read_to_string(&path).map_err(ManifestError::Io)?;
        Self::from_toml(&text)
    }

    /// Serialize back to TOML (used by `otc add`/`init`).
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }

    /// Write `olang.toml` into a directory.
    pub fn save(&self, dir: &Path) -> Result<(), ManifestError> {
        std::fs::write(dir.join("olang.toml"), self.to_toml()).map_err(ManifestError::Io)
    }

    /// Set (or replace) the dependency `name` in `dir/olang.toml`, editing
    /// the file in place: its comments, its order and any keys olang does
    /// not know stay as they were.
    pub fn write_dependency(dir: &Path, name: &str, dep: &Dependency) -> Result<(), ManifestError> {
        Self::edit(dir, |doc| {
            let deps = doc
                .entry("dependencies")
                .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
            let Some(table) = deps.as_table_like_mut() else {
                return Err("[dependencies] is not a table".to_string());
            };
            table.insert(name, toml_edit::Item::Value(dependency_value(dep)?));
            Ok(())
        })
    }

    /// Remove the dependency `name` from `dir/olang.toml`, in place.
    pub fn remove_dependency(dir: &Path, name: &str) -> Result<(), ManifestError> {
        Self::edit(dir, |doc| {
            if let Some(table) = doc
                .get_mut("dependencies")
                .and_then(|d| d.as_table_like_mut())
            {
                table.remove(name);
            }
            Ok(())
        })
    }

    fn edit(
        dir: &Path,
        change: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<(), String>,
    ) -> Result<(), ManifestError> {
        let path = dir.join("olang.toml");
        let text = std::fs::read_to_string(&path).map_err(ManifestError::Io)?;
        let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
            ManifestError::Io(std::io::Error::other(e.to_string()))
        })?;
        change(&mut doc).map_err(|e| ManifestError::Io(std::io::Error::other(e)))?;
        std::fs::write(&path, doc.to_string()).map_err(ManifestError::Io)
    }

    /// `[package] lock = "never"`: no incidental install writes the lock.
    /// An unknown value reads as `"auto"` (`olang check` warns about it).
    pub fn lock_never(&self) -> bool {
        self.package.lock.as_deref() == Some("never")
    }

    /// A `[package] olang` that is not a version, with the message
    /// `olang check` reports.
    pub fn olang_value_problem(&self) -> Option<String> {
        let v = self.package.olang.as_deref()?;
        if crate::pkg::toolchain::minimum(v).is_some() {
            return None;
        }
        Some(format!(
            "[package] olang = \"{}\" is not a version: it names the oldest olang the package runs on, as \"0.88\" or \"1.0.2\"",
            v
        ))
    }

    /// A `[package] lock` value olang does not know, with the message
    /// `olang check` warns with.
    pub fn lock_value_problem(&self) -> Option<String> {
        let v = self.package.lock.as_deref()?;
        if LOCK_VALUES.contains(&v) {
            return None;
        }
        Some(format!(
            "[package] lock = \"{}\" is not a lock policy (expected \"auto\" or \"never\"); treated as \"auto\"",
            v
        ))
    }

    /// The keys and sections of manifest text that olang does not read,
    /// each with the warning `olang check` and `olang doctor` give: a key
    /// in `[package]`, `[check]` or a dependency entry olang does not know
    /// (a misspelling is otherwise silently ignored), and a top-level
    /// section that is a near miss of one olang reads (`[dependency]`,
    /// `[Package]`). Other top-level sections belong to other tools —
    /// Loom's bundler reads `[bundle]`, Studio `[plugin]` — and pass.
    /// `[capabilities]` is not here: a key it does not know is refused.
    pub fn unknown_keys(text: &str) -> Vec<String> {
        let Ok(toml::Value::Table(top)) = toml::from_str::<toml::Value>(text) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let unknown = |section: &str,
                       table: &toml::Table,
                       known: &[&str],
                       out: &mut Vec<String>| {
            for key in table.keys() {
                if !known.contains(&key.as_str()) {
                    let hint = near_miss(key, known)
                        .map(|k| format!(" (did you mean `{k}`?)"))
                        .unwrap_or_default();
                    out.push(format!(
                        "{section} has a key olang does not read: `{key}`{hint}; expected one of {}",
                        known.join(", ")
                    ));
                }
            }
        };
        for (name, value) in &top {
            if TOP_SECTIONS.contains(&name.as_str()) {
                continue;
            }
            if let Some(k) = near_miss(name, TOP_SECTIONS) {
                out.push(format!(
                    "`[{name}]` is not a section olang reads (did you mean `[{k}]`?)"
                ));
            } else if !value.is_table() && !value.is_array() {
                out.push(format!(
                    "`{name}` is not a top-level key olang reads; keys belong in a section such as [package]"
                ));
            }
        }
        if let Some(toml::Value::Table(t)) = top.get("package") {
            unknown("[package]", t, PACKAGE_KEYS, &mut out);
        }
        if let Some(toml::Value::Table(t)) = top.get("check") {
            unknown("[check]", t, CHECK_KEYS, &mut out);
        }
        if let Some(toml::Value::Table(deps)) = top.get("dependencies") {
            for (name, dep) in deps {
                let toml::Value::Table(t) = dep else { continue };
                let known: &[&str] = if t.contains_key("path") {
                    &["path"]
                } else if t.contains_key("git") {
                    &["git", "tag", "rev", "branch"]
                } else if t.contains_key("shelf") {
                    &["shelf"]
                } else if t.contains_key("version") {
                    &["version", "registry"]
                } else {
                    &["path", "git", "shelf", "version"]
                };
                unknown(&format!("dependency `{name}`"), t, known, &mut out);
            }
        }
        out
    }

    /// Every warning olang gives about the manifest in `dir` — an unknown
    /// `[package] lock` value, keys and sections olang does not read, and
    /// `[check] promote` names it does not know — as `olang check` and
    /// `olang doctor` print them. Empty when there is no readable manifest.
    pub fn warnings(dir: &Path) -> Vec<String> {
        let Ok(text) = crate::vfs::read_to_string(&dir.join("olang.toml")) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Ok(m) = Self::from_toml(&text) {
            out.extend(m.lock_value_problem());
            out.extend(m.promote_problems());
        }
        out.extend(Self::unknown_keys(&text));
        out
    }

    /// `[check] promote` names that are not a class this olang knows.
    pub fn promote_problems(&self) -> Vec<String> {
        let Some(check) = &self.check else {
            return Vec::new();
        };
        check
            .promote
            .iter()
            .filter(|p| p.as_str() != "all" && !PROMOTE_CLASSES.contains(&p.as_str()))
            .map(|p| {
                let hint = near_miss(p, PROMOTE_CLASSES)
                    .map(|k| format!(" (did you mean `{k}`?)"))
                    .unwrap_or_default();
                format!(
                    "[check] promote names `{p}`, which is not a class this olang knows{hint}; the classes are {}, and `all`",
                    PROMOTE_CLASSES.join(", ")
                )
            })
            .collect()
    }

    /// Walk upward from `start` to find the nearest directory containing an
    /// `olang.toml`, returning that directory (the project root).
    pub fn find_root(start: &Path) -> Option<std::path::PathBuf> {
        let mut dir = if crate::vfs::is_file(start) {
            start.parent()?.to_path_buf()
        } else {
            start.to_path_buf()
        };
        loop {
            if crate::vfs::exists(&dir.join("olang.toml")) {
                return Some(dir);
            }
            if !dir.pop() {
                return None;
            }
        }
    }
}

/// A dependency as the value a manifest writes for it: a version string,
/// or an inline table (`{ path = "../lib" }`).
fn dependency_value(dep: &Dependency) -> Result<toml_edit::Value, String> {
    let as_toml = toml::Value::try_from(dep).map_err(|e| e.to_string())?;
    fn convert(v: &toml::Value) -> toml_edit::Value {
        match v {
            toml::Value::String(s) => s.as_str().into(),
            toml::Value::Integer(i) => (*i).into(),
            toml::Value::Float(f) => (*f).into(),
            toml::Value::Boolean(b) => (*b).into(),
            toml::Value::Array(xs) => {
                let mut a = toml_edit::Array::new();
                for x in xs {
                    a.push(convert(x));
                }
                toml_edit::Value::Array(a)
            }
            toml::Value::Table(t) => {
                let mut it = toml_edit::InlineTable::new();
                for (k, x) in t {
                    it.insert(k, convert(x));
                }
                toml_edit::Value::InlineTable(it)
            }
            toml::Value::Datetime(d) => d.to_string().into(),
        }
    }
    Ok(convert(&as_toml))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_dependency_kinds() {
        let text = r#"
[package]
name = "app"
version = "0.2.0"
description = "a test app"

[dependencies]
alpha = "1.4.0"
beta = { path = "../beta" }
epsilon = { shelf = "epsilon" }
gamma = { git = "https://example.com/gamma", tag = "v2.0.0" }
delta = { git = "https://example.com/delta", rev = "abc123" }
"#;
        let m = Manifest::from_toml(text).unwrap();
        assert_eq!(m.package.name, "app");
        assert_eq!(m.package.version, Version::new(0, 2, 0));
        assert_eq!(m.dependencies["alpha"].version_req(), Some("1.4.0"));
        assert!(matches!(m.dependencies["beta"], Dependency::Path { .. }));
        assert!(matches!(
            m.dependencies["epsilon"],
            Dependency::Shelf { .. }
        ));
        assert!(matches!(
            m.dependencies["gamma"],
            Dependency::Git { tag: Some(_), .. }
        ));
        assert!(matches!(
            m.dependencies["delta"],
            Dependency::Git { rev: Some(_), .. }
        ));
    }

    #[test]
    fn round_trips_through_toml() {
        let text = r#"
[package]
name = "app"
version = "1.0.0"

[dependencies]
lib = "2.0.0"
"#;
        let m = Manifest::from_toml(text).unwrap();
        let reparsed = Manifest::from_toml(&m.to_toml()).unwrap();
        assert_eq!(m, reparsed);
    }
}

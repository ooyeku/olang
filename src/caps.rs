//! Capability manifests: what a program — and each of its dependencies —
//! is allowed to touch.
//!
//! olang's stdlib is cleanly module-gated (`fs`, `http`, `db`, `proc`, and
//! the environment surface of `os` are distinct modules), so capability
//! enforcement is one check at the builtin dispatch boundary. A package
//! declares its needs in `olang.toml`:
//!
//! ```toml
//! [capabilities]
//! fs = "read"        # false | "read" | true
//! net = false
//! proc = false
//! db = true
//! env = true
//!
//! [capabilities.dependencies.somelib]
//! fs = false         # attenuation: a dependency's grant can only shrink
//! ```
//!
//! Enforcement is fail-closed and attributed: when dependency code (by the
//! executing function's `def_file`) calls a gated builtin, the *dependency's*
//! grant applies — the intersection of the app's capabilities and the
//! attenuation, so a dependency can never exceed the app. No manifest means
//! full capability (backward compatible); `--deny` restricts any run from
//! the command line. Pure helpers inside gated modules (path joins, URL
//! parsing) are never gated — capabilities police *effects*, not arithmetic.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Filesystem access level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FsCap {
    /// No filesystem access (pure path helpers still work).
    None,
    /// Read-only: reads, listings, walks, globs — no writes, no chdir.
    Read,
    /// Full read/write access.
    #[default]
    Full,
}

/// One resolved capability set. The default is everything allowed —
/// capabilities are opt-in restriction, never surprise breakage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub fs: FsCap,
    pub net: bool,
    pub proc: bool,
    pub db: bool,
    pub env: bool,
}

impl Default for Caps {
    fn default() -> Self {
        Caps {
            fs: FsCap::Full,
            net: true,
            proc: true,
            db: true,
            env: true,
        }
    }
}

impl Caps {
    /// The intersection of two capability sets: the result allows only
    /// what both allow. Attenuation composes through this, so a grant can
    /// only ever shrink.
    pub fn intersect(self, other: Caps) -> Caps {
        Caps {
            fs: match (self.fs, other.fs) {
                (FsCap::None, _) | (_, FsCap::None) => FsCap::None,
                (FsCap::Read, _) | (_, FsCap::Read) => FsCap::Read,
                _ => FsCap::Full,
            },
            net: self.net && other.net,
            proc: self.proc && other.proc,
            db: self.db && other.db,
            env: self.env && other.env,
        }
    }

    /// Human-readable one-line summary ("fs=read net=false proc=false ...").
    pub fn summary(&self) -> String {
        format!(
            "fs={} net={} proc={} db={} env={}",
            match self.fs {
                FsCap::None => "false",
                FsCap::Read => "read",
                FsCap::Full => "true",
            },
            self.net,
            self.proc,
            self.db,
            self.env
        )
    }
}

/// A capability value as written in TOML: `true`, `false`, or a level
/// string like `"read"`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CapValue {
    Flag(bool),
    Level(String),
}

/// The `[capabilities]` (or `[capabilities.dependencies.<name>]`) table as
/// written: every field optional, absent means unchanged from the default.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CapsSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs: Option<CapValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proc: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<bool>,
}

impl CapsSpec {
    /// Resolve the written spec against the wide-open default.
    pub fn resolve(&self) -> Result<Caps, String> {
        let mut caps = Caps::default();
        if let Some(v) = &self.fs {
            caps.fs = match v {
                CapValue::Flag(true) => FsCap::Full,
                CapValue::Flag(false) => FsCap::None,
                CapValue::Level(s) if s == "read" => FsCap::Read,
                CapValue::Level(s) => {
                    return Err(format!(
                        "capabilities: fs = {:?} is not one of true, false, \"read\"",
                        s
                    ));
                }
            };
        }
        if let Some(v) = self.net {
            caps.net = v;
        }
        if let Some(v) = self.proc {
            caps.proc = v;
        }
        if let Some(v) = self.db {
            caps.db = v;
        }
        if let Some(v) = self.env {
            caps.env = v;
        }
        Ok(caps)
    }
}

/// The whole `[capabilities]` block: the package's own grant plus
/// per-dependency attenuations.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CapsConfig {
    #[serde(flatten)]
    pub base: CapsSpec,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, CapsSpec>,
}

/// The runtime enforcement table: the app's resolved grant, and each
/// dependency's resolved grant keyed by the directory its code lives in
/// (matched against the executing function's `def_file`).
#[derive(Debug, Clone, Default)]
pub struct CapTable {
    pub app: Caps,
    /// (package dir, package name, resolved caps) — longest dir match wins.
    pub deps: Vec<(PathBuf, String, Caps)>,
}

impl CapTable {
    /// Build the table from a parsed config and the resolved dependency
    /// map (name -> source directory). A named attenuation for a package
    /// that is not a dependency is an error — a typo there would silently
    /// grant nothing to the wrong name.
    #[allow(clippy::type_complexity)]
    pub fn build(
        config: &CapsConfig,
        dep_dirs: &std::collections::HashMap<String, PathBuf>,
    ) -> Result<CapTable, String> {
        let app = config.base.resolve()?;
        let mut deps = Vec::new();
        for (name, spec) in &config.dependencies {
            let dir = dep_dirs.get(name).ok_or_else(|| {
                format!(
                    "capabilities: attenuation for '{}' names no known dependency",
                    name
                )
            })?;
            // Canonicalize so `app/../leftpad` and the loader's view of the
            // same directory compare equal — attribution is by real path.
            let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone());
            // Attenuation intersects with the app grant: shrink-only.
            let attenuated = spec.resolve()?.intersect(app);
            deps.push((dir, name.clone(), attenuated));
        }
        // Longest path first, so nested package dirs attribute correctly.
        deps.sort_by_key(|d| std::cmp::Reverse(d.0.as_os_str().len()));
        Ok(CapTable { app, deps })
    }

    /// The capability set that governs code defined in `def_file`
    /// (None / unknown files fall to the app grant).
    pub fn caps_for(&self, def_file: Option<&std::path::Path>) -> (&Caps, Option<&str>) {
        if let Some(path) = def_file {
            for (dir, name, caps) in &self.deps {
                if path.starts_with(dir) {
                    return (caps, Some(name.as_str()));
                }
            }
        }
        (&self.app, None)
    }
}

/// What a denied call was denied for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapDenial {
    /// The capability name ("fs", "net", "proc", "db", "env").
    pub capability: &'static str,
    /// The attenuated dependency responsible, if the caller was dep code.
    pub package: Option<String>,
}

/// Pure helpers inside gated modules: never gated — they compute, they
/// don't touch the world.
fn fs_pure(name: &str) -> bool {
    matches!(name, "join" | "basename" | "dirname" | "ext")
}

fn fs_read_only(name: &str) -> bool {
    matches!(
        name,
        "read_file"
            | "exists"
            | "is_dir"
            | "is_file"
            | "list_dir"
            | "file_info"
            | "file_size"
            | "walk"
            | "glob"
            | "abs_path"
    )
}

fn http_pure(name: &str) -> bool {
    matches!(name, "encode_query" | "decode_query" | "parse_url")
}

fn os_env_gated(name: &str) -> bool {
    matches!(
        name,
        "get_env"
            | "set_env"
            | "list_env"
            | "has_env"
            | "remove_env"
            | "hostname"
            | "username"
            | "home_dir"
    )
}

/// The gate: does `caps` permit the builtin `full_name` (e.g.
/// "fs.write_file")? Returns the denied capability, or None when allowed.
/// Anything not explicitly gated is allowed — capabilities restrict the
/// effectful surface, not computation.
pub fn check(caps: &Caps, full_name: &str) -> Option<&'static str> {
    if let Some(f) = full_name.strip_prefix("fs.") {
        if fs_pure(f) {
            return None;
        }
        return match caps.fs {
            FsCap::Full => None,
            FsCap::Read if fs_read_only(f) => None,
            _ => Some("fs"),
        };
    }
    if let Some(f) = full_name.strip_prefix("http.") {
        if http_pure(f) {
            return None;
        }
        return if caps.net { None } else { Some("net") };
    }
    if full_name.starts_with("db.") {
        return if caps.db { None } else { Some("db") };
    }
    if full_name.starts_with("proc.") || full_name == "os.exec" {
        return if caps.proc { None } else { Some("proc") };
    }
    if let Some(f) = full_name.strip_prefix("os.") {
        if os_env_gated(f) {
            return if caps.env { None } else { Some("env") };
        }
        // chdir moves the filesystem cursor: full fs only.
        if f == "chdir" {
            return match caps.fs {
                FsCap::Full => None,
                _ => Some("fs"),
            };
        }
        return None;
    }
    None
}

/// A capability a builtin call exercises, for the `--trace-caps` profiler.
/// Finer than the gate's yes/no verdict: `fs` splits read from write, so a
/// suggested manifest can propose `fs = "read"` when nothing wrote. `Ord`
/// so the used set collects into a stable, deduplicated `BTreeSet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapUse {
    FsRead,
    FsWrite,
    Net,
    Db,
    Proc,
    Env,
}

/// What capability the builtin `full_name` exercises, or None for a pure
/// helper / benign call. This is the profiler's dual of `check`: `check`
/// asks "does this grant permit the call?"; `required` asks "what would the
/// call need?" — the two share the same purity predicates so they never
/// disagree about what counts as an effect.
pub fn required(full_name: &str) -> Option<CapUse> {
    if let Some(f) = full_name.strip_prefix("fs.") {
        if fs_pure(f) {
            return None;
        }
        return Some(if fs_read_only(f) {
            CapUse::FsRead
        } else {
            CapUse::FsWrite
        });
    }
    if let Some(f) = full_name.strip_prefix("http.") {
        if http_pure(f) {
            return None;
        }
        return Some(CapUse::Net);
    }
    if full_name.starts_with("db.") {
        return Some(CapUse::Db);
    }
    if full_name.starts_with("proc.") || full_name == "os.exec" {
        return Some(CapUse::Proc);
    }
    if let Some(f) = full_name.strip_prefix("os.") {
        if os_env_gated(f) {
            return Some(CapUse::Env);
        }
        // chdir moves the fs cursor — a write-level fs demand.
        if f == "chdir" {
            return Some(CapUse::FsWrite);
        }
        return None;
    }
    None
}

/// Render a `--trace-caps` report from the set of capabilities a run
/// exercised: a human summary plus a ready-to-paste least-privilege
/// `[capabilities]` manifest. Every capability the program did *not* touch
/// is pinned shut, so pasting the block can only tighten, never loosen.
pub fn trace_report(used: &std::collections::BTreeSet<CapUse>) -> String {
    let fs = if used.contains(&CapUse::FsWrite) {
        FsCap::Full
    } else if used.contains(&CapUse::FsRead) {
        FsCap::Read
    } else {
        FsCap::None
    };
    let net = used.contains(&CapUse::Net);
    let db = used.contains(&CapUse::Db);
    let proc = used.contains(&CapUse::Proc);
    let env = used.contains(&CapUse::Env);

    let mut out = String::new();
    out.push_str("\n── capability profile (--trace-caps) ──\n");
    if used.is_empty() {
        out.push_str("this program exercised no gated capabilities.\n");
    } else {
        let touched: Vec<&str> = [
            (fs != FsCap::None).then_some(match fs {
                FsCap::Full => "fs (read+write)",
                _ => "fs (read)",
            }),
            net.then_some("net"),
            db.then_some("db"),
            proc.then_some("proc"),
            env.then_some("env"),
        ]
        .into_iter()
        .flatten()
        .collect();
        out.push_str(&format!("exercised: {}\n", touched.join(", ")));
    }
    let fs_field = match fs {
        FsCap::None => "false".to_string(),
        FsCap::Read => "\"read\"".to_string(),
        FsCap::Full => "true".to_string(),
    };
    out.push_str("\nsuggested least-privilege manifest:\n\n");
    out.push_str("  [capabilities]\n");
    out.push_str(&format!("  fs = {}\n", fs_field));
    out.push_str(&format!("  net = {}\n", net));
    out.push_str(&format!("  db = {}\n", db));
    out.push_str(&format!("  proc = {}\n", proc));
    out.push_str(&format!("  env = {}\n", env));
    if proc {
        out.push_str(
            "\nnote: proc lets the program spawn other processes, which run outside olang's\n\
             capability sandbox — grant it only to code you trust with the whole machine.\n",
        );
    }
    out
}

/// Parse a `--deny` list ("fs,net" or "fs=read") into a restriction set
/// to intersect with whatever the manifest grants.
pub fn parse_deny(list: &str) -> Result<Caps, String> {
    let mut caps = Caps::default();
    for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match item {
            "fs" => caps.fs = FsCap::None,
            "fs-write" => caps.fs = FsCap::Read,
            "net" => caps.net = false,
            "proc" => caps.proc = false,
            "db" => caps.db = false,
            "env" => caps.env = false,
            other => {
                return Err(format!(
                    "--deny: unknown capability '{}' (known: fs, fs-write, net, proc, db, env)",
                    other
                ));
            }
        }
    }
    Ok(caps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_wide_open_and_specs_restrict() {
        assert_eq!(CapsSpec::default().resolve().unwrap(), Caps::default());
        let spec = CapsSpec {
            fs: Some(CapValue::Level("read".into())),
            net: Some(false),
            ..Default::default()
        };
        let caps = spec.resolve().unwrap();
        assert_eq!(caps.fs, FsCap::Read);
        assert!(!caps.net);
        assert!(caps.proc && caps.db && caps.env);
        assert!(
            CapsSpec {
                fs: Some(CapValue::Level("write".into())),
                ..Default::default()
            }
            .resolve()
            .is_err()
        );
    }

    #[test]
    fn intersection_only_shrinks() {
        let read_only = Caps {
            fs: FsCap::Read,
            ..Caps::default()
        };
        let no_net = Caps {
            net: false,
            ..Caps::default()
        };
        let both = read_only.intersect(no_net);
        assert_eq!(both.fs, FsCap::Read);
        assert!(!both.net);
        // widening is impossible
        assert_eq!(both.intersect(Caps::default()), both);
    }

    #[test]
    fn the_gate_polices_effects_not_computation() {
        let sealed = Caps {
            fs: FsCap::None,
            net: false,
            proc: false,
            db: false,
            env: false,
        };
        // effects denied, with the right capability named
        assert_eq!(check(&sealed, "fs.read_file"), Some("fs"));
        assert_eq!(check(&sealed, "fs.write_file"), Some("fs"));
        assert_eq!(check(&sealed, "http.get"), Some("net"));
        assert_eq!(check(&sealed, "db.open"), Some("db"));
        assert_eq!(check(&sealed, "proc.spawn"), Some("proc"));
        assert_eq!(check(&sealed, "os.exec"), Some("proc"));
        assert_eq!(check(&sealed, "os.get_env"), Some("env"));
        assert_eq!(check(&sealed, "os.chdir"), Some("fs"));
        // pure helpers and benign os calls always pass
        assert_eq!(check(&sealed, "fs.join"), None);
        assert_eq!(check(&sealed, "http.parse_url"), None);
        assert_eq!(check(&sealed, "os.args"), None);
        assert_eq!(check(&sealed, "os.is_tty"), None);
        assert_eq!(check(&sealed, "str.trim"), None);
        // read level splits the fs surface
        let ro = Caps {
            fs: FsCap::Read,
            ..Caps::default()
        };
        assert_eq!(check(&ro, "fs.read_file"), None);
        assert_eq!(check(&ro, "fs.glob"), None);
        assert_eq!(check(&ro, "fs.write_file"), Some("fs"));
        assert_eq!(check(&ro, "os.chdir"), Some("fs"));
    }

    #[test]
    fn attenuation_attributes_by_directory() {
        let mut dirs = std::collections::HashMap::new();
        dirs.insert("lp".to_string(), PathBuf::from("/deps/lp"));
        let config = CapsConfig {
            base: CapsSpec {
                net: Some(true),
                ..Default::default()
            },
            dependencies: [(
                "lp".to_string(),
                CapsSpec {
                    net: Some(false),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
        };
        let table = CapTable::build(&config, &dirs).unwrap();
        let (app_caps, none) = table.caps_for(Some(std::path::Path::new("/src/main.ol")));
        assert!(app_caps.net);
        assert!(none.is_none());
        let (dep_caps, who) = table.caps_for(Some(std::path::Path::new("/deps/lp/index.ol")));
        assert!(!dep_caps.net);
        assert_eq!(who, Some("lp"));
        // unknown attenuation target is an error, not a silent no-op
        let bad = CapsConfig {
            dependencies: [("ghost".to_string(), CapsSpec::default())]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert!(CapTable::build(&bad, &dirs).is_err());
    }

    #[test]
    fn required_classifies_the_effect_surface() {
        assert_eq!(required("fs.read_file"), Some(CapUse::FsRead));
        assert_eq!(required("fs.glob"), Some(CapUse::FsRead));
        assert_eq!(required("fs.write_file"), Some(CapUse::FsWrite));
        assert_eq!(required("os.chdir"), Some(CapUse::FsWrite));
        assert_eq!(required("http.get"), Some(CapUse::Net));
        assert_eq!(required("db.open"), Some(CapUse::Db));
        assert_eq!(required("proc.spawn"), Some(CapUse::Proc));
        assert_eq!(required("os.exec"), Some(CapUse::Proc));
        assert_eq!(required("os.get_env"), Some(CapUse::Env));
        // pure / benign: no demand
        assert_eq!(required("fs.join"), None);
        assert_eq!(required("http.parse_url"), None);
        assert_eq!(required("os.args"), None);
        assert_eq!(required("str.trim"), None);
    }

    #[test]
    fn trace_report_suggests_a_shrink_only_manifest() {
        // A read + net program suggests fs="read", net=true, the rest shut.
        let used: std::collections::BTreeSet<CapUse> =
            [CapUse::FsRead, CapUse::Net].into_iter().collect();
        let report = trace_report(&used);
        assert!(report.contains("fs = \"read\""));
        assert!(report.contains("net = true"));
        assert!(report.contains("db = false"));
        assert!(report.contains("proc = false"));
        // A write demand upgrades fs to true.
        let w: std::collections::BTreeSet<CapUse> = [CapUse::FsWrite].into_iter().collect();
        assert!(trace_report(&w).contains("fs = true"));
        // proc carries the escape-hatch warning.
        let p: std::collections::BTreeSet<CapUse> = [CapUse::Proc].into_iter().collect();
        assert!(trace_report(&p).contains("outside olang's"));
        // An empty run pins everything shut.
        let empty = std::collections::BTreeSet::new();
        let r = trace_report(&empty);
        assert!(r.contains("no gated capabilities"));
        assert!(r.contains("fs = false"));
    }

    #[test]
    fn deny_lists_parse_and_reject_typos() {
        let d = parse_deny("fs-write, net").unwrap();
        assert_eq!(d.fs, FsCap::Read);
        assert!(!d.net);
        assert!(d.proc);
        assert!(parse_deny("networ").is_err());
    }
}

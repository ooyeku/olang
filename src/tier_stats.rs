//! Tier statistics for an editor: where each function ran, and why.
//!
//! `--ovm-stats=json[:PATH]` (or `OLANG_OVM_STATS=json[:PATH]`, which
//! reaches `olang test` and anything else this binary runs) keeps them
//! for the whole process and writes them as JSON at exit; `olang repl
//! --serve` keeps them for its evaluations and answers them to `stats`.
//! `olang check --tier --format json` is the static half: the verdict
//! for every function before anything runs.
//!
//! What is measured, and how (docs/tooling.md, "Tier statistics"):
//!
//! - calls per tier are exact: counted on the shadow-stack push every
//!   tier already makes for `olang profile` (src/profile.rs);
//! - time per tier is sampled, as the profiler samples it (a tick every
//!   250 µs), so no clock is read per call;
//! - a native attempt that declined is a *deopt*, with its reason;
//! - what the bytecode tier refused, a call it gave back to the
//!   tree-walker, and what native code refused are noted as they happen
//!   (failure paths only, so a run that compiles pays nothing for them).
//!
//! A function can be *pinned* native: a `// studio: native` comment on
//! the line above its `fn` (or at the end of that line), or a pattern in
//! the project's `olang.toml` `[check] native = [...]`. A pinned
//! function that falls back is a *violation*: listed in the JSON, said
//! on stderr at exit, and, with `OLANG_TIER_GUARD=1`, an exit status of
//! 3 — a local verify script's guard. `olang check --tier` fails on a
//! pinned function the tier refuses outright.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value as J, json};

/// The JSON's `format`, raised when a field changes meaning.
pub const FORMAT: u32 = 1;
/// The sampler's tick (µs): coarse enough to cost nothing, fine enough
/// for a 10 ms evaluation to be seen.
pub const INTERVAL_US: u64 = 250;
/// The comment that pins the function below it (or on its line).
pub const PIN_COMMENT: &str = "studio: native";

#[derive(Default, Clone)]
struct Facts {
    /// The bytecode tier's refusal (the compiler's message).
    refused: Option<String>,
    /// Why native code does not take it.
    native: Option<String>,
    /// Calls the tier handed back to the tree-walker after compiling it.
    fallbacks: u64,
    fallback_reason: Option<String>,
}

type Key = (String, Option<String>);

static FACTS: Mutex<Option<HashMap<Key, Facts>>> = Mutex::new(None);

fn with_facts(name: &str, file: Option<&str>, f: impl FnOnce(&mut Facts)) {
    let mut guard = FACTS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    f(map
        .entry((name.to_string(), file.map(str::to_string)))
        .or_default());
}

/// The bytecode tier refused `name` (declared in `file`).
pub fn note_refused(name: &str, file: Option<&str>, reason: &str) {
    if crate::profile::stats_on() {
        with_facts(name, file, |f| f.refused = Some(reason.to_string()));
    }
}

/// Native code will not take `name`.
pub fn note_native(name: &str, file: Option<&str>, why: &str) {
    if crate::profile::stats_on() {
        with_facts(name, file, |f| {
            if f.native.is_none() {
                f.native = Some(why.to_string())
            }
        });
    }
}

/// A call of compiled `name` went back to the tree-walker.
pub fn note_fallback(name: &str, file: Option<&str>, why: &str) {
    if crate::profile::stats_on() {
        with_facts(name, file, |f| {
            f.fallbacks += 1;
            if f.fallback_reason.is_none() {
                f.fallback_reason = Some(why.to_string());
            }
        });
    }
}

/// `json`, `json:PATH`: where the statistics go (`ovm-stats.json` in the
/// working directory when no path is given). Anything else is not JSON.
pub fn spec_path(spec: &str) -> Option<PathBuf> {
    match spec {
        "json" => Some(PathBuf::from("ovm-stats.json")),
        s => s
            .strip_prefix("json:")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
    }
}

static OUT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The promotion threshold the run's tier was built with (calls before
/// a function compiles; 1 unless `--ovm-tier` raised it).
static THRESHOLD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

pub fn note_threshold(n: u32) {
    THRESHOLD.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// Whether a statistics path names a directory: written with a trailing
/// `/`, or one that exists.
pub fn is_dir_spec(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().ends_with('/') || path.is_dir()
}

/// Keep statistics for the rest of the process and write them to `path`
/// at exit (`finish_run`). A directory gets a file of the process's own,
/// `<pid>.json`, so a run's children each leave theirs beside it.
pub fn begin_run(path: PathBuf) {
    let path = if is_dir_spec(&path) {
        let _ = std::fs::create_dir_all(&path);
        path.join(format!("{}.json", std::process::id()))
    } else {
        path
    };
    *OUT.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    crate::profile::stats_sample_begin(INTERVAL_US);
}

/// Write the statistics if a run asked for them (once; later calls do
/// nothing). Returns the exit status the guard asks for: 3 when
/// `OLANG_TIER_GUARD=1` and a pinned function fell back, else None.
pub fn finish_run() -> Option<i32> {
    let path = OUT.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    crate::profile::stats_sample_end();
    let report = report(false);
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    if let Err(e) = std::fs::write(&path, text) {
        eprintln!("--ovm-stats: cannot write {}: {}", path.display(), e);
    }
    let violations = report["violations"].as_array().cloned().unwrap_or_default();
    for v in &violations {
        eprintln!(
            "tier guard: {}:{} `{}` {}",
            v["file"].as_str().unwrap_or("?"),
            v["line"].as_u64().unwrap_or(0),
            v["name"].as_str().unwrap_or("?"),
            v["message"].as_str().unwrap_or("")
        );
    }
    let guard = matches!(std::env::var("OLANG_TIER_GUARD").as_deref(), Ok("1"));
    (guard && !violations.is_empty()).then_some(3)
}

/// A refusal's class, by its message — the anchor of its section in
/// docs/tooling.md ("Why a function stays on the tree-walker").
pub fn reason_code(reason: &str) -> &'static str {
    let r = reason.to_lowercase();
    if r.contains("ambiguous") || r.contains("more than one") {
        "ambiguous"
    } else if r.contains("which cannot compile") {
        "callee"
    } else if r.contains("nothing in its scope defines") || r.contains("unresolved variable") {
        "undefined"
    } else if r.contains("spawn") || r.contains("task") {
        "task"
    } else if r.contains("cell") {
        "cell"
    } else if r.contains("captur") || r.contains("closure") || r.contains("dynamic function call") {
        "capture"
    } else if r.contains("cannot convert") || r.contains("boundary") || r.contains("representable") {
        "boundary"
    } else if r.contains("dependency chain") {
        "depth"
    } else {
        "unsupported"
    }
}

/// The documentation of a reason code.
pub fn docs_ref(code: &str) -> String {
    format!("docs/tooling.md#tier-{code}")
}

// ── pins ───────────────────────────────────────────────────────────────

/// Each `fn` declared at the start of a line: its name and 1-based
/// (line, column of the name). The first declaration of a name wins.
pub fn fn_lines(text: &str) -> HashMap<String, (u32, u32)> {
    let mut out = HashMap::new();
    for (i, line) in text.lines().enumerate() {
        if let Some((name, col)) = fn_decl_name(line) {
            out.entry(name).or_insert((i as u32 + 1, col));
        }
    }
    out
}

/// The name a line declares with `fn`, and its column.
fn fn_decl_name(line: &str) -> Option<(String, u32)> {
    let mut rest = line.trim_start();
    let mut col = line.len() - rest.len();
    for prefix in ["share ", "pub ", "async "] {
        if let Some(r) = rest.strip_prefix(prefix) {
            let r2 = r.trim_start();
            col += rest.len() - r2.len();
            rest = r2;
        }
    }
    let r = rest.strip_prefix("fn ")?;
    let r2 = r.trim_start();
    col += rest.len() - r2.len();
    let name: String = r2
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then(|| (name, col as u32 + 1))
}

/// The functions a file pins with the comment: on the line above the
/// `fn` (comments and blank lines between are allowed) or at its end.
pub fn comment_pins(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut armed = false;
    for line in text.lines() {
        let t = line.trim();
        let pin_here = t
            .find("//")
            .is_some_and(|i| t[i + 2..].trim().starts_with(PIN_COMMENT));
        if let Some((name, _)) = fn_decl_name(line) {
            if armed || pin_here {
                out.insert(name);
            }
            armed = false;
        } else if pin_here && t.starts_with("//") {
            armed = true;
        } else if !t.is_empty() && !t.starts_with("//") && !t.starts_with('@') {
            armed = false;
        }
    }
    out
}

/// `[check] native` of the project `file` is in: its root and patterns.
pub fn project_rules(file: &Path) -> Option<(PathBuf, Vec<String>)> {
    let root = crate::pkg::manifest::Manifest::find_root(file)?;
    let rules = crate::pkg::manifest::Manifest::load(&root)
        .ok()
        .and_then(|m| m.check)
        .map(|c| c.native)
        .unwrap_or_default();
    (!rules.is_empty()).then_some((root, rules))
}

/// Does a `[check] native` pattern name `name` in `file`? A pattern is a
/// function name (`*` matches any run of characters), optionally after
/// a path relative to the project's root and a colon:
/// `"rt_frame"`, `"ly_*"`, `"lib/engine.ol:rt_*"`.
pub fn rule_matches(pattern: &str, root: &Path, file: &Path, name: &str) -> bool {
    match pattern.rsplit_once(':') {
        Some((path, pat)) => {
            let want = root.join(path);
            let same = match (want.canonicalize(), file.canonicalize()) {
                (Ok(a), Ok(b)) => a == b,
                _ => want == file,
            };
            same && glob(pat, name)
        }
        None => glob(pattern, name),
    }
}

fn glob(pattern: &str, s: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len()).any(|i| go(rest, &s[i..])),
            Some((c, rest)) => s.first() == Some(c) && go(rest, &s[1..]),
        }
    }
    go(pattern.as_bytes(), s.as_bytes())
}

/// What a file says about its functions: where each is declared, which
/// the comment pins, and the project's rules.
pub struct FileFacts {
    pub lines: HashMap<String, (u32, u32)>,
    pub comment: HashSet<String>,
    pub rules: Option<(PathBuf, Vec<String>)>,
    pub path: PathBuf,
}

impl FileFacts {
    pub fn read(path: &Path) -> FileFacts {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        FileFacts::of(path, &text)
    }

    pub fn of(path: &Path, text: &str) -> FileFacts {
        FileFacts {
            lines: fn_lines(text),
            comment: comment_pins(text),
            rules: project_rules(path),
            path: path.to_path_buf(),
        }
    }

    /// How `name` is pinned: `comment`, `olang.toml`, or not at all.
    pub fn pin(&self, name: &str) -> Option<&'static str> {
        if self.comment.contains(name) {
            return Some("comment");
        }
        let (root, rules) = self.rules.as_ref()?;
        rules
            .iter()
            .any(|r| rule_matches(r, root, &self.path, name))
            .then_some("olang.toml")
    }
}

// ── the report ─────────────────────────────────────────────────────────

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// The statistics so far as the JSON `--ovm-stats=json` writes and the
/// REPL's `stats` answers (docs/tooling.md). `reset` starts them again.
pub fn report(reset: bool) -> J {
    let snap = crate::profile::stats_snapshot(reset);
    let facts = {
        let mut guard = FACTS.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        if reset {
            std::mem::take(map)
        } else {
            map.clone()
        }
    };
    let threshold = THRESHOLD.load(std::sync::atomic::Ordering::Relaxed) as u64;
    let mut files: HashMap<String, FileFacts> = HashMap::new();
    let mut functions = Vec::new();
    let mut violations = Vec::new();
    let mut tiers_ms = [0.0f64; 4];
    let mut seen: HashSet<Key> = HashSet::new();
    let mut rows = snap.rows;
    // a function the run noted something about but never pushed (a
    // refusal at compile time of a function called only from compiled
    // code, say) still gets its row
    for ((name, file), _) in facts.iter() {
        if !rows.iter().any(|r| &r.name == name && &r.file == file) {
            rows.push(crate::profile::StatRow {
                name: name.clone(),
                file: file.clone(),
                ..Default::default()
            });
        }
    }
    for row in rows {
        let key: Key = (row.name.clone(), row.file.clone());
        if !seen.insert(key.clone()) {
            continue;
        }
        for (t, ms) in tiers_ms.iter_mut().zip(row.self_ms.iter()) {
            *t += ms;
        }
        let fact = facts.get(&key).cloned().unwrap_or_default();
        let lambda = row.name.starts_with('<');
        let ff = row.file.as_ref().filter(|f| !f.starts_with("__embedded__")).map(|f| {
            files
                .entry(f.clone())
                .or_insert_with(|| FileFacts::read(Path::new(f)))
        });
        let (line, col) = ff
            .as_ref()
            .and_then(|ff| ff.lines.get(&row.name).copied())
            .map(|(l, c)| (J::from(l), J::from(c)))
            .unwrap_or((J::Null, J::Null));
        let pinned = if lambda {
            None
        } else {
            ff.as_ref().and_then(|ff| ff.pin(&row.name))
        };
        let [ci, cv, cn] = row.calls;
        let [ti, tv, tn, tb] = row.self_ms;
        // the tier that served its calls (counted exactly); time, which
        // is sampled, only when nothing was counted
        let tier = if cn + cv + ci > 0 {
            if cn >= cv && cn >= ci {
                "native"
            } else if cv >= ci {
                "bytecode"
            } else {
                "interpreter"
            }
        } else if ti + tv + tn > 0.0 {
            if tn >= tv && tn >= ti {
                "native"
            } else if tv >= ti {
                "bytecode"
            } else {
                "interpreter"
            }
        } else {
            "none"
        };
        let deopt_reasons: Vec<J> = row
            .decline_reasons
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(i, n)| json!({ "reason": crate::profile::DECLINE_REASONS[i], "count": n }))
            .collect();
        let refused = fact.refused.as_ref().map(|r| {
            let code = reason_code(r);
            json!({ "reason": r, "code": code, "docs": docs_ref(code) })
        });
        // a pinned function falls back when it ran on the tree-walker past
        // its warm-up, or stayed on bytecode because native code would not
        // take it
        let violation = pinned.and_then(|_| {
            if ci > threshold || fact.refused.is_some() || fact.fallbacks > 0 {
                let why = fact
                    .refused
                    .clone()
                    .or(fact.fallback_reason.clone())
                    .unwrap_or_else(|| "it never reached a compiled tier".to_string());
                Some(format!(
                    "is pinned native but {} call{} ran on the tree-walker: {}",
                    ci,
                    if ci == 1 { "" } else { "s" },
                    why
                ))
            } else if cv > 0 && cn == 0 && (fact.native.is_some() || row.declines >= cv) {
                let why = fact
                    .native
                    .clone()
                    .or_else(|| {
                        row.decline_reasons
                            .iter()
                            .enumerate()
                            .max_by_key(|(_, n)| **n)
                            .filter(|(_, n)| **n > 0)
                            .map(|(i, _)| crate::profile::DECLINE_REASONS[i].to_string())
                    })
                    .unwrap_or_else(|| "native code did not take it".to_string());
                Some(format!(
                    "is pinned native but its {} call{} stayed on bytecode: {}",
                    cv,
                    if cv == 1 { "" } else { "s" },
                    why
                ))
            } else {
                None
            }
        });
        if let Some(v) = &violation {
            violations.push(json!({ "name": row.name, "file": row.file, "line": line, "message": v }));
        }
        functions.push(json!({
            "name": row.name,
            "file": row.file,
            "line": line,
            "col": col,
            "lambda": lambda,
            "tier": tier,
            "calls": { "interpreter": ci, "bytecode": cv, "native": cn },
            "self_ms": { "interpreter": round3(ti), "bytecode": round3(tv), "native": round3(tn), "builtin": round3(tb) },
            "total_ms": round3(row.total_ms),
            "promoted": cv + cn > 0,
            "deopts": row.declines,
            "deopt_reasons": deopt_reasons,
            "refused": refused,
            "native_refused": fact.native,
            "fallbacks": if fact.fallbacks > 0 {
                json!({ "count": fact.fallbacks, "reason": fact.fallback_reason })
            } else {
                J::Null
            },
            "pinned": pinned,
            "violation": violation,
        }));
    }
    json!({
        "format": FORMAT,
        "kind": "ovm-stats",
        "olang": crate::version::VERSION,
        "threshold": threshold,
        "interval_us": INTERVAL_US,
        "sampled_ms": round3(snap.sampled_ms),
        "ms_per_sample": round3(snap.ms_per_sample),
        "tiers_ms": {
            "interpreter": round3(tiers_ms[0]),
            "bytecode": round3(tiers_ms[1]),
            "native": round3(tiers_ms[2]),
            "builtin": round3(tiers_ms[3]),
        },
        "functions": functions,
        "violations": violations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_by_comment_above_or_at_the_end() {
        let text = "// studio: native\nfn a() = 1\nfn b() = 2 // studio: native\n// studio: native\n// why\nshare fn c() = 3\nfn d() = 4\n";
        let pins = comment_pins(text);
        assert!(pins.contains("a") && pins.contains("b") && pins.contains("c"));
        assert!(!pins.contains("d"));
        assert_eq!(fn_lines(text).get("c"), Some(&(6, 10)));
    }

    #[test]
    fn rules_glob_names_and_paths() {
        assert!(glob("ly_*", "ly_cell"));
        assert!(!glob("ly_*", "rt_frame"));
        assert!(glob("rt_frame", "rt_frame"));
        assert_eq!(spec_path("json:/tmp/x.json"), Some(PathBuf::from("/tmp/x.json")));
        assert_eq!(spec_path("text"), None);
        assert_eq!(reason_code("calls 'ly_cell', which cannot compile"), "callee");
    }
}

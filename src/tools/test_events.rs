//! `olang test --format json`: the runner's events as line-delimited JSON
//! on stdout, for an editor (olang Studio's tests panel) — one object a
//! line, each with an `event` field, written as the run goes:
//!
//! - `run`       the files to run and how many test blocks they declare
//! - `test`      a block starts (file, name, line)
//! - `passed`    a block passed (ms)
//! - `failed`    a block failed: the message, where the failing assertion
//!               or error is (file, line, col), the call frames, and for
//!               `assert_eq`/`assert_ne` (and a text snapshot) the expected
//!               and actual values (`kind`: `assert_eq`, `assert_ne`,
//!               `snapshot`, `pixels`, `error`)
//! - `snapshot`  a pixel snapshot that differs (Loom's `snapshot`, through
//!               `testing.snapshot_failed`): baseline, actual and diff
//!               paths and the difference's numbers
//! - `skipped`   a block of the file the run names that `--only` left out
//! - `divergence` `--verify-tiers` found native code and the VM disagreeing
//!               (the function and both values); the run ends after it
//! - `file`      a file is done: its counts, and the error that stopped it
//! - `args`      with `--record-args`, once a file is done: the last call
//!               a test block made to each of the project's top-level
//!               functions (file, fn, params, the block, and each
//!               argument as olang source — `null` for one with no
//!               literal: a function, a handle, a record)
//! - `finished`  the totals, the milliseconds, and the exit status
//!
//! What the program under test prints still goes to stdout, between the
//! events, as it was printed: a reader takes a line holding `{"event":`
//! as an event and anything else as the current test's output. The human
//! output is unchanged when the format is not JSON (docs/tooling.md,
//! "olang test --format json").

use crate::ast::Value;
use serde_json::{Value as J, json};
use std::cell::RefCell;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The events' `format`, raised when a field changes meaning.
pub const FORMAT: u32 = 1;

static ON: AtomicBool = AtomicBool::new(false);

/// True while the run writes JSON events.
#[inline(always)]
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Write JSON events from now on.
pub fn enable() {
    ON.store(true, Ordering::Relaxed);
}

/// The paths as the run was asked for them: `(canonical, as given)`
/// prefixes, longest first. The runner works on canonical paths (a
/// module's identity); a reader names its files as it gave them (a
/// project under `/var/…`, which is `/private/var/…`, or a symlink).
static ALIASES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// The paths the run was given: each, and the package root above it (the
/// directory with an `olang.toml`), spelled as given (made absolute
/// without resolving links) stand for their canonical selves.
pub fn set_aliases(paths: &[std::path::PathBuf]) {
    let mut out: Vec<(String, String)> = Vec::new();
    for p in paths {
        let given = std::path::absolute(p).unwrap_or_else(|_| p.clone());
        let mut dirs = vec![if given.is_dir() { given.clone() } else { given.parent().map(|d| d.to_path_buf()).unwrap_or_default() }];
        let mut up = dirs[0].clone();
        while let Some(parent) = up.parent().map(|d| d.to_path_buf()) {
            if up.join("olang.toml").exists() {
                dirs.push(up.clone());
                break;
            }
            up = parent;
        }
        for d in dirs {
            if let Ok(c) = d.canonicalize() {
                let (c, g) = (c.display().to_string(), d.display().to_string());
                if c != g && !out.iter().any(|(a, _)| *a == c) {
                    out.push((c, g));
                }
            }
        }
    }
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    *ALIASES.lock().unwrap_or_else(|e| e.into_inner()) = out;
}

fn shown(path: &str, aliases: &[(String, String)]) -> Option<String> {
    aliases.iter().find_map(|(c, g)| {
        path.strip_prefix(c.as_str())
            .filter(|rest| rest.is_empty() || rest.starts_with('/'))
            .map(|rest| format!("{}{}", g, rest))
    })
}

// An event's paths (`file`, `files`, a snapshot's pictures) as given.
fn rewrite(v: &mut J, aliases: &[(String, String)]) {
    match v {
        J::Object(o) => {
            for (k, x) in o.iter_mut() {
                match (k.as_str(), &mut *x) {
                    ("file" | "baseline" | "actual" | "diff", J::String(p)) => {
                        if let Some(n) = shown(p, aliases) {
                            *p = n;
                        }
                    }
                    ("files", J::Array(items)) => {
                        for it in items.iter_mut() {
                            if let J::String(p) = it
                                && let Some(n) = shown(p, aliases)
                            {
                                *p = n;
                            }
                        }
                    }
                    ("at" | "snapshot" | "error", _) => rewrite(x, aliases),
                    ("frames" | "snapshots", J::Array(items)) => items.iter_mut().for_each(|it| rewrite(it, aliases)),
                    _ => {}
                }
            }
        }
        J::Array(items) => items.iter_mut().for_each(|it| rewrite(it, aliases)),
        _ => {}
    }
}

/// One event, a line of its own (stdout flushed around it, so it never
/// lands inside a line the program printed without its newline).
pub fn emit(mut event: J) {
    {
        let aliases = ALIASES.lock().unwrap_or_else(|e| e.into_inner());
        if !aliases.is_empty() {
            rewrite(&mut event, &aliases);
        }
    }
    let out = std::io::stdout();
    let mut lock = out.lock();
    let _ = lock.flush();
    let text = serde_json::to_string(&event).unwrap_or_default();
    let _ = writeln!(lock, "{}", text);
    let _ = lock.flush();
}

/// The run's tallies and the block under way, for the events that need
/// them from where the runner cannot see (a divergence ends the process
/// from inside the VM).
#[derive(Default)]
struct RunState {
    test: Option<(String, String, u32)>,
    passed: u64,
    failed: u64,
    skipped: u64,
    files: u64,
    started: Option<std::time::Instant>,
}

static STATE: Mutex<Option<RunState>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut RunState) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(RunState::default))
}

thread_local! {
    /// The last failing assertion's detail on this thread (its values),
    /// taken by the block that fails.
    static DETAIL: RefCell<Option<J>> = const { RefCell::new(None) };
    /// The pixel snapshots that differed in the block under way.
    static SNAPSHOTS: RefCell<Vec<J>> = const { RefCell::new(Vec::new()) };
}

/// The run starts: the files it will visit, the blocks they declare.
pub fn run_started(files: &[String], count: usize, only: Option<&str>) {
    with_state(|s| {
        *s = RunState {
            started: Some(std::time::Instant::now()),
            ..RunState::default()
        }
    });
    emit(json!({ "event": "run", "format": FORMAT, "files": files, "count": count, "only": only,
                 "verify_tiers": std::env::var("OLANG_VERIFY_TIERS").ok().and_then(|v| v.parse::<f64>().ok()) }));
}

/// A block starts.
pub fn test_started(file: &str, name: &str, line: u32) {
    DETAIL.with(|d| d.borrow_mut().take());
    SNAPSHOTS.with(|s| s.borrow_mut().clear());
    with_state(|s| s.test = Some((file.to_string(), name.to_string(), line)));
    emit(json!({ "event": "test", "file": file, "name": name, "line": line }));
}

/// Where a block's failure is, and the frames above it.
pub struct Failure {
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub col: Option<u32>,
    pub frames: Vec<String>,
    /// The line of the block's statement the failure surfaced from (its
    /// place in the test file, when the failure itself is in a library).
    pub stmt: Option<u32>,
}

/// A block ends: passed when `failure` is None.
pub fn test_done(file: &str, name: &str, line: u32, ms: f64, failure: Option<Failure>) {
    let detail = DETAIL.with(|d| d.borrow_mut().take());
    let snapshots = SNAPSHOTS.with(|s| std::mem::take(&mut *s.borrow_mut()));
    with_state(|s| {
        s.test = None;
        if failure.is_some() {
            s.failed += 1
        } else {
            s.passed += 1
        }
    });
    let ms = (ms * 1000.0).round() / 1000.0;
    match failure {
        None => emit(json!({ "event": "passed", "file": file, "name": name, "line": line, "ms": ms })),
        Some(f) => {
            let mut ev = json!({ "event": "failed", "file": file, "name": name, "line": line, "ms": ms,
                                 "message": f.message,
                                 "at": { "file": f.file.unwrap_or_else(|| file.to_string()), "line": f.line, "col": f.col },
                                 "frames": frames_json(&f.frames, file), "stmt": f.stmt });
            let kind = if !snapshots.is_empty() {
                "pixels"
            } else {
                detail
                    .as_ref()
                    .and_then(|d| d["kind"].as_str())
                    .unwrap_or("error")
            };
            ev["kind"] = J::String(kind.to_string());
            if let Some(d) = detail {
                ev["expected"] = d["expected"].clone();
                ev["actual"] = d["actual"].clone();
            }
            if !snapshots.is_empty() {
                ev["snapshots"] = J::Array(snapshots);
            }
            emit(ev)
        }
    }
}

/// A block of the named file `--only` left out.
pub fn test_skipped(file: &str, name: &str, line: u32) {
    with_state(|s| s.skipped += 1);
    emit(json!({ "event": "skipped", "file": file, "name": name, "line": line }));
}

/// A file is done: its counts, and the error outside any block (or the
/// parse error) that stopped it.
pub fn file_done(file: &str, passed: usize, failed: usize, error: Option<Failure>, kind: &str) {
    with_state(|s| s.files += 1);
    let err = error.map(|f| {
        json!({ "kind": kind, "message": f.message, "line": f.line, "col": f.col,
                "file": f.file.unwrap_or_else(|| file.to_string()), "frames": frames_json(&f.frames, file) })
    });
    emit(json!({ "event": "file", "file": file, "passed": passed, "failed": failed, "error": err }));
}

/// The run is over.
pub fn finished(passed: usize, failed: usize, files: usize, ms: u128, code: i32) {
    let skipped = with_state(|s| s.skipped);
    emit(json!({ "event": "finished", "passed": passed, "failed": failed, "skipped": skipped, "files": files,
                 "ms": ms as u64, "code": code }));
}

/// `--verify-tiers` found native code and the VM disagreeing: said as an
/// event (with the block under way), then the run's end, before the
/// process exits.
pub fn divergence(what: &str, native: &str, vm: &str) {
    let (test, passed, failed, files, ms) = with_state(|s| {
        (
            s.test.clone(),
            s.passed,
            s.failed,
            s.files,
            s.started.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0),
        )
    });
    let (file, name, line) = match test {
        Some((f, n, l)) => (Some(f), Some(n), Some(l)),
        None => (None, None, None),
    };
    emit(json!({ "event": "divergence", "function": what, "native": native, "vm": vm,
                 "file": file, "name": name, "line": line }));
    emit(json!({ "event": "finished", "passed": passed, "failed": failed + 1, "skipped": 0, "files": files,
                 "ms": ms, "code": 102, "aborted": "divergence" }));
}

/// A failing `assert_eq` (`kind` `assert_eq`; `assert_ne`; a text
/// snapshot): its values kept for the block's `failed` event.
pub fn note_values(kind: &str, expected: &Value, actual: &Value) {
    if !enabled() {
        return;
    }
    let d = json!({ "kind": kind, "expected": value_json(expected), "actual": value_json(actual) });
    DETAIL.with(|slot| *slot.borrow_mut() = Some(d));
}

/// `testing.snapshot_failed(info)`: a pixel snapshot that differs, from
/// the library that compares them (Loom's `snapshot`). `info` is a map:
/// `name`, `baseline`, `actual`, `diff` (paths), `differing`, `total`,
/// `worst`, `sizes`.
pub fn note_snapshot(info: &Value) {
    if !enabled() {
        return;
    }
    let j = match info {
        Value::Map(m) => {
            let mut o = serde_json::Map::new();
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for k in keys {
                o.insert(k.clone(), plain_json(&m[k], 0));
            }
            // the paths as a reader can open them: a library's relative
            // path is the test file's directory's, where the run stands
            for k in ["baseline", "actual", "diff"] {
                if let Some(J::String(p)) = o.get(k)
                    && std::path::Path::new(p).is_relative()
                    && let Ok(here) = std::env::current_dir()
                {
                    let joined = here.join(p.trim_start_matches("./"));
                    o.insert(k.to_string(), J::String(joined.display().to_string()));
                }
            }
            J::Object(o)
        }
        other => json!({ "message": other.to_string() }),
    };
    SNAPSHOTS.with(|s| s.borrow_mut().push(j.clone()));
    let test = with_state(|s| s.test.clone());
    let mut ev = json!({ "event": "snapshot", "snapshot": j });
    if let Some((file, name, line)) = test {
        ev["file"] = J::String(file);
        ev["name"] = J::String(name);
        ev["line"] = json!(line);
    }
    emit(ev);
}

// A frame `name (file)` as `{ name, file, line }`: the line of its `fn`
// when its file says it.
fn frames_json(frames: &[String], entry: &str) -> J {
    J::Array(
        frames
            .iter()
            .rev()
            .map(|frame| {
                let (name, file) = match frame.rfind(" (") {
                    Some(i) if frame.ends_with(')') => (frame[..i].to_string(), frame[i + 2..frame.len() - 1].to_string()),
                    _ => (frame.clone(), entry.to_string()),
                };
                let bare = name.rsplit('.').next().unwrap_or(&name).to_string();
                let line = std::fs::read_to_string(&file)
                    .ok()
                    .and_then(|text| crate::tier_stats::fn_lines(&text).get(&bare).map(|(l, _)| *l));
                json!({ "name": name, "file": file, "line": line })
            })
            .collect(),
    )
}

const CLIP: usize = 20_000;

fn clip(s: String) -> (String, bool) {
    if s.len() <= CLIP {
        return (s, false);
    }
    let cut = (0..=CLIP).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(0);
    (s[..cut].to_string(), true)
}

/// The last call a test made to a function (`--record-args`).
pub fn args(call: &crate::interpreter::TestCallArgs) {
    let args: Vec<J> = call.args.iter().map(|a| literal(a).map(J::String).unwrap_or(J::Null)).collect();
    emit(json!({ "event": "args", "file": call.file, "fn": call.name, "params": call.params, "args": args, "test": call.test }));
}

/// The longest literal `args` says; a longer value is left for a person
/// to give (`null`).
const LITERAL_MAX: usize = 4000;

/// A value as olang source that makes it again — scalars, strings, lists,
/// tuples and maps of them — or None (a function, a handle, a record, a
/// Result, a float that is not finite, a value too long to read).
pub fn literal(v: &Value) -> Option<String> {
    let mut out = String::new();
    if write_literal(v, &mut out, 0) && out.len() <= LITERAL_MAX { Some(out) } else { None }
}

fn write_literal(v: &Value, out: &mut String, depth: usize) -> bool {
    if depth > 16 || out.len() > LITERAL_MAX {
        return false;
    }
    match v {
        Value::Integer(n) => out.push_str(&n.to_string()),
        Value::Float(x) if x.is_finite() => {
            let t = format!("{:?}", x);
            out.push_str(&t);
            if !t.contains('.') && !t.contains('e') && !t.contains("inf") {
                out.push_str(".0");
            }
        }
        Value::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Unit => out.push_str("()"),
        Value::String(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Value::List(items) | Value::Tuple(items) => {
            let tuple = matches!(v, Value::Tuple(_));
            // a tuple of one or none has no literal of its own
            if tuple && items.len() < 2 {
                return false;
            }
            out.push(if tuple { '(' } else { '[' });
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if !write_literal(it, out, depth + 1) {
                    return false;
                }
            }
            out.push(if tuple { ')' } else { ']' });
        }
        Value::Map(m) => {
            if m.is_empty() {
                out.push_str("#{}");
                return true;
            }
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push_str("#{ ");
            for (i, k) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if !write_literal(&Value::String(std::sync::Arc::new(k.clone())), out, depth + 1) {
                    return false;
                }
                out.push_str(": ");
                if !write_literal(&m[k], out, depth + 1) {
                    return false;
                }
            }
            out.push_str(" }");
        }
        _ => return false,
    }
    true
}

/// A value for a reader: its type, its `show` form, a form one entry a
/// line for a structure too long for one line (what a diff reads), the
/// text itself for a string, and the value as JSON where it has one.
pub fn value_json(v: &Value) -> J {
    let (show, cut) = clip(v.to_string());
    let (pretty, _) = clip(pretty(v, 0));
    let mut o = json!({ "type": v.type_name(), "show": show, "pretty": pretty, "value": plain_json(v, 0) });
    if cut {
        o["clipped"] = J::Bool(true);
    }
    if let Value::String(s) = v {
        o["string"] = J::String(clip(s.to_string()).0);
    }
    o
}

// The value as JSON, as far as it has one (depth and breadth bounded).
fn plain_json(v: &Value, depth: usize) -> J {
    if depth > 12 {
        return J::String(v.to_string());
    }
    match v {
        Value::Integer(n) => json!(n),
        Value::Float(x) if x.is_finite() => json!(x),
        Value::Float(x) => J::String(crate::ast::format_float(*x)),
        Value::String(s) => J::String(s.to_string()),
        Value::Boolean(b) => J::Bool(*b),
        Value::Unit => J::Null,
        Value::List(items) | Value::Tuple(items) => J::Array(items.iter().take(2000).map(|x| plain_json(x, depth + 1)).collect()),
        Value::Map(m) => {
            let mut o = serde_json::Map::new();
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            for k in keys.into_iter().take(2000) {
                o.insert(k.clone(), plain_json(&m[k], depth + 1));
            }
            J::Object(o)
        }
        Value::Struct { fields, .. } => {
            let mut o = serde_json::Map::new();
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            for k in keys.into_iter().take(2000) {
                o.insert(k.clone(), plain_json(&fields[k], depth + 1));
            }
            J::Object(o)
        }
        other => J::String(other.to_string()),
    }
}

/// `show`, an entry a line (indented two spaces a level) for a list, map,
/// tuple or struct whose one-line form is longer than 60 characters.
pub fn pretty(v: &Value, indent: usize) -> String {
    let flat = v.to_string();
    if flat.len() <= 60 || indent > 40 {
        return flat;
    }
    let pad = "  ".repeat(indent + 1);
    let end = "  ".repeat(indent);
    let join = |open: &str, items: Vec<String>, close: &str| -> String {
        if items.is_empty() {
            return format!("{}{}", open, close);
        }
        format!("{}\n{}{}\n{}{}", open, pad, items.join(&format!(",\n{}", pad)), end, close)
    };
    match v {
        Value::List(items) => join("[", items.iter().map(|x| pretty(x, indent + 1)).collect(), "]"),
        Value::Tuple(items) => join("(", items.iter().map(|x| pretty(x, indent + 1)).collect(), ")"),
        Value::Map(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            join("#{", keys.iter().map(|k| format!("\"{}\": {}", k, pretty(&m[*k], indent + 1))).collect(), "}")
        }
        Value::Struct { type_name, fields } => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            join(
                &format!("<struct: {}>{{", type_name),
                keys.iter().map(|k| format!("{}: {}", k, pretty(&fields[*k], indent + 1))).collect(),
                "}",
            )
        }
        Value::Ok(inner) => format!("Ok({})", pretty(inner, indent)),
        Value::Err(inner) => format!("Err({})", pretty(inner, indent)),
        _ => flat,
    }
}

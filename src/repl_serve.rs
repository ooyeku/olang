//! `olang repl --serve` — the REPL as a protocol, for an editor (olang
//! Studio) that keeps one long-lived REPL per project.
//!
//! Line-delimited JSON over stdio: one request a line in, one reply a
//! line out (docs/tooling.md has the messages). The session is bound to
//! the project the working directory is in, as `olang eval` is: its
//! dependencies and shelf libraries resolve.
//!
//! - **Scopes.** Code evaluated with a `file` runs in that file's scope:
//!   the file's declarations (its `use`s, functions, types, and the
//!   top-level `let`s that call nothing) loaded from disk the first time,
//!   and whatever was evaluated there since. Two files' helpers of the
//!   same name never meet. Code with no file runs in the session's own
//!   scope (a REPL prompt's).
//! - **Lines.** A snippet evaluated at line `n` of its file is parsed as
//!   if it stood there, so every line an error names is the file's.
//! - **Values.** A reply carries a value as structure, not text: scalars
//!   with their type, containers with a handle and their first items,
//!   lists of maps or records as a table, lists of numbers as a series,
//!   Bytes holding a picture as the picture, a Loom view as a view.
//!   `expand` asks for more of a handle (a range; a table's rows sorted).
//! - **Interrupt.** Requests are read on a thread of their own, so an
//!   `interrupt` reaches a running evaluation: every tier polls one flag
//!   (src/interrupt.rs) and the evaluation fails with "interrupted",
//!   leaving the session as it was.
//! - **Reload.** `reload` re-reads a saved file: its scope's declarations
//!   again, and every scope's `use`s (the module cache sees the new
//!   content). A file that no longer parses changes nothing.
//! - **Views.** `render` draws a Loom view value headless (Loom's
//!   `render_view`, from the project's dependency or a path given) and
//!   answers a PNG.
//!
//! What a program prints while the session serves is collected and sent
//! with the evaluation that printed it; the protocol keeps stdout to
//! itself (fd 1 is pointed at stderr for anything else).

use crate::ast::{ShareDecl, Statement, Value};
use crate::interpreter::{Interpreter, InterpreterError, ReplScope};
use crate::parser::{ParseError, Parser};
use base64::Engine;
use serde_json::{Value as J, json};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

/// The protocol's version, said in `hello`.
pub const PROTOCOL: u32 = 1;

/// Items sent with a container before `expand` is asked.
const FIRST_ITEMS: usize = 20;
/// Characters of a string sent before `expand`.
const TEXT_CHARS: usize = 2000;
/// Points of a series sent (pairs of a bucket's least and most).
const SERIES_POINTS: usize = 512;
/// Rows a table is sampled over for its columns.
const TABLE_SAMPLE: usize = 200;
/// Evaluations whose handles are kept.
const KEEP_EVALS: usize = 200;
/// The most `expand` answers at once.
const EXPAND_MAX: usize = 2000;

struct Out(Mutex<Box<dyn Write + Send>>);

impl Out {
    fn send(&self, v: &J) {
        let mut line = serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string());
        line.push('\n');
        if let Ok(mut w) = self.0.lock() {
            let _ = w.write_all(line.as_bytes());
            let _ = w.flush();
        }
    }
}

/// Run the protocol on stdio until stdin closes or `shutdown`. Returns
/// the process's exit code.
pub fn serve(no_ovm: bool) -> i32 {
    // The protocol's own stdout: fd 1 kept aside, then pointed at stderr
    // so a child process or a stray write cannot break a reply.
    let out: Box<dyn Write + Send> = {
        #[cfg(unix)]
        {
            use std::os::fd::FromRawFd;
            let _ = std::io::stdout().flush();
            let kept = unsafe { libc::dup(1) };
            if kept >= 0 && unsafe { libc::dup2(2, 1) } >= 0 {
                Box::new(unsafe { std::fs::File::from_raw_fd(kept) })
            } else {
                Box::new(std::io::stdout())
            }
        }
        #[cfg(not(unix))]
        {
            Box::new(std::io::stdout())
        }
    };
    let out = Arc::new(Out(Mutex::new(out)));
    crate::output::collect_output();
    crate::interrupt::arm();

    let running = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel::<J>();
    {
        let out = out.clone();
        let running = running.clone();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                let msg: J = match serde_json::from_str(&line) {
                    Ok(m) => m,
                    Err(e) => {
                        out.send(&json!({ "event": "bad_request", "error": e.to_string() }));
                        continue;
                    }
                };
                // a ping is answered here, even while an evaluation runs:
                // the editor tells a busy session from one not answering
                if msg.get("op").and_then(|o| o.as_str()) == Some("ping") {
                    let busy = running.load(Ordering::SeqCst);
                    out.send(&json!({ "id": msg.get("id").cloned().unwrap_or(J::Null), "ok": true, "running": busy }));
                    continue;
                }
                if msg.get("op").and_then(|o| o.as_str()) == Some("interrupt") {
                    let busy = running.load(Ordering::SeqCst);
                    if busy {
                        crate::interrupt::request();
                    }
                    out.send(&json!({ "id": msg.get("id").cloned().unwrap_or(J::Null), "ok": true, "running": busy }));
                    continue;
                }
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
    }

    let mut session = Session::new(no_ovm);
    out.send(&session.hello(None));
    for msg in rx {
        let op = msg.get("op").and_then(|o| o.as_str()).unwrap_or("").to_string();
        let reply = session.handle(&op, &msg, &running);
        out.send(&reply);
        if op == "shutdown" {
            break;
        }
    }
    0
}

/// A file's declarations as the session knows them (for frames, and for
/// naming a parameter that is not bound).
struct FileIndex {
    stamp: (u64, Option<std::time::SystemTime>),
    /// (name, first line, last line, parameters) of each top-level fn.
    fns: Vec<(String, u32, u32, Vec<String>)>,
    /// Top-level `let`s left unloaded (they call something): name, line.
    skipped: Vec<(String, u32)>,
}

struct Scope {
    scope: ReplScope,
    /// The `use` statements evaluated in it, as source: run again on a
    /// reload so a scope sees a module's new version.
    uses: Vec<String>,
}

struct Session {
    interp: Interpreter,
    parser: Parser,
    root: PathBuf,
    scopes: HashMap<String, Scope>,
    handles: HashMap<u64, Value>,
    next_handle: u64,
    evals: VecDeque<(u64, Vec<u64>)>,
    eval_seq: u64,
    sorts: HashMap<(u64, usize, bool), Arc<Vec<usize>>>,
    files: HashMap<PathBuf, FileIndex>,
    loom: Option<PathBuf>,
    /// Whether the bytecode and native tiers are on (a reload starts
    /// them afresh: compiled code links its callees directly).
    tiers: bool,
}

impl Session {
    fn new(no_ovm: bool) -> Session {
        let mut interp = Interpreter::new();
        if !no_ovm {
            interp.enable_bytecode_tier(1, false);
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let root = crate::pkg::manifest::Manifest::find_root(&cwd).unwrap_or(cwd.clone());
        interp.set_current_file(&root.join("olang.toml"));
        if crate::pkg::manifest::Manifest::find_root(&cwd).is_some() {
            let opts = crate::pkg::InstallOptions {
                registry: std::env::var("OLANG_REGISTRY").ok().map(PathBuf::from),
                ..Default::default()
            };
            let (map, missing) = crate::pkg::install_lenient(&root, &opts);
            interp.set_missing_dependencies(missing.into_iter().collect());
            let mut map: HashMap<_, _> = map.into_iter().collect();
            if let Ok(m) = crate::pkg::manifest::Manifest::load(&root) {
                map.entry(m.package.name.clone()).or_insert(root.clone());
            }
            interp.set_dependency_map(map);
        }
        Session {
            interp,
            parser: Parser::new(),
            root,
            scopes: HashMap::new(),
            handles: HashMap::new(),
            next_handle: 1,
            evals: VecDeque::new(),
            eval_seq: 0,
            sorts: HashMap::new(),
            files: HashMap::new(),
            loom: None,
            tiers: !no_ovm,
        }
    }

    fn hello(&self, id: Option<&J>) -> J {
        let mut v = json!({
            "event": "hello", "protocol": PROTOCOL, "olang": crate::version::VERSION,
            "root": self.root.to_string_lossy(), "pid": std::process::id(),
            "ops": ["hello", "eval", "expand", "release", "interrupt", "ping", "reload", "render", "reset", "shutdown"],
        });
        if let Some(id) = id {
            v["id"] = id.clone();
            v["ok"] = J::Bool(true);
            v.as_object_mut().map(|o| o.remove("event"));
        }
        v
    }

    fn handle(&mut self, op: &str, msg: &J, running: &AtomicBool) -> J {
        let id = msg.get("id").cloned().unwrap_or(J::Null);
        let mut reply = match op {
            "hello" => return self.hello(Some(&id)),
            "eval" => self.eval(msg, running),
            "expand" => self.expand(msg),
            "release" => {
                if let Some(e) = msg.get("eval").and_then(|e| e.as_u64()) {
                    self.drop_eval(e);
                }
                json!({ "ok": true })
            }
            "reload" => self.reload(msg, running),
            "render" => self.render(msg, running),
            "reset" => {
                self.scopes.clear();
                self.handles.clear();
                self.evals.clear();
                self.sorts.clear();
                json!({ "ok": true })
            }
            "shutdown" => json!({ "ok": true }),
            other => json!({ "ok": false, "error": { "kind": "protocol", "message": format!("unknown op \"{}\"", other) } }),
        };
        reply["id"] = id;
        reply
    }

    // ── scopes ──────────────────────────────────────────────────────

    fn take_scope(&mut self, file: Option<&Path>) -> (String, Scope, Vec<String>) {
        let key = file.map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(s) = self.scopes.remove(&key) {
            return (key, s, Vec::new());
        }
        let mut scope = Scope {
            scope: self.interp.new_repl_scope(file),
            uses: Vec::new(),
        };
        let mut notes = Vec::new();
        if let Some(f) = file {
            notes = self.load_declarations(&mut scope, f);
        }
        (key, scope, notes)
    }

    /// A file's declarations evaluated in its scope (from disk). Notes
    /// say what could not be.
    fn load_declarations(&mut self, scope: &mut Scope, file: &Path) -> Vec<String> {
        let Ok(src) = std::fs::read_to_string(file) else {
            return vec![format!("{} could not be read", file.display())];
        };
        let program = match self.parser.parse(&src) {
            Ok(p) => p,
            Err(e) => {
                let (line, _) = parse_position(&e);
                return vec![format!(
                    "{} does not parse (line {}): its declarations are not loaded",
                    short(&self.root, file),
                    line
                )];
            }
        };
        self.index_file(file, &src, &program.statements);
        let mut keep = Vec::new();
        let lines: Vec<&str> = src.lines().collect();
        let starts: Vec<u32> = program.statements.iter().map(stmt_line).collect();
        for (i, st) in program.statements.iter().enumerate() {
            let inner = st.unwrapped();
            let take = match inner {
                Statement::FunctionDecl(_)
                | Statement::UseDecl(_)
                | Statement::TypeDecl(_)
                | Statement::ErrorTypeDecl(_)
                | Statement::TraitDecl(_)
                | Statement::ImplDecl(_) => true,
                Statement::ShareDecl(ShareDecl::Let(_)) | Statement::LetDecl(_) => {
                    !calls_something(&stmt_text(&lines, &starts, i))
                }
                Statement::ShareDecl(_) => true,
                _ => false,
            };
            if take {
                if matches!(inner, Statement::UseDecl(_) | Statement::ShareDecl(ShareDecl::Use(_))) {
                    scope.uses.push(stmt_text(&lines, &starts, i));
                }
                keep.push(st.clone());
            }
        }
        let mut notes = Vec::new();
        self.interp.swap_repl_scope(&mut scope.scope);
        for st in keep {
            let line = stmt_line(&st);
            let one = crate::ast::Program { statements: vec![st] };
            if let Err(e) = self.interp.eval_program(one) {
                notes.push(format!("{}:{}: {}", short(&self.root, file), line, plain_message(&e)));
                let _ = self.interp.take_error_location();
            }
        }
        self.interp.swap_repl_scope(&mut scope.scope);
        notes
    }

    fn index_file(&mut self, file: &Path, src: &str, statements: &[Statement]) {
        let starts: Vec<u32> = statements.iter().map(stmt_line).collect();
        let total = src.lines().count() as u32;
        let mut fns = Vec::new();
        let mut skipped = Vec::new();
        for (i, st) in statements.iter().enumerate() {
            let end = starts.get(i + 1).map(|n| n.saturating_sub(1)).unwrap_or(total);
            match st.unwrapped() {
                Statement::FunctionDecl(f) | Statement::ShareDecl(ShareDecl::Function(f)) => {
                    let params = f.parameters.iter().map(|p| p.name.clone()).collect();
                    fns.push((f.name.clone(), starts[i], end.max(starts[i]), params));
                }
                Statement::LetDecl(l) | Statement::ShareDecl(ShareDecl::Let(l)) => {
                    if let crate::ast::Pattern::Identifier(n) = &l.pattern {
                        skipped.push((n.clone(), starts[i]));
                    }
                }
                _ => {}
            }
        }
        self.files.insert(file.to_path_buf(), FileIndex { stamp: file_stamp(file), fns, skipped });
    }

    /// The index of `file` (read again when it changed on disk).
    fn file_index(&mut self, file: &Path) -> Option<&FileIndex> {
        let fresh = self.files.get(file).map(|f| f.stamp == file_stamp(file)).unwrap_or(false);
        if !fresh {
            let src = std::fs::read_to_string(file).ok()?;
            let program = self.parser.parse(&src).ok()?;
            self.index_file(file, &src, &program.statements);
        }
        self.files.get(file)
    }

    // ── eval ────────────────────────────────────────────────────────

    fn eval(&mut self, msg: &J, running: &AtomicBool) -> J {
        let code = msg.get("code").and_then(|c| c.as_str()).unwrap_or("").to_string();
        let file = msg.get("file").and_then(|f| f.as_str()).map(|f| self.resolve(f));
        let line = msg.get("line").and_then(|l| l.as_u64()).unwrap_or(1).max(1) as usize;
        let t0 = Instant::now();
        let (key, mut scope, notes) = self.take_scope(file.as_deref());
        // the snippet parsed where it stands in its file
        let padded = if file.is_some() && line > 1 { format!("{}{}", "\n".repeat(line - 1), code) } else { code.clone() };
        let program = match self.parser.parse(&padded) {
            Ok(p) => p,
            Err(e) => {
                self.scopes.insert(key, scope);
                let (l, c) = parse_position(&e);
                return json!({ "ok": false, "ms": 0.0, "notes": notes, "error": {
                    "kind": "parse", "message": parse_message(&e), "file": file.as_ref().map(|f| f.to_string_lossy().into_owned()),
                    "line": l, "col": c, "stack": [] } });
            }
        };
        let last = program.statements.last().map(|s| s.unwrapped().clone());
        let uses: Vec<String> = if program
            .statements
            .iter()
            .any(|s| matches!(s.unwrapped(), Statement::UseDecl(_) | Statement::ShareDecl(ShareDecl::Use(_))))
        {
            vec![padded.clone()]
        } else {
            Vec::new()
        };
        let stats0 = self.interp.bytecode_tier_stats();
        self.interp.swap_repl_scope(&mut scope.scope);
        crate::interrupt::clear();
        let _ = crate::output::take_collected();
        running.store(true, Ordering::SeqCst);
        let started = Instant::now();
        let outcome = self.interp.eval_program(program);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        running.store(false, Ordering::SeqCst);
        let interrupted = crate::interrupt::pending();
        crate::interrupt::clear();
        let location = if outcome.is_err() { self.interp.take_error_location() } else { None };
        // a `let`'s value is its binding's
        let bound = match (&outcome, &last) {
            (Ok(_), Some(Statement::LetDecl(l))) | (Ok(_), Some(Statement::ShareDecl(ShareDecl::Let(l)))) => match &l.pattern {
                crate::ast::Pattern::Identifier(n) => Some(n.clone()),
                _ => None,
            },
            _ => None,
        };
        let bound_value = bound.as_ref().and_then(|n| self.interp.get_environment().get(n));
        self.interp.swap_repl_scope(&mut scope.scope);
        if outcome.is_ok() {
            scope.uses.extend(uses);
        }
        let stats1 = self.interp.bytecode_tier_stats();
        let printed = crate::output::take_collected();
        let (bc, nat) = match (stats0, stats1) {
            (Some(a), Some(b)) => (
                b.bytecode_calls.saturating_sub(a.bytecode_calls),
                b.jit_native_calls.saturating_sub(a.jit_native_calls),
            ),
            _ => (0, 0),
        };
        let tier = if nat > 0 { "native" } else if bc > 0 { "bytecode" } else { "interpreter" };
        let mut reply = match outcome {
            Ok(value) => {
                self.eval_seq += 1;
                let seq = self.eval_seq;
                let mut held = Vec::new();
                let shown = match (&last, bound_value) {
                    (_, Some(v)) => self.encode(&v, 0, &mut held),
                    (Some(Statement::FunctionDecl(f)), _) | (Some(Statement::ShareDecl(ShareDecl::Function(f))), _) => json!({
                        "k": "decl", "t": "Function",
                        "s": format!("fn {}({})", f.name, f.parameters.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")) }),
                    (Some(Statement::Expression(_)), _) => self.encode(&value, 0, &mut held),
                    (Some(Statement::UseDecl(_)), _) => json!({ "k": "decl", "t": "Use", "s": "use" }),
                    _ => json!({ "k": "unit", "t": "Unit", "s": "()" }),
                };
                self.evals.push_back((seq, held));
                while self.evals.len() > KEEP_EVALS {
                    let (old, _) = self.evals[0];
                    self.drop_eval(old);
                }
                let mut r = json!({ "ok": true, "eval": seq, "value": shown });
                if let Some(n) = bound {
                    r["binding"] = J::String(n);
                }
                r
            }
            Err(e) => {
                let err = self.error_json(&e, location, file.as_deref(), line, interrupted);
                json!({ "ok": false, "error": err })
            }
        };
        self.scopes.insert(key, scope);
        reply["ms"] = json!(round3(ms));
        reply["server_ms"] = json!(round3(t0.elapsed().as_secs_f64() * 1000.0));
        reply["tier"] = J::String(tier.to_string());
        reply["calls"] = json!({ "bytecode": bc, "native": nat });
        reply["out"] = J::String(printed);
        if !notes.is_empty() {
            reply["notes"] = json!(notes);
        }
        reply
    }

    fn error_json(
        &mut self,
        e: &InterpreterError,
        loc: Option<crate::ast::ErrorLocation>,
        file: Option<&Path>,
        at_line: usize,
        interrupted: bool,
    ) -> J {
        let mut message = plain_message(e);
        let mut kind = if interrupted || crate::interrupt::is_interrupt(&message) { "interrupted" } else { "runtime" };
        if kind == "interrupted" {
            message = "interrupted".to_string();
        }
        let (line, col, efile, stack, hint) = match &loc {
            Some(l) => {
                let f = l.file.as_ref().map(|f| self.resolve(f)).or_else(|| file.map(|f| f.to_path_buf()));
                (l.line, l.column, f, l.call_stack.clone(), l.hint.clone())
            }
            None => (at_line as u32, 1, file.map(|f| f.to_path_buf()), Vec::new(), None),
        };
        // a parameter of the function the snippet stands in, not bound
        // here: the REPL evaluates code as written (SPEC §18.1 is open)
        if let (InterpreterError::UndefinedVariable { name }, Some(f)) = (e, file)
            && let Some(ix) = self.file_index(f)
        {
            if let Some((fname, ..)) = ix
                .fns
                .iter()
                .find(|(_, a, b, ps)| (*a as usize) < at_line && at_line <= *b as usize && ps.contains(name))
            {
                message = format!("`{}` is a parameter of `{}` — not bound here", name, fname);
                kind = "unbound";
            } else if let Some((_, l)) = ix.skipped.iter().find(|(n, _)| n == name) {
                message = format!("`{}` is a top-level binding that runs code — evaluate its line ({}) first", name, l);
                kind = "unbound";
            }
        }
        let frames: Vec<J> = stack
            .iter()
            .map(|frame| {
                let (name, ffile) = match frame.rfind(" (") {
                    Some(i) if frame.ends_with(')') => (frame[..i].to_string(), Some(self.resolve(&frame[i + 2..frame.len() - 1]))),
                    _ => (frame.clone(), None),
                };
                let place = self.locate_fn(&name, ffile.as_deref().or(file));
                json!({ "name": name, "file": place.as_ref().map(|p| p.0.to_string_lossy().into_owned()),
                        "line": place.as_ref().map(|p| p.1) })
            })
            .collect();
        json!({ "kind": kind, "message": message, "file": efile.map(|f| f.to_string_lossy().into_owned()),
                "line": line, "col": col, "stack": frames, "hint": hint })
    }

    /// Where function `name` is declared: in `near`'s file, else the
    /// function value's own file.
    fn locate_fn(&mut self, name: &str, near: Option<&Path>) -> Option<(PathBuf, u32)> {
        let bare = name.rsplit('.').next().unwrap_or(name).to_string();
        if let Some(f) = near
            && let Some(ix) = self.file_index(f)
            && let Some((_, l, ..)) = ix.fns.iter().find(|(n, ..)| *n == bare)
        {
            return Some((f.to_path_buf(), *l));
        }
        let def = self
            .scopes
            .values()
            .find_map(|s| match s.scope.get(&bare) {
                Some(Value::Function(ref f)) => f.def_file.clone(),
                _ => None,
            })?;
        let path = self.resolve(&def);
        let l = self.file_index(&path)?.fns.iter().find(|(n, ..)| *n == bare)?.1;
        Some((path, l))
    }

    fn resolve(&self, f: &str) -> PathBuf {
        let p = Path::new(f);
        if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) }
    }

    // ── reload ──────────────────────────────────────────────────────

    fn reload(&mut self, msg: &J, running: &AtomicBool) -> J {
        let t0 = Instant::now();
        let Some(path) = msg.get("path").and_then(|p| p.as_str()).map(|p| self.resolve(p)) else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "reload needs a path" } });
        };
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => return json!({ "ok": false, "error": { "kind": "io", "message": e.to_string(), "file": path.to_string_lossy() } }),
        };
        // a file that no longer parses changes nothing
        if let Err(e) = self.parser.parse(&src) {
            let (l, c) = parse_position(&e);
            return json!({ "ok": false, "kept": true, "error": { "kind": "parse", "message": parse_message(&e),
                           "file": path.to_string_lossy(), "line": l, "col": c, "stack": [] } });
        }
        let key = path.to_string_lossy().into_owned();
        let mut problems: Vec<String> = Vec::new();
        running.store(true, Ordering::SeqCst);
        crate::interrupt::clear();
        if let Some(mut scope) = self.scopes.remove(&key) {
            scope.uses.clear();
            problems.extend(self.load_declarations(&mut scope, &path));
            self.scopes.insert(key.clone(), scope);
        }
        // every other scope's `use`s again: the module cache sees the new
        // content and loads it; a module that fails keeps the old values
        let keys: Vec<String> = self.scopes.keys().filter(|k| **k != key).cloned().collect();
        let mut touched = 0;
        for k in keys {
            let mut scope = self.scopes.remove(&k).expect("listed");
            let uses = scope.uses.clone();
            if !uses.is_empty() {
                touched += 1;
            }
            // a file's functions close over what they saw declared: its
            // declarations again (its `use`s among them), so they call the
            // module loaded now
            if !k.is_empty() && !uses.is_empty() {
                scope.uses.clear();
                problems.extend(self.load_declarations(&mut scope, Path::new(&k)));
                self.scopes.insert(k, scope);
                continue;
            }
            self.interp.swap_repl_scope(&mut scope.scope);
            for u in &uses {
                if let Ok(p) = self.parser.parse(u) {
                    // only the `use` statements of what was evaluated
                    let only: Vec<Statement> = p
                        .statements
                        .into_iter()
                        .filter(|s| matches!(s.unwrapped(), Statement::UseDecl(_) | Statement::ShareDecl(ShareDecl::Use(_))))
                        .collect();
                    if let Err(e) = self.interp.eval_program(crate::ast::Program { statements: only }) {
                        problems.push(plain_message(&e));
                        let _ = self.interp.take_error_location();
                    }
                }
            }
            self.interp.swap_repl_scope(&mut scope.scope);
            self.scopes.insert(k, scope);
        }
        running.store(false, Ordering::SeqCst);
        self.files.remove(&path);
        // compiled code calls its callees directly (the native tier links
        // a group of functions together): start the tiers afresh, so
        // what runs next is compiled from what is loaded now
        if self.tiers {
            self.interp.enable_bytecode_tier(1, false);
        }
        let _ = crate::output::take_collected();
        let ms = round3(t0.elapsed().as_secs_f64() * 1000.0);
        if problems.is_empty() {
            json!({ "ok": true, "path": key, "scopes": touched, "ms": ms })
        } else {
            json!({ "ok": false, "kept": true, "path": key, "ms": ms,
                    "error": { "kind": "reload", "message": problems.join("\n"), "file": key, "stack": [] } })
        }
    }

    // ── values ──────────────────────────────────────────────────────

    fn hold(&mut self, v: &Value, held: &mut Vec<u64>) -> u64 {
        let h = self.next_handle;
        self.next_handle += 1;
        self.handles.insert(h, v.clone());
        held.push(h);
        h
    }

    fn drop_eval(&mut self, seq: u64) {
        if let Some(i) = self.evals.iter().position(|(s, _)| *s == seq)
            && let Some((_, hs)) = self.evals.remove(i)
        {
            for h in hs {
                self.handles.remove(&h);
                self.sorts.retain(|k, _| k.0 != h);
            }
        }
    }

    /// A value as the protocol carries it (docs/tooling.md, "Values").
    fn encode(&mut self, v: &Value, depth: usize, held: &mut Vec<u64>) -> J {
        let t = v.type_name();
        match v {
            Value::Integer(i) => json!({ "k": "int", "t": t, "s": i.to_string() }),
            Value::Float(_) => json!({ "k": "float", "t": t, "s": v.to_string() }),
            Value::Boolean(b) => json!({ "k": "bool", "t": t, "s": b.to_string() }),
            Value::Unit => json!({ "k": "unit", "t": t, "s": "()" }),
            Value::String(s) => {
                let n = s.chars().count();
                let mut r = json!({ "k": "str", "t": t, "n": n, "s": s.chars().take(TEXT_CHARS).collect::<String>() });
                if n > TEXT_CHARS {
                    r["h"] = json!(self.hold(v, held));
                }
                if let Some(p) = self.image_path(s) {
                    r["img"] = J::String(p);
                }
                r
            }
            Value::Ok(inner) | Value::Err(inner) => {
                let ok = matches!(v, Value::Ok(_));
                json!({ "k": "result", "t": t, "ok": ok, "v": self.encode(inner, depth, held) })
            }
            Value::List(items) | Value::Tuple(items) => {
                let kind = if matches!(v, Value::List(_)) { "list" } else { "tuple" };
                let h = self.hold(v, held);
                let mut r = json!({ "k": kind, "t": t, "n": items.len(), "h": h, "s": preview(v) });
                if depth < 2 {
                    let first: Vec<J> = items.iter().take(FIRST_ITEMS).map(|x| self.encode(x, depth + 1, held)).collect();
                    r["items"] = J::Array(first);
                }
                if let Some(cols) = table_columns(items) {
                    r["table"] = json!({ "cols": cols });
                } else if let Some(series) = series_of(items) {
                    r["series"] = series;
                }
                r
            }
            Value::Range { start, end, inclusive } => {
                let n = (if *inclusive { end - start + 1 } else { end - start }).max(0);
                json!({ "k": "range", "t": t, "n": n, "s": v.to_string() })
            }
            Value::Map(m) => {
                if let Some(role) = loom_view_role(m) {
                    let h = self.hold(v, held);
                    return json!({ "k": "view", "t": "View", "role": role, "h": h, "s": format!("a Loom {} view", role) });
                }
                let h = self.hold(v, held);
                let mut r = json!({ "k": "map", "t": t, "n": m.len(), "h": h, "s": preview(v) });
                if depth < 2 {
                    let mut keys: Vec<&String> = m.keys().collect();
                    keys.sort();
                    let entries: Vec<J> = keys
                        .into_iter()
                        .take(FIRST_ITEMS)
                        .map(|k| json!({ "key": k, "v": self.encode(&m[k], depth + 1, held) }))
                        .collect();
                    r["entries"] = J::Array(entries);
                }
                r
            }
            Value::Struct { fields, .. } => {
                let h = self.hold(v, held);
                let mut r = json!({ "k": "record", "t": t, "n": fields.len(), "h": h, "s": preview(v) });
                if depth < 2 {
                    let mut keys: Vec<&String> = fields.keys().collect();
                    keys.sort();
                    let entries: Vec<J> = keys
                        .into_iter()
                        .take(FIRST_ITEMS)
                        .map(|k| json!({ "key": k, "v": self.encode(&fields[k], depth + 1, held) }))
                        .collect();
                    r["entries"] = J::Array(entries);
                }
                r
            }
            Value::Enum(_) => json!({ "k": "enum", "t": t, "s": preview(v) }),
            Value::Function(f) => {
                let ps: Vec<&str> = f.parameters.iter().map(|p| p.name.as_str()).collect();
                json!({ "k": "fn", "t": t, "s": format!("fn {}({})", f.name.as_deref().unwrap_or(""), ps.join(", ")) })
            }
            Value::Builtin(b) => json!({ "k": "fn", "t": t, "s": format!("builtin {}", b.name) }),
            Value::EnumConstructor(_) | Value::TypeInfo(_) => json!({ "k": "fn", "t": t, "s": preview(v) }),
            Value::Native(_) => match crate::stdlib::bytes::bytes_of(v) {
                Ok(b) => {
                    let h = self.hold(v, held);
                    let mut r = json!({ "k": "bytes", "t": "Bytes", "n": b.len(), "h": h, "s": format!("{} bytes", b.len()) });
                    if let Some((fmt, w, ht)) = image_kind(b) {
                        r["img"] = json!({ "format": fmt, "w": w, "h": ht });
                        if b.len() <= 16 * 1024 * 1024 {
                            r["png"] = J::String(base64::engine::general_purpose::STANDARD.encode(b));
                        }
                    }
                    r
                }
                Err(_) => json!({ "k": "native", "t": t, "s": preview(v) }),
            },
        }
    }

    /// A string that names a picture on disk: its absolute path.
    fn image_path(&self, s: &str) -> Option<String> {
        if s.len() > 4096 || s.contains('\n') {
            return None;
        }
        let lower = s.to_ascii_lowercase();
        let pic = [".png", ".jpg", ".jpeg", ".gif", ".webp"].iter().any(|e| lower.ends_with(e));
        if !pic {
            return None;
        }
        let p = self.resolve(s);
        p.is_file().then(|| p.to_string_lossy().into_owned())
    }

    fn expand(&mut self, msg: &J) -> J {
        let Some(h) = msg.get("h").and_then(|h| h.as_u64()) else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "expand needs a handle (h)" } });
        };
        let Some(v) = self.handles.get(&h).cloned() else {
            return json!({ "ok": false, "error": { "kind": "gone", "message": "that value is no longer held (released, or the REPL restarted)" } });
        };
        let start = msg.get("start").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
        let count = (msg.get("count").and_then(|s| s.as_u64()).unwrap_or(100) as usize).min(EXPAND_MAX);
        let mut held = Vec::new();
        let reply = match &v {
            Value::String(s) => {
                let text: String = s.chars().skip(start).take(count.max(1) * 100).collect();
                json!({ "ok": true, "s": text })
            }
            Value::List(items) | Value::Tuple(items) => {
                if msg.get("table").and_then(|t| t.as_bool()) == Some(true) {
                    let cols = table_columns(items).unwrap_or_default();
                    let order = match msg.get("sort").and_then(|s| s.get("col")).and_then(|c| c.as_u64()) {
                        Some(c) => {
                            let desc = msg["sort"].get("desc").and_then(|d| d.as_bool()).unwrap_or(false);
                            Some(self.sorted(h, items, &cols, c as usize, desc))
                        }
                        None => None,
                    };
                    let n = items.len();
                    let rows: Vec<J> = (start..(start + count).min(n))
                        .map(|i| {
                            let at = order.as_ref().map(|o| o[i]).unwrap_or(i);
                            let cells: Vec<J> = cols.iter().map(|c| J::String(cell_text(&items[at], c))).collect();
                            json!({ "i": at, "c": cells })
                        })
                        .collect();
                    json!({ "ok": true, "n": n, "cols": cols, "rows": rows })
                } else {
                    let n = items.len();
                    let out: Vec<J> = (start..(start + count).min(n))
                        .map(|i| json!({ "i": i, "v": self.encode(&items[i], 1, &mut held) }))
                        .collect();
                    json!({ "ok": true, "n": n, "items": out })
                }
            }
            Value::Map(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let n = keys.len();
                let out: Vec<J> = keys
                    .into_iter()
                    .skip(start)
                    .take(count)
                    .map(|k| json!({ "key": k, "v": self.encode(&m[k], 1, &mut held) }))
                    .collect();
                json!({ "ok": true, "n": n, "entries": out })
            }
            Value::Struct { fields, .. } => {
                let mut keys: Vec<&String> = fields.keys().collect();
                keys.sort();
                let n = keys.len();
                let out: Vec<J> = keys
                    .into_iter()
                    .skip(start)
                    .take(count)
                    .map(|k| json!({ "key": k, "v": self.encode(&fields[k], 1, &mut held) }))
                    .collect();
                json!({ "ok": true, "n": n, "entries": out })
            }
            Value::Native(_) => match crate::stdlib::bytes::bytes_of(&v) {
                Ok(b) => json!({ "ok": true, "n": b.len(), "b64": base64::engine::general_purpose::STANDARD.encode(b) }),
                Err(_) => json!({ "ok": false, "error": { "kind": "protocol", "message": "nothing to expand" } }),
            },
            _ => json!({ "ok": false, "error": { "kind": "protocol", "message": "nothing to expand" } }),
        };
        // what an expansion holds lives as long as the evaluation it came from
        if !held.is_empty() {
            if let Some(entry) = self.evals.iter_mut().find(|(_, hs)| hs.contains(&h)) {
                entry.1.extend(held);
            } else {
                for x in held {
                    self.handles.remove(&x);
                }
            }
        }
        reply
    }

    /// A table's rows ordered by column `c` (cached by handle).
    fn sorted(&mut self, h: u64, items: &[Value], cols: &[String], c: usize, desc: bool) -> Arc<Vec<usize>> {
        if let Some(o) = self.sorts.get(&(h, c, desc)) {
            return o.clone();
        }
        let col = cols.get(c).cloned().unwrap_or_default();
        let keys: Vec<Value> = items.iter().map(|r| field_of(r, &col).unwrap_or(Value::Unit)).collect();
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by(|a, b| {
            let o = compare_cells(&keys[*a], &keys[*b]);
            if desc { o.reverse() } else { o }
        });
        let o = Arc::new(order);
        self.sorts.insert((h, c, desc), o.clone());
        o
    }

    // ── views ───────────────────────────────────────────────────────

    fn render(&mut self, msg: &J, running: &AtomicBool) -> J {
        let t0 = Instant::now();
        let Some(h) = msg.get("h").and_then(|h| h.as_u64()) else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "render needs a handle (h)" } });
        };
        let Some(view) = self.handles.get(&h).cloned() else {
            return json!({ "ok": false, "error": { "kind": "gone", "message": "that view is no longer held" } });
        };
        // Loom: the project's dependency, else the path given
        let own = crate::pkg::manifest::Manifest::load(&self.root)
            .map(|m| m.dependencies.contains_key("loom") || m.package.name == "loom")
            .unwrap_or(false);
        if let Some(p) = msg.get("loom").and_then(|p| p.as_str())
            && !own
            && self.loom.as_deref() != Some(Path::new(p))
        {
            let lroot = PathBuf::from(p);
            let opts = crate::pkg::InstallOptions::default();
            let (map, _missing) = crate::pkg::install_lenient(&lroot, &opts);
            let mut map: HashMap<_, _> = map.into_iter().collect();
            map.insert("loom".to_string(), lroot.clone());
            self.interp.add_to_dependency_map(map);
            self.loom = Some(lroot);
        }
        let opts = json!({
            "dark": msg.get("dark").and_then(|d| d.as_bool()).unwrap_or(false),
            "contrast": msg.get("contrast").and_then(|d| d.as_bool()).unwrap_or(false),
            "scale": msg.get("scale").and_then(|d| d.as_f64()).unwrap_or(2.0),
            "width": msg.get("width").and_then(|d| d.as_f64()).unwrap_or(640.0),
            "max_height": msg.get("max_height").and_then(|d| d.as_f64()).unwrap_or(1600.0),
        });
        let (key, mut scope, _) = self.take_scope(None);
        self.interp.swap_repl_scope(&mut scope.scope);
        self.interp.define_global("__studio_view", view);
        self.interp.define_global("__studio_opts", json_to_value(&opts));
        crate::interrupt::clear();
        running.store(true, Ordering::SeqCst);
        let program = self
            .parser
            .parse("use loom.lib.test { render_view as __studio_render_view }\n__studio_render_view(__studio_view, __studio_opts)");
        let outcome = match program {
            Ok(p) => self.interp.eval_program(p),
            Err(e) => Err(InterpreterError::RuntimeError { message: parse_message(&e) }),
        };
        running.store(false, Ordering::SeqCst);
        let _ = self.interp.take_error_location();
        self.interp.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        let _ = crate::output::take_collected();
        let ms = round3(t0.elapsed().as_secs_f64() * 1000.0);
        match outcome {
            Ok(Value::Map(ref m)) => {
                let png = m.get("png").and_then(|p| crate::stdlib::bytes::bytes_of(p).ok().map(|b| b.to_vec()));
                let num = |k: &str| match m.get(k) {
                    Some(Value::Integer(i)) => *i as f64,
                    Some(Value::Float(f)) => *f,
                    _ => 0.0,
                };
                match png {
                    Some(b) => json!({ "ok": true, "png": base64::engine::general_purpose::STANDARD.encode(&b),
                                       "width": num("width"), "height": num("height"), "scale": opts["scale"], "ms": ms }),
                    None => json!({ "ok": false, "ms": ms, "error": { "kind": "render", "message": "render_view answered no picture" } }),
                }
            }
            Ok(other) => json!({ "ok": false, "ms": ms, "error": { "kind": "render", "message": format!("render_view answered {}", other.type_name()) } }),
            Err(e) => {
                let mut message = plain_message(&e);
                if message.contains("loom") && (message.contains("not found") || message.contains("Module")) {
                    message = format!("{} — the project has no `loom` dependency; pass its path as `loom`", message);
                }
                json!({ "ok": false, "ms": ms, "error": { "kind": "render", "message": message } })
            }
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn short(root: &Path, f: &Path) -> String {
    f.strip_prefix(root).unwrap_or(f).to_string_lossy().into_owned()
}

fn file_stamp(f: &Path) -> (u64, Option<std::time::SystemTime>) {
    match std::fs::metadata(f) {
        Ok(m) => (m.len(), m.modified().ok()),
        Err(_) => (0, None),
    }
}

/// The source lines of top-level statement `i` (to the next one's).
fn stmt_text(lines: &[&str], starts: &[u32], i: usize) -> String {
    let a = (starts[i].max(1) as usize - 1).min(lines.len());
    let b = starts.get(i + 1).map(|n| (*n as usize).saturating_sub(1)).unwrap_or(lines.len()).clamp(a, lines.len());
    lines[a..b.max(a + 1).min(lines.len())].join("\n")
}

fn stmt_line(s: &Statement) -> u32 {
    match s {
        Statement::Located { line, .. } => *line,
        Statement::DecoratedDecl { line, .. } => *line,
        _ => 0,
    }
}

/// An error's message without the "Runtime error: " its Display adds.
fn plain_message(e: &InterpreterError) -> String {
    match e {
        InterpreterError::RuntimeError { message } => message.clone(),
        other => other.to_string(),
    }
}

fn parse_position(e: &ParseError) -> (usize, usize) {
    match e {
        ParseError::InvalidSyntaxWithPosition { line, column, .. }
        | ParseError::UnexpectedTokenWithPosition { line, column, .. } => (*line, *column),
        ParseError::Pest(p) => match p.line_col {
            pest::error::LineColLocation::Pos((l, c)) => (l, c),
            pest::error::LineColLocation::Span((l, c), _) => (l, c),
        },
        _ => (1, 1),
    }
}

/// A parse error's first line (its message, without the snippet).
fn parse_message(e: &ParseError) -> String {
    match e {
        ParseError::InvalidSyntaxWithPosition { message, .. } => message.clone(),
        ParseError::UnexpectedTokenWithPosition { token, .. } => format!("unexpected {}", token),
        ParseError::Pest(p) => p.variant.message().to_string(),
        other => other.to_string().lines().next().unwrap_or("").to_string(),
    }
}

/// Whether a declaration's text calls something (a call, a pipe): such a
/// top-level `let` is not loaded with its file's declarations — it may
/// read a file, start a window, or take a minute.
fn calls_something(text: &str) -> bool {
    let rhs = match text.find('=') {
        Some(i) => &text[i + 1..],
        None => return false,
    };
    let mut prev = ' ';
    let mut in_str = false;
    let mut chars = rhs.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            if c == '\\' {
                chars.next();
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '/' if chars.peek() == Some(&'/') => {
                // a comment to the line's end
                for d in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
            }
            '(' if prev.is_alphanumeric() || prev == '_' || prev == ')' || prev == ']' => return true,
            '>' if prev == '|' => return true,
            _ => {}
        }
        if !c.is_whitespace() {
            prev = c;
        }
    }
    false
}

/// A short one-line form of a value (at most 200 characters).
fn preview(v: &Value) -> String {
    let s = v.to_string();
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 200 {
        format!("{}…", one.chars().take(199).collect::<String>())
    } else {
        one
    }
}

const LOOM_ROLES: &[&str] = &[
    "window", "group", "separator", "text", "heading", "button", "input", "textarea", "checkbox", "list", "table",
    "dialog", "popover", "menu", "select", "toast", "tabs", "segmented", "tab_bar", "split", "log", "tree", "canvas",
    "image", "region", "memo", "slider", "progress", "icon", "link", "toggle", "radio",
];

/// A Loom view node (`#{ role, key, props, children }`): its role.
fn loom_view_role(m: &crate::ast::ValueMap) -> Option<String> {
    if m.len() != 4 || !m.contains_key("props") || !m.contains_key("children") || !m.contains_key("key") {
        return None;
    }
    match m.get("role") {
        Some(Value::String(r)) if LOOM_ROLES.contains(&r.as_str()) => Some(r.to_string()),
        _ => None,
    }
}

/// The columns of a list of maps or records: their keys, in the order
/// first met over the first rows (at most 64).
fn table_columns(items: &[Value]) -> Option<Vec<String>> {
    if items.is_empty() {
        return None;
    }
    let mut cols: Vec<String> = Vec::new();
    for it in items.iter().take(TABLE_SAMPLE) {
        let fields = match it {
            Value::Map(m) if loom_view_role(m).is_none() => m.as_ref(),
            Value::Struct { fields, .. } => fields.as_ref(),
            _ => return None,
        };
        let mut keys: Vec<&String> = fields.keys().collect();
        keys.sort();
        for k in keys {
            if cols.len() < 64 && !cols.contains(k) {
                cols.push(k.clone());
            }
        }
    }
    (!cols.is_empty()).then_some(cols)
}

fn field_of(row: &Value, col: &str) -> Option<Value> {
    match row {
        Value::Map(m) => m.get(col).cloned(),
        Value::Struct { fields, .. } => fields.get(col).cloned(),
        _ => None,
    }
}

fn cell_text(row: &Value, col: &str) -> String {
    match field_of(row, col) {
        None => String::new(),
        Some(Value::String(ref s)) => s.chars().take(200).collect(),
        Some(v) => preview(&v),
    }
}

fn compare_cells(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Value::Unit, Value::Unit) => Ordering::Equal,
        (Value::Unit, _) => Ordering::Greater,
        (_, Value::Unit) => Ordering::Less,
        _ => a.compare_for_sort(b).unwrap_or_else(|| a.to_string().cmp(&b.to_string())),
    }
}

/// A list of numbers (or of `(x, number)` pairs) as a series: the values'
/// range and at most SERIES_POINTS of them (each bucket's least and most
/// past that).
fn series_of(items: &[Value]) -> Option<J> {
    if items.len() < 2 {
        return None;
    }
    let num = |v: &Value| match v {
        Value::Integer(i) => Some(*i as f64),
        Value::Float(f) if f.is_finite() => Some(*f),
        _ => None,
    };
    let ys: Option<Vec<f64>> = items
        .iter()
        .map(|v| match v {
            Value::Tuple(t) if t.len() == 2 => num(&t[1]),
            other => num(other),
        })
        .collect();
    let ys = ys?;
    let lo = ys.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let pts: Vec<f64> = if ys.len() <= SERIES_POINTS {
        ys.clone()
    } else {
        let buckets = SERIES_POINTS / 2;
        let per = ys.len() as f64 / buckets as f64;
        let mut out = Vec::with_capacity(SERIES_POINTS);
        for b in 0..buckets {
            let a = (b as f64 * per) as usize;
            let z = (((b + 1) as f64 * per) as usize).min(ys.len()).max(a + 1);
            let s = &ys[a..z];
            let mn = s.iter().cloned().fold(f64::INFINITY, f64::min);
            let mx = s.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            out.push(mn);
            out.push(mx);
        }
        out
    };
    Some(json!({ "lo": lo, "hi": hi, "pts": pts, "dated": matches!(items[0], Value::Tuple(_)) }))
}

/// A picture's format and size, from its first bytes.
fn image_kind(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    if b.len() > 24 && b.starts_with(&[0x89, b'P', b'N', b'G']) {
        let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
        let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
        return Some(("png", w, h));
    }
    if b.len() > 10 && b.starts_with(b"GIF8") {
        let w = u16::from_le_bytes([b[6], b[7]]) as u32;
        let h = u16::from_le_bytes([b[8], b[9]]) as u32;
        return Some(("gif", w, h));
    }
    if b.len() > 4 && b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        // the size is in a frame header further in; the picture reads it
        return Some(("jpeg", 0, 0));
    }
    if b.len() > 30 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(("webp", 0, 0));
    }
    None
}

fn json_to_value(j: &J) -> Value {
    match j {
        J::Null => Value::Unit,
        J::Bool(b) => Value::Boolean(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) if !n.to_string().contains('.') => Value::Integer(i),
            _ => Value::Float(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => Value::String(Arc::new(s.clone())),
        J::Array(a) => Value::List(Arc::new(a.iter().map(json_to_value).collect())),
        J::Object(o) => {
            let mut m = crate::ast::ValueMap::default();
            for (k, v) in o {
                m.insert(k.clone(), json_to_value(v));
            }
            Value::Map(Arc::new(m))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_let_that_calls_is_not_loaded() {
        assert!(calls_something("let x = load(\"a\")"));
        assert!(calls_something("let x = xs |> len"));
        assert!(!calls_something("let x = #{ \"a\": (y) => y + 1 }"));
        assert!(!calls_something("let s = \"f(x)\" // g(y)"));
        assert!(!calls_something("share let N = 3"));
    }

    #[test]
    fn numbers_are_a_series_and_maps_a_table() {
        let nums: Vec<Value> = (0..2000).map(|i| Value::Float(i as f64)).collect();
        let s = series_of(&nums).unwrap();
        assert_eq!(s["pts"].as_array().unwrap().len(), SERIES_POINTS);
        assert_eq!(s["hi"], json!(1999.0));
        let mut m = crate::ast::ValueMap::default();
        m.insert("b".into(), Value::Integer(1));
        m.insert("a".into(), Value::Integer(2));
        let rows = vec![Value::Map(Arc::new(m))];
        assert_eq!(table_columns(&rows), Some(vec!["a".to_string(), "b".to_string()]));
    }
}

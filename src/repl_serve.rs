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
//! - **A function's parameters.** A snippet that stands inside a
//!   function and names a parameter is refused as not bound, the
//!   function and its parameters said (`fn`, `params`); `bind` gives
//!   them values for one evaluation (`{"x": "3"}`: each an expression,
//!   evaluated in the file's scope first) and takes them back after.
//! - **Purity.** `pure: true` evaluates under no capability at all (no
//!   filesystem, network, process, database or environment): code that
//!   reaches for one is refused with `kind` `impure` and the capability
//!   it wanted, and the session's own grant is back afterwards. An editor
//!   re-runs on save only what passes.
//! - **Views.** `render` draws a Loom view value headless (Loom's
//!   `render_view`, from the project's dependency or a path given) and
//!   answers a PNG.
//!
//! - **Commands.** Every `:` command of the interactive REPL runs through
//!   the REPL itself (`Repl::run_command`, headless), in the session's
//!   own scope: what it says is captured (src/stdio.rs) and streamed,
//!   then sent as text and styled lines with what it found besides
//!   (src/repl_commands.rs has the registry, completion, styled runs).
//!
//! - **A notebook's kernel.** `--notebook x.olmd` makes the session the
//!   notebook's: its folder the working directory, its front matter's
//!   params bound and grant applied (with the project's and `OLANG_DENY`,
//!   as `olang run` takes them), and every evaluation in the notebook's
//!   scope. An `eval` with `cell` is that cell's: what it defines is the
//!   cell's, and evaluating the cell again drops those definitions first.
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
/// The furthest line a request may place code on (it is parsed after as
/// many line breaks).
const LINE_MAX: usize = 1_000_000;

/// A request's `line`, 1 at least and `LINE_MAX` at most.
fn asked_line(msg: &J) -> usize {
    msg.get("line")
        .and_then(|l| l.as_u64())
        .unwrap_or(1)
        .clamp(1, LINE_MAX as u64) as usize
}

/// The frames of an error's stack a reply names: its first and last.
const STACK_HEAD: usize = 40;
const STACK_TAIL: usize = 10;

struct Out(Mutex<Box<dyn Write + Send>>);

impl Out {
    fn send(&self, v: &J) {
        let mut line = serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string());
        line.push('\n');
        if let Ok(mut w) = self.0.lock() {
            // no one reads the replies any more: the session ends
            if let Err(e) = w.write_all(line.as_bytes()).and_then(|_| w.flush())
                && e.kind() == std::io::ErrorKind::BrokenPipe
            {
                std::process::exit(0);
            }
        }
    }
}

/// Run the protocol on stdio until stdin closes or `shutdown`. Returns
/// the process's exit code. `notebook`: the session is that notebook's
/// kernel (its folder the working directory).
pub fn serve(no_ovm: bool, notebook: Option<PathBuf>) -> i32 {
    // a notebook's kernel runs in the notebook's folder, as `olang run`
    // runs a notebook: its project is the folder's
    let notebook = match notebook {
        None => None,
        Some(p) => {
            let abs = std::path::absolute(&p).unwrap_or(p);
            if !abs.is_file() || !crate::notebook::is_notebook(&abs) {
                eprintln!(
                    "olang repl --serve --notebook: {} is no notebook (an .olmd file)",
                    abs.display()
                );
                return 2;
            }
            if let Some(dir) = abs.parent()
                && let Err(e) = std::env::set_current_dir(dir)
            {
                eprintln!("olang repl --serve: cannot enter {}: {}", dir.display(), e);
                return 2;
            }
            Some(abs)
        }
    };
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
                // a cell's names read from its text: parsing only, answered
                // at once (an editor asks as a person types, while a cell runs)
                if msg.get("op").and_then(|o| o.as_str()) == Some("deps") {
                    let mut reply = deps_reply(&msg);
                    reply["id"] = msg.get("id").cloned().unwrap_or(J::Null);
                    out.send(&reply);
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
            // stdin closed: the requests sent before it are answered (the
            // loop below drains them, then ends); an editor that died is
            // seen by the parent's watch below, a reader gone by stdout
        });
    }
    // the parent gone without closing our stdin (it was handed on to a
    // process of its own): ended too, within a second
    #[cfg(unix)]
    {
        let parent = unsafe { libc::getppid() };
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if unsafe { libc::getppid() } != parent {
                    std::process::exit(0);
                }
            }
        });
    }

    let mut session = Session::new(no_ovm, Some(out.clone()));
    if let Some(nb) = &notebook {
        session.open_notebook(nb);
    }
    out.send(&session.hello(None));
    for msg in rx {
        let op = msg
            .get("op")
            .and_then(|o| o.as_str())
            .unwrap_or("")
            .to_string();
        let reply = session.handle(&op, &msg, &running);
        out.send(&reply);
        if op == "shutdown" {
            break;
        }
    }
    0
}

/// A notebook run cell by cell, as its kernel runs it (`olang notebook
/// export`, `olang notebook pin`): each code cell evaluated in order in the
/// notebook's own scope — its params bound (`overrides` replace them), its
/// grant, its folder — and a control cell named by an override set to it
/// as Studio sets one; then every `${expr}` of its prose evaluated under no
/// capability. Answers the outputs document (src/notebook_export.rs): each
/// cell's reply as the kernel gave it. The process enters the notebook's
/// folder. A notebook with problems, or an override naming nothing, is an
/// error (the problems as `olang run` prints them).
pub fn run_notebook(path: &Path, overrides: &[(String, String)]) -> Result<J, String> {
    let abs = std::path::absolute(path).unwrap_or(path.to_path_buf());
    if !abs.is_file() || !crate::notebook::is_notebook(&abs) {
        return Err(format!("{} is no notebook (an .olmd file)", abs.display()));
    }
    let text = std::fs::read_to_string(&abs).map_err(|e| format!("{}: {}", abs.display(), e))?;
    let nb = crate::notebook::parse(&text);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let code_of = |c: &crate::notebook::Cell| -> String {
        let from = c.fence_line as usize;
        let to = (c.close_line as usize - 1).min(lines.len());
        lines[from.min(to)..to].join("\n")
    };
    // params and controls apart: a param's override is the program's, a
    // control's is set as its cell runs
    type Pairs = Vec<(String, String)>;
    let (params_over, controls_over): (Pairs, Pairs) = overrides
        .iter()
        .cloned()
        .partition(|(n, _)| nb.params.iter().any(|p| &p.name == n));
    let controls: Vec<(usize, String)> = nb
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.no_run)
        .filter_map(|(i, c)| crate::notebook::control_cell_name(&code_of(c)).map(|n| (i, n)))
        .collect();
    for (n, _) in &controls_over {
        if !controls.iter().any(|(_, c)| c == n) {
            let declared: Vec<String> = nb
                .params
                .iter()
                .map(|p| p.name.clone())
                .chain(controls.iter().map(|c| c.1.clone()))
                .collect();
            return Err(format!(
                "{}: --param {}: the notebook has no param or control of that name{}",
                abs.display(),
                n,
                if declared.is_empty() {
                    String::new()
                } else {
                    format!(" (its params and controls: {})", declared.join(", "))
                }
            ));
        }
    }
    let program = crate::notebook::program_with(&text, crate::notebook::Mode::Run, &params_over);
    if !program.diagnostics.is_empty() {
        return Err(crate::notebook::report(&abs, &program.diagnostics));
    }
    if let Some(dir) = abs.parent() {
        std::env::set_current_dir(dir)
            .map_err(|e| format!("cannot enter {}: {}", dir.display(), e))?;
    }
    crate::output::collect_output();
    let running = AtomicBool::new(false);
    let mut session = Session::new(false, None);
    session.open_notebook(&abs);
    // the params as overridden (validated by the program above)
    if let Some(info) = session.notebook.as_mut() {
        for (name, value) in &params_over {
            if let Some(p) = info.params.iter_mut().find(|p| &p.0 == name) {
                p.1 = crate::notebook::param_value_source(value);
            }
        }
    }
    let hash = |code: &str| {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(code.as_bytes()))[..16].to_string()
    };
    let mut cells = Vec::new();
    let mut controls_said = Vec::new();
    for (i, c) in nb.cells.iter().enumerate() {
        let code = code_of(c);
        let id = format!("c{}", i + 1);
        let line = c.fence_line + 1;
        let mut entry =
            json!({ "id": id, "name": c.options.name, "line": line, "code": hash(&code) });
        if c.no_run {
            entry["reply"] = J::Null;
            cells.push(entry);
            continue;
        }
        let msg = json!({ "op": "eval", "cell": id, "code": code, "line": line });
        let mut reply = session.eval(&msg, &running);
        // a control named by an override: set as an editor sets it, the cell
        // run again so its reply says the value
        if let Some((_, name)) = controls.iter().find(|(k, _)| *k == i)
            && let Some((_, value)) = controls_over.iter().find(|(n, _)| n == name)
        {
            let set = session.set_control(
                &json!({ "cell": id, "value": crate::notebook::param_value_source(value) }),
            );
            if set.get("ok").and_then(|o| o.as_bool()) != Some(true) {
                let m = set
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("refused");
                return Err(format!(
                    "{}:{}: --param {}: {}",
                    abs.display(),
                    line,
                    name,
                    m
                ));
            }
            reply = session.eval(&msg, &running);
        }
        if let Some(ctl) = reply.get("control")
            && ctl.get("kind").and_then(|k| k.as_str()) != Some("brush")
            && let Some(name) = ctl.get("name").and_then(|n| n.as_str())
        {
            let v = ctl
                .get("value")
                .map(crate::notebook_export::value_text)
                .unwrap_or_default();
            controls_said.push(json!({ "name": name, "value": v }));
        }
        entry["reply"] = reply;
        cells.push(entry);
    }
    let mut interp = Vec::new();
    for e in crate::notebook_export::prose_exprs(&text) {
        if interp
            .iter()
            .any(|x: &J| x.get("expr").and_then(|x| x.as_str()) == Some(e.as_str()))
        {
            continue;
        }
        let reply = session.eval(&json!({ "op": "eval", "code": e, "pure": true }), &running);
        interp.push(json!({ "expr": e, "reply": reply }));
    }
    let params: Vec<J> = session
        .notebook
        .as_ref()
        .map(|n| {
            n.params
                .iter()
                .map(|p| json!({ "name": p.0, "value": p.1 }))
                .collect()
        })
        .unwrap_or_default();
    let label = controls_said
        .iter()
        .chain(params.iter())
        .filter_map(|p| {
            Some(format!(
                "{} {}",
                p.get("name")?.as_str()?,
                crate::notebook_export::unquote(p.get("value")?.as_str()?)
            ))
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(
        json!({ "v": 1, "at": at, "label": label, "notebook": abs.to_string_lossy(), "title": nb.title,
        "params": params, "controls": controls_said, "cells": cells, "interp": interp }),
    )
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

/// The notebook a kernel serves (`--notebook`).
struct NotebookInfo {
    path: PathBuf,
    title: Option<String>,
    /// (name, its value as olang source, its front matter line)
    params: Vec<(String, String, u32)>,
    /// The notebook's own problems as olang reads it (a bad option, front
    /// matter outside the subset): said in `hello`, the session served.
    problems: Vec<crate::notebook::Diagnostic>,
    /// Its grant could not be read: the session runs under none.
    refused: bool,
    /// The front matter's grant as read (to tell when it changed).
    grant: Option<crate::caps::Caps>,
}

struct Scope {
    scope: ReplScope,
    /// The `use` statements evaluated in it, as source: run again on a
    /// reload so a scope sees a module's new version.
    uses: Vec<String>,
}

struct Session {
    /// The REPL itself, headless: its interpreter is the session's, and
    /// every `:` command runs through it as the terminal runs it.
    cmd: crate::repl::Repl,
    /// The protocol's stdout, for what a command says while it runs.
    out: Option<Arc<Out>>,
    parser: Parser,
    root: PathBuf,
    scopes: HashMap<String, Scope>,
    handles: HashMap<u64, Value>,
    next_handle: u64,
    evals: VecDeque<(u64, Vec<u64>)>,
    eval_seq: u64,
    sorts: HashMap<(u64, usize, bool), Arc<Vec<usize>>>,
    /// A Frame's filtered (and sorted) views' row orders, by handle and
    /// the view's filters and sort as JSON text.
    views: HashMap<(u64, String), Arc<Vec<usize>>>,
    files: HashMap<PathBuf, FileIndex>,
    loom: Option<PathBuf>,
    /// The library's names, for completion (made the first time asked).
    library: Option<Vec<String>>,
    /// Whether the bytecode and native tiers are on (a reload starts
    /// them afresh: compiled code links its callees directly).
    tiers: bool,
    /// The project's dependencies (name → folder), for a notebook's grant.
    deps: HashMap<String, PathBuf>,
    /// The notebook this session is the kernel of, if any.
    notebook: Option<NotebookInfo>,
    /// What each cell defined when it was last evaluated, by its scope and
    /// its id: dropped before the cell is evaluated again.
    cells: HashMap<(String, String), Vec<String>>,
    /// Where a cell's function is declared (its file and line), for frames.
    declared: HashMap<String, (PathBuf, u32)>,
    /// The control each cell bound when it was last evaluated (one bound
    /// alone), and the name it binds when the cell defines one alone.
    controls: HashMap<String, CellControl>,
    /// A recorded run being made, or one being replayed (`record`,
    /// `replay`): each cell's evaluation records its inputs, or is served
    /// them.
    timeline: Option<KernelTimeline>,
    /// The params that could not be bound, and why: an evaluation that
    /// reads one says so (it said "a cell not run yet").
    unbound_params: HashMap<String, String>,
}

/// A kernel's record or replay (docs/tooling.md, "A recorded run").
enum KernelTimeline {
    /// Each cell's stretch of events as it was evaluated.
    Record {
        cells: Vec<crate::timeline::TraceCell>,
        events: Vec<crate::timeline::Event>,
    },
    /// The trace served, and what each cell's replay found.
    Replay { trace: crate::timeline::Trace },
}

/// One cell's evaluation under the timeline: the tier kept aside, and in
/// a replay the stretch it is served from.
struct Stretch {
    kept: Option<Box<crate::ovm::tier::BytecodeTier>>,
    cell: String,
    code: String,
    line: u32,
    /// Replay: the recorded cell's index, and how many of its events the
    /// main thread makes.
    from: Option<(usize, usize)>,
}

/// A cell's control, as `set` validates a value for it.
struct CellControl {
    kind: String,
    default: Value,
    spec: Value,
    /// The cell's one name (none: several controls or names).
    name: Option<String>,
}

impl Session {
    fn new(no_ovm: bool, out: Option<Arc<Out>>) -> Session {
        // every evaluation keeps tier statistics (the `stats` op)
        crate::profile::stats_enable();
        let mut interp = Interpreter::new();
        if !no_ovm {
            interp.enable_bytecode_tier(1, false);
        }
        // every evaluation says the capabilities it reached (its purity):
        // the gate's own classification, on every tier
        interp.enable_caps_trace();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let root = crate::pkg::manifest::Manifest::find_root(&cwd).unwrap_or(cwd.clone());
        interp.set_current_file(&root.join("olang.toml"));
        let mut deps = HashMap::new();
        if crate::pkg::manifest::Manifest::find_root(&cwd).is_some() {
            // what this machine holds, fetched from nowhere: opening a
            // project must not clone what its manifest names
            let opts = crate::pkg::InstallOptions {
                registry: std::env::var("OLANG_REGISTRY").ok().map(PathBuf::from),
                offline: true,
                ..Default::default()
            };
            let (map, missing) = crate::pkg::install_lenient(&root, &opts);
            interp.set_missing_dependencies(missing.into_iter().collect());
            let mut map: HashMap<_, _> = map.into_iter().collect();
            if let Ok(m) = crate::pkg::manifest::Manifest::load(&root) {
                map.entry(m.package.name.clone()).or_insert(root.clone());
            }
            deps = map.clone();
            interp.set_dependency_map(map);
        }
        Session {
            cmd: crate::repl::Repl::headless(interp),
            out,
            parser: Parser::new(),
            root,
            scopes: HashMap::new(),
            handles: HashMap::new(),
            next_handle: 1,
            evals: VecDeque::new(),
            eval_seq: 0,
            sorts: HashMap::new(),
            views: HashMap::new(),
            files: HashMap::new(),
            loom: None,
            tiers: !no_ovm,
            library: None,
            deps,
            notebook: None,
            cells: HashMap::new(),
            declared: HashMap::new(),
            controls: HashMap::new(),
            timeline: None,
            unbound_params: HashMap::new(),
        }
    }

    /// The session made `path`'s kernel: its params kept (bound as its
    /// scope is made), its grant applied — the front matter's, intersected
    /// with the enclosing project's (and each dependency's attenuation) and
    /// `OLANG_DENY`'s, as `olang run` takes them; a grant that cannot be
    /// read, under no capability at all.
    fn open_notebook(&mut self, path: &Path) {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let nb = crate::notebook::parse(&text);
        if let Some(t) = self.grant_table(&nb) {
            self.cmd.interpreter.set_capabilities(t);
        }
        self.cmd.interpreter.set_current_file(path);
        self.notebook = Some(NotebookInfo {
            path: path.to_path_buf(),
            title: nb.title.clone(),
            params: nb
                .params
                .iter()
                .map(|p| (p.name.clone(), p.value.clone(), p.line))
                .collect(),
            problems: nb.diagnostics.clone(),
            refused: nb.capabilities_refused,
            grant: nb.capabilities,
        });
        self.note_param_problems(&text, &nb.diagnostics);
    }

    /// A front matter's problems on a param's line (a value outside its
    /// range, of no kind it takes): the param is not bound, and a cell
    /// that reads it says so and why (it said "a cell not run yet").
    fn note_param_problems(&mut self, text: &str, problems: &[crate::notebook::Diagnostic]) {
        let lines: Vec<&str> = text.lines().collect();
        for d in problems {
            let Some(l) = (d.line as usize).checked_sub(1).and_then(|i| lines.get(i)) else {
                continue;
            };
            let Some((key, _)) = l.trim_start().split_once(':') else {
                continue;
            };
            let key = key.trim();
            if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                self.unbound_params
                    .insert(key.to_string(), d.message.clone());
            }
        }
    }

    /// The grant a notebook's front matter makes the session's: the front
    /// matter's, intersected with the enclosing project's (and each
    /// dependency's attenuation) and `OLANG_DENY`'s; a grant that cannot be
    /// read, none at all. `None`: nothing restricts the session.
    fn grant_table(&self, nb: &crate::notebook::Notebook) -> Option<crate::caps::CapTable> {
        let deny = std::env::var("OLANG_DENY")
            .ok()
            .map(|d| crate::caps::parse_deny(&d).unwrap_or_else(|_| crate::caps::Caps::none()));
        let grant = if nb.capabilities_refused {
            Some(crate::caps::Caps::none())
        } else {
            nb.capabilities
        };
        let restrict = match (grant, deny) {
            (Some(a), Some(b)) => Some(a.intersect(b)),
            (a, b) => a.or(b),
        };
        match crate::pkg::manifest::Manifest::load_for_capabilities(&self.root) {
            Ok(m) => match m.and_then(|m| m.capabilities.clone()) {
                Some(config) => Some(match crate::caps::CapTable::build(&config, &self.deps) {
                    Ok(mut t) => {
                        if let Some(r) = restrict {
                            t.app = t.app.intersect(r);
                            for e in &mut t.deps {
                                e.2 = e.2.intersect(r);
                            }
                        }
                        t
                    }
                    // the project's grant cannot be honoured: none
                    Err(_) => crate::caps::CapTable {
                        app: crate::caps::Caps::none(),
                        deps: Vec::new(),
                        fixed: false,
                    },
                }),
                None => restrict.map(|r| crate::caps::CapTable {
                    app: r,
                    deps: Vec::new(),
                    fixed: false,
                }),
            },
            Err(_) => Some(crate::caps::CapTable {
                app: crate::caps::Caps::none(),
                deps: Vec::new(),
                fixed: false,
            }),
        }
    }

    /// The notebook's front matter read again (`text`, the editor's unsaved
    /// buffer, else the file): each param whose value changed bound again in
    /// the notebook's scope on its line, a param gone forgotten, the grant
    /// applied anew. The reply says which params changed and whether the
    /// grant did, so an editor marks what they reach.
    fn reread(&mut self, msg: &J) -> J {
        let Some(path) = self.notebook_path() else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "reread is a notebook's kernel's (--notebook)" } });
        };
        let text = match msg.get("text").and_then(|t| t.as_str()) {
            Some(t) => t.to_string(),
            None => std::fs::read_to_string(&path).unwrap_or_default(),
        };
        let nb = crate::notebook::parse(&text);
        let info = self.notebook.as_ref().expect("a kernel's notebook");
        let before = info.params.clone();
        let grant_changed =
            info.grant != nb.capabilities || info.refused != nb.capabilities_refused;
        let now: Vec<(String, String, u32)> = nb
            .params
            .iter()
            .map(|p| (p.name.clone(), p.value.clone(), p.line))
            .collect();
        let changed: Vec<(String, String, u32)> = now
            .iter()
            .filter(|p| !before.iter().any(|b| b.0 == p.0 && b.1 == p.1))
            .cloned()
            .collect();
        let removed: Vec<String> = before
            .iter()
            .filter(|b| !now.iter().any(|p| p.0 == b.0))
            .map(|b| b.0.clone())
            .collect();
        // the scope as it is (made with the params as they were when it is new)
        let (key, mut scope, mut notes) = self.take_scope(Some(&path));
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        for name in &removed {
            self.cmd.interpreter.forget_top_level(name);
        }
        for (name, value, line) in &changed {
            let code = format!(
                "{}let {} = {}",
                "\n".repeat(*line as usize - 1),
                name,
                value
            );
            let outcome = match self.parser.parse(&code) {
                Ok(p) => self.cmd.interpreter.eval_program(p),
                Err(e) => Err(InterpreterError::RuntimeError {
                    message: parse_message(&e),
                }),
            };
            if let Err(e) = outcome {
                let _ = self.cmd.interpreter.take_error_location();
                self.unbound_params
                    .insert(name.to_string(), plain_message(&e));
                notes.push(format!(
                    "{}:{}: the param `{}` is not bound: {}",
                    short(&self.root, &path),
                    line,
                    name,
                    plain_message(&e)
                ));
            } else {
                self.unbound_params.remove(name.as_str());
            }
        }
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        if grant_changed {
            match self.grant_table(&nb) {
                Some(t) => self.cmd.interpreter.set_capabilities(t),
                None => {
                    let _ = self.cmd.interpreter.replace_capabilities(None);
                }
            }
        }
        let info = self.notebook.as_mut().expect("a kernel's notebook");
        info.title = nb.title.clone();
        info.params = now;
        info.problems = nb.diagnostics.clone();
        info.refused = nb.capabilities_refused;
        self.unbound_params
            .retain(|n, _| nb.params.iter().all(|p| p.name != *n));
        self.note_param_problems(&text, &nb.diagnostics);
        let info = self.notebook.as_mut().expect("a kernel's notebook");
        info.grant = nb.capabilities;
        let mut r = json!({
            "ok": true,
            "params": changed.iter().map(|p| p.0.clone()).collect::<Vec<_>>(),
            "removed": removed,
            "grant": grant_changed,
            "problems": nb.diagnostics.iter().map(|d| json!({ "line": d.line, "column": d.column, "message": d.message })).collect::<Vec<_>>(),
        });
        if !notes.is_empty() {
            r["notes"] = json!(notes);
        }
        r
    }

    /// A cell's definitions forgotten (a cell deleted): `{"cell"}`, in the
    /// notebook's scope (or a `file`'s). The reply says what was dropped.
    fn drop_cell(&mut self, msg: &J) -> J {
        let Some(c) = msg.get("cell").map(|c| match c.as_str() {
            Some(s) => s.to_string(),
            None => c.to_string(),
        }) else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "drop needs a cell" } });
        };
        let file = msg
            .get("file")
            .and_then(|f| f.as_str())
            .map(|f| self.resolve(f))
            .or_else(|| self.notebook_path());
        let (key, mut scope, _) = self.take_scope(file.as_deref());
        // its control's host value forgotten with it
        crate::controls::forget(&c);
        self.controls.remove(&c);
        let names = self.cells.remove(&(key.clone(), c)).unwrap_or_default();
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        for name in &names {
            self.cmd.interpreter.forget_top_level(name);
            self.declared.remove(name);
        }
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        json!({ "ok": true, "dropped": names })
    }

    /// The notebook's names and their values: each cell's definitions (and
    /// the params), with the value as the protocol carries it (its handle
    /// held, its first items left to `expand`) and its size in memory,
    /// roughly.
    fn vars(&mut self) -> J {
        let Some(path) = self.notebook_path() else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "vars is a notebook's kernel's (--notebook)" } });
        };
        let key = path.to_string_lossy().into_owned();
        let params: Vec<String> = self
            .notebook
            .as_ref()
            .map(|n| n.params.iter().map(|p| p.0.clone()).collect())
            .unwrap_or_default();
        let mut owned: Vec<(String, String)> = self
            .cells
            .iter()
            .filter(|((k, _), _)| *k == key)
            .flat_map(|((_, c), names)| names.iter().map(move |n| (c.clone(), n.clone())))
            .collect();
        owned.sort();
        let (skey, mut scope, _) = self.take_scope(Some(&path));
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        let mut found: Vec<(Option<String>, String, Value)> = Vec::new();
        for p in &params {
            if let Some(v) = self.cmd.interpreter.get_environment().get(p) {
                found.push((None, p.clone(), v));
            }
        }
        for (c, n) in &owned {
            if let Some(v) = self.cmd.interpreter.get_environment().get(n) {
                found.push((Some(c.clone()), n.clone(), v));
            }
        }
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(skey, scope);
        self.eval_seq += 1;
        let seq = self.eval_seq;
        let mut held = Vec::new();
        let vars: Vec<J> = found
            .iter()
            .map(|(c, n, v)| {
                let enc = self.encode(v, 2, &mut held);
                let mut r = json!({ "name": n, "v": enc, "bytes": approx_bytes(v) });
                match c {
                    Some(c) => r["cell"] = J::String(c.clone()),
                    None => r["param"] = J::Bool(true),
                }
                r
            })
            .collect();
        self.evals.push_back((seq, held));
        while self.evals.len() > KEEP_EVALS {
            let (old, _) = self.evals[0];
            self.drop_eval(old);
        }
        json!({ "ok": true, "eval": seq, "vars": vars })
    }

    /// What a value held by name is made of, for an editor's completion:
    /// a Frame's columns with their types, a map's keys or a record's
    /// fields with their values' types, a list of maps' columns. `name`
    /// is read in the notebook's scope (or a `file`'s, or the session's),
    /// `path` walks keys into it (`m.inner` is `name: "m", path:
    /// ["inner"]`). Nothing is evaluated: a name that is not bound is an
    /// `unbound` error; a value of another kind has no fields.
    fn fields(&mut self, msg: &J) -> J {
        if let Some(code) = msg.get("code").and_then(|c| c.as_str()) {
            return self.fields_at(code, msg);
        }
        let Some(name) = msg.get("name").and_then(|n| n.as_str()).map(str::to_string) else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "fields needs a name (or code)" } });
        };
        let path: Vec<String> = msg
            .get("path")
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|k| k.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let file = msg
            .get("file")
            .and_then(|f| f.as_str())
            .map(|f| self.resolve(f));
        let (key, mut scope, _) = self.take_scope(file.as_deref());
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        let found = self.cmd.interpreter.get_environment().get(&name);
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        let Some(mut v) = found else {
            return json!({ "ok": false, "name": name, "error": { "kind": "unbound", "message": format!("`{name}` is not bound in this session") } });
        };
        for k in &path {
            let inner = match &v {
                Value::Map(m) => m.get(k).cloned(),
                Value::Struct { fields, .. } => fields.get(k).cloned(),
                _ => None,
            };
            match inner {
                Some(x) => v = x,
                None => {
                    return json!({ "ok": true, "name": name, "path": path, "kind": "none", "t": v.type_name(), "fields": [] });
                }
            }
        }
        let (kind, fields) = value_fields(&v);
        let total = fields.len();
        let shown: Vec<J> = fields
            .into_iter()
            .take(FIELDS_MAX)
            .map(|(n, t)| json!({ "name": n, "type": t }))
            .collect();
        let mut r =
            json!({ "ok": true, "name": name, "kind": kind, "t": v.type_name(), "fields": shown });
        if !path.is_empty() {
            r["path"] = json!(path);
        }
        if total > FIELDS_MAX {
            r["more"] = json!(total - FIELDS_MAX);
        }
        r
    }

    /// The fields that complete at the end of `code` (the text before an
    /// editor's caret): the place read as the language server reads it
    /// (`tools::fields`), the receiver's fields from the values held — a
    /// name not held from `known` (an editor's cached copy: `{"sales":
    /// {"kind": "frame", "fields": [{"name", "type"}…]}}`), else from
    /// what `code` itself binds.
    fn fields_at(&mut self, code: &str, msg: &J) -> J {
        use crate::tools::fields;
        let Some(ctx) = fields::context(code) else {
            return json!({ "ok": true, "context": false });
        };
        let file = msg
            .get("file")
            .and_then(|f| f.as_str())
            .map(|f| self.resolve(f));
        let known = msg.get("known").cloned().unwrap_or(J::Null);
        let (key, mut scope, _) = self.take_scope(file.as_deref());
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        let mut from_live = Vec::new();
        let found = {
            let env = self.cmd.interpreter.get_environment();
            let mut depth = 0u32;
            let mut resolve = |n: &str| -> Option<fields::Fields> {
                depth += 1;
                if depth > 16 {
                    return None;
                }
                if let Some(v) = env.get(n) {
                    let (kind, fs) = value_fields(&v);
                    if !matches!(kind, "none" | "list" | "series" | "module") {
                        from_live.push(n.to_string());
                        return Some(fields::Fields { kind, fields: fs });
                    }
                    return None;
                }
                if let Some(k) = known.get(n) {
                    let kind = match k.get("kind").and_then(|x| x.as_str()) {
                        Some("frame") => "frame",
                        Some("map") => "map",
                        Some("record") => "record",
                        Some("table") => "table",
                        _ => return None,
                    };
                    let fs = k
                        .get("fields")
                        .and_then(|f| f.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|f| {
                                    Some((
                                        f.get("name")?.as_str()?.to_string(),
                                        f.get("type")
                                            .and_then(|t| t.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    ))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    return Some(fields::Fields { kind, fields: fs });
                }
                let init = fields::binding_text(code, n)?;
                fields::fields_of(&init, &mut |_: &str| None)
            };
            fields::fields_of(&ctx.receiver, &mut resolve)
        };
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        let want = match ctx.want {
            fields::Want::Columns => "columns",
            fields::Want::Keys => "keys",
            fields::Want::Any => "any",
        };
        let mut r = json!({ "ok": true, "context": true, "receiver": ctx.receiver, "want": want,
                            "typed": ctx.typed, "quoted": ctx.quoted, "live": !from_live.is_empty() });
        match found {
            Some(f) if f.answers(ctx.want) => {
                r["kind"] = json!(f.kind);
                r["fields"] = J::Array(
                    f.fields
                        .into_iter()
                        .take(FIELDS_MAX)
                        .map(|(n, t)| json!({ "name": n, "type": t }))
                        .collect(),
                );
            }
            _ => r["fields"] = json!([]),
        }
        r
    }

    /// A bound control as an eval's reply says it: its kind, the cell's
    /// name (when it binds one alone), its value and default as values are
    /// encoded, and what its kind has of min, max, step, options and a
    /// brush's chart (its description, decimated).
    fn control_json(
        &mut self,
        b: &crate::controls::Bound,
        name: Option<&str>,
        held: &mut Vec<u64>,
    ) -> J {
        let mut o = json!({ "kind": b.kind });
        if let Some(n) = name {
            o["name"] = J::String(n.to_string());
        }
        o["value"] = self.encode(&b.value, 1, held);
        o["default"] = self.encode(&b.default, 1, held);
        if let Value::Map(m) = &b.spec {
            for k in ["min", "max", "step"] {
                match m.get(k) {
                    Some(Value::Integer(i)) => o[k] = json!(i),
                    Some(Value::Float(f)) => o[k] = json!(f),
                    _ => {}
                }
            }
            if let Some(Value::List(opts)) = m.get("options") {
                let enc: Vec<J> = opts.iter().map(|x| self.encode(x, 1, held)).collect();
                o["options"] = J::Array(enc);
            }
            if let Some(Value::String(svg)) = m.get("chart")
                && let Some(mut c) = chart_of(svg)
            {
                decimate_chart(&mut c, None);
                o["chart"] = c;
                // the chart itself, as a page draws it (an export, an app)
                if let Some(sv) = svg_of(svg) {
                    o["svg"] = sv;
                }
            } else if let Some(v) = m.get("chart")
                && crate::ods::gg::is_spec(v)
                && let Ok(scene) = crate::ods::gg::scene_of(v, None)
            {
                // a gg plot's: its scene (what an editor draws and brushes
                // natively, a range in the dragged panel's data) and its SVG
                o["chart"] = serde_json::from_str(&scene.to_json(&olang_ods::gg::SceneJsonOptions::default()))
                    .unwrap_or(J::Null);
                if let Some(sv) = svg_of(&scene.to_svg()) {
                    o["svg"] = sv;
                }
            }
        }
        o
    }

    /// `set`: the value an editor chose for a cell's control (`value`, an
    /// olang literal, evaluated under no capability), validated against the
    /// control, bound to the cell's name in place and kept as the cell's
    /// host value, so running the cell again keeps it.
    fn set_control(&mut self, msg: &J) -> J {
        let fail =
            |kind: &str, m: String| json!({ "ok": false, "error": { "kind": kind, "message": m } });
        let Some(c) = msg.get("cell").map(|c| match c.as_str() {
            Some(s) => s.to_string(),
            None => c.to_string(),
        }) else {
            return fail("protocol", "set needs a cell".to_string());
        };
        let Some(src) = msg
            .get("value")
            .and_then(|v| v.as_str())
            .map(str::to_string)
        else {
            return fail(
                "protocol",
                "set needs a value (an olang literal, as text)".to_string(),
            );
        };
        let Some(ctl) = self.controls.get(&c) else {
            return fail(
                "protocol",
                format!("the cell {} bound no control when it last ran", c),
            );
        };
        let Some(name) = ctl.name.clone() else {
            return fail(
                "invalid",
                format!(
                    "the cell {}'s control binds no one name (the cell binds several controls or defines several names)",
                    c
                ),
            );
        };
        let (kind, default, spec) = (ctl.kind.clone(), ctl.default.clone(), ctl.spec.clone());
        let program = match self.parser.parse(&src) {
            Ok(p)
                if p.statements.len() == 1
                    && matches!(p.statements[0].unwrapped(), Statement::Expression(_)) =>
            {
                p
            }
            Ok(_) => return fail("invalid", format!("`{}` is no value (one expression)", src)),
            Err(e) => {
                return fail(
                    "invalid",
                    format!("`{}` does not parse: {}", src, parse_message(&e)),
                );
            }
        };
        let file = self.notebook_path();
        let (key, mut scope, _) = self.take_scope(file.as_deref());
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        // evaluated purely: a value, reaching nothing
        let kept =
            self.cmd
                .interpreter
                .replace_capabilities(Some(Arc::new(crate::caps::CapTable {
                    app: crate::caps::Caps::none(),
                    deps: Vec::new(),
                    fixed: false,
                })));
        let value = self.cmd.interpreter.eval_program(program);
        self.cmd.interpreter.replace_capabilities(kept);
        let outcome = match value {
            Ok(v) => {
                crate::controls::validate(&kind, &default, &spec, &v).map_err(|m| ("invalid", m))
            }
            Err(e) => {
                let _ = self.cmd.interpreter.take_error_location();
                Err(("invalid", format!("`{}` fails: {}", src, plain_message(&e))))
            }
        };
        if let Ok(v) = &outcome {
            self.cmd
                .interpreter
                .define_variable(name.clone(), v.clone());
        }
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        let _ = crate::output::take_collected();
        match outcome {
            Ok(v) => {
                crate::controls::set_value(
                    &c,
                    crate::controls::HostValue {
                        kind,
                        default,
                        value: v.clone(),
                    },
                );
                self.eval_seq += 1;
                let seq = self.eval_seq;
                let mut held = Vec::new();
                let enc = self.encode(&v, 0, &mut held);
                self.evals.push_back((seq, held));
                while self.evals.len() > KEEP_EVALS {
                    let (old, _) = self.evals[0];
                    self.drop_eval(old);
                }
                json!({ "ok": true, "eval": seq, "name": name, "value": enc })
            }
            Err((k, m)) => fail(k, m),
        }
    }

    /// A profile's frames named for where a cell declared them: a cell's
    /// function's `file` and `line` are the notebook's (its text as the
    /// kernel ran it, not the file on disk).
    fn cell_frames(&self, mut doc: J) -> J {
        if let Some(frames) = doc.get_mut("frames").and_then(|f| f.as_array_mut()) {
            for f in frames.iter_mut() {
                let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let bare = name
                    .strip_prefix("<lambda in ")
                    .and_then(|r| r.strip_suffix('>'))
                    .unwrap_or(name)
                    .to_string();
                if let Some((file, line)) = self.declared.get(&bare) {
                    f["file"] = J::String(file.to_string_lossy().to_string());
                    f["line"] = json!(line);
                }
            }
        }
        doc
    }

    // ── a recorded run ──────────────────────────────────────────────

    /// `record`: from now each cell's evaluation records what it read (the
    /// clock, random numbers, the environment, files, processes, the
    /// network, the database); `record` with `path` writes the trace there
    /// and stops. `{"stop": true}` stops without writing.
    fn record(&mut self, msg: &J) -> J {
        let path = msg
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| self.resolve(p));
        let stop = msg.get("stop").and_then(|s| s.as_bool()) == Some(true);
        if path.is_none() && !stop {
            self.timeline = Some(KernelTimeline::Record {
                cells: Vec::new(),
                events: Vec::new(),
            });
            return json!({ "ok": true, "recording": true });
        }
        let Some(KernelTimeline::Record { cells, events }) = self.timeline.take() else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "nothing is being recorded (`record` starts it)" } });
        };
        let Some(path) = path else {
            return json!({ "ok": true, "recording": false });
        };
        // the notebook's program as it ran: its params as bound now
        let nb_path = self.notebook_path();
        let text = nb_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        let params: Vec<(String, String)> = self
            .notebook
            .as_ref()
            .map(|n| {
                n.params
                    .iter()
                    .map(|p| (p.0.clone(), p.1.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let source = if nb_path.is_some() {
            crate::notebook::program_with(&text, crate::notebook::Mode::Run, &params).source
        } else {
            cells
                .iter()
                .map(|c| c.code.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let controls: Vec<J> = self
            .controls
            .iter()
            .filter_map(|(c, ctl)| ctl.name.as_ref().map(|n| (c, n)))
            .filter_map(|(c, n)| {
                let v = self.cmd.interpreter.get_environment().get(n)?;
                Some(json!({ "cell": c, "name": n, "value": literal_text(&v) }))
            })
            .collect();
        let sha = {
            use sha2::{Digest, Sha256};
            format!("{:x}", Sha256::digest(source.as_bytes()))
        };
        let n_events = events.len();
        let n_cells = cells.len();
        let trace = crate::timeline::Trace {
            format: crate::timeline::TRACE_FORMAT,
            olang_version: crate::version::VERSION.to_string(),
            program_path: nb_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            program_sha256: sha,
            source,
            events,
            cells,
            notebook: Some(json!({
                "params": params.iter().map(|(n, v)| json!({ "name": n, "value": v })).collect::<Vec<_>>(),
                "controls": controls,
            })),
        };
        let written = serde_json::to_vec(&trace)
            .map_err(|e| e.to_string())
            .and_then(|b| {
                if let Some(d) = path.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                std::fs::write(&path, &b)
                    .map(|_| b.len())
                    .map_err(|e| e.to_string())
            });
        match written {
            Ok(bytes) => {
                json!({ "ok": true, "path": path.to_string_lossy(), "cells": n_cells, "events": n_events, "bytes": bytes })
            }
            Err(e) => {
                json!({ "ok": false, "error": { "kind": "io", "message": format!("the trace cannot be written to {}: {}", path.display(), e) } })
            }
        }
    }

    /// `replay` with `path`: from now each cell's evaluation is served what
    /// the trace recorded for it (the cell found by its id and code, else
    /// its code, else its id) — never the clock, the disk or a process.
    /// `{"stop": true}` ends it.
    fn replay(&mut self, msg: &J) -> J {
        if msg.get("stop").and_then(|s| s.as_bool()) == Some(true) {
            let was = matches!(self.timeline, Some(KernelTimeline::Replay { .. }));
            if was {
                self.timeline = None;
            }
            return json!({ "ok": true, "replaying": false, "was": was });
        }
        let Some(path) = msg
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| self.resolve(p))
        else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "`replay` takes the trace's `path`" } });
        };
        match crate::timeline::Timeline::load_trace(&path) {
            Ok(trace) => {
                let cells: Vec<J> = trace
                    .cells
                    .iter()
                    .map(|c| json!({ "cell": c.cell, "line": c.line, "events": c.to - c.from }))
                    .collect();
                let reply = json!({ "ok": true, "replaying": true, "olang": trace.olang_version, "cells": cells,
                    "events": trace.events.len(), "notebook": trace.notebook.clone().unwrap_or(J::Null) });
                self.timeline = Some(KernelTimeline::Replay { trace });
                reply
            }
            Err(e) => json!({ "ok": false, "error": { "kind": "io", "message": e } }),
        }
    }

    /// A cell's evaluation begun under the timeline, if there is one.
    fn stretch_begin(&mut self, cell: &str, code: &str, line: u32) -> Option<Stretch> {
        let (timeline, from) = match self.timeline.as_ref()? {
            KernelTimeline::Record { .. } => (
                crate::timeline::Timeline::record(
                    PathBuf::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                ),
                None,
            ),
            KernelTimeline::Replay { trace, .. } => {
                let at = trace
                    .cells
                    .iter()
                    .position(|c| c.cell == cell && c.code == code)
                    .or_else(|| trace.cells.iter().position(|c| c.code == code))
                    .or_else(|| trace.cells.iter().position(|c| c.cell == cell));
                let events: Vec<crate::timeline::Event> = match at {
                    Some(i) => trace.events[trace.cells[i].from..trace.cells[i].to].to_vec(),
                    None => Vec::new(),
                };
                let main = events.iter().filter(|e| e.thread == 0).count();
                let t = crate::timeline::Trace {
                    events,
                    cells: Vec::new(),
                    notebook: None,
                    ..trace.clone()
                };
                crate::timeline::clear_diverged();
                (
                    crate::timeline::Timeline::replay(t),
                    Some((at.unwrap_or(usize::MAX), main)),
                )
            }
        };
        let kept = self.cmd.interpreter.begin_timeline_stretch(timeline);
        Some(Stretch {
            kept,
            cell: cell.to_string(),
            code: code.to_string(),
            line,
            from,
        })
    }

    /// The stretch ended: a recording's events kept as the cell's, a
    /// replay's verdict. Answers the reply's key (`record`, `replay`) and
    /// what it says.
    fn stretch_end(
        &mut self,
        st: Stretch,
        outcome: &Result<Value, InterpreterError>,
    ) -> (&'static str, J) {
        let t = self.cmd.interpreter.end_timeline_stretch(st.kept);
        match (self.timeline.as_mut(), st.from) {
            (Some(KernelTimeline::Record { cells, events }), _) => {
                let mine = t.map(|t| t.recorded_events()).unwrap_or_default();
                let n = mine.len();
                // a cell recorded again replaces its stretch
                if let Some(i) = cells.iter().position(|c| c.cell == st.cell) {
                    let old = cells.remove(i);
                    let gone = old.to - old.from;
                    events.drain(old.from..old.to);
                    for c in cells.iter_mut().filter(|c| c.from >= old.to) {
                        c.from -= gone;
                        c.to -= gone;
                    }
                }
                let from = events.len();
                events.extend(mine);
                cells.push(crate::timeline::TraceCell {
                    cell: st.cell,
                    code: st.code,
                    line: st.line,
                    from,
                    to: from + n,
                });
                ("record", json!({ "events": n }))
            }
            (Some(KernelTimeline::Replay { trace, .. }), Some((at, main))) => {
                let used = t.as_ref().map(|t| t.len()).unwrap_or(0);
                let rec = trace.cells.get(at);
                let mut note = json!({ "events": used, "recorded": main });
                match rec {
                    None => {
                        note["missing"] = J::Bool(true);
                    }
                    Some(rc) => {
                        if rc.code != st.code {
                            note["code_changed"] = J::Bool(true);
                        }
                    }
                }
                let diverged = match outcome {
                    Err(e) => {
                        let m = plain_message(e);
                        m.contains("timeline: replay diverged").then_some(m)
                    }
                    Ok(_) => None,
                }
                .or_else(|| {
                    (crate::timeline::any_thread_diverged()).then(|| "a task the cell spawned diverged from the trace".to_string())
                })
                .or_else(|| {
                    (rec.is_some() && used < main).then(|| {
                        format!(
                            "the cell made {} of the {} recorded calls: it reads less than it did when recorded",
                            used, main
                        )
                    })
                });
                if let Some(m) = diverged {
                    note["diverged"] = J::String(m);
                }
                ("replay", note)
            }
            _ => ("replay", json!({ "missing": true })),
        }
    }

    /// The notebook's path, when this session is a kernel.
    fn notebook_path(&self) -> Option<PathBuf> {
        self.notebook.as_ref().map(|n| n.path.clone())
    }

    fn hello(&self, id: Option<&J>) -> J {
        let mut v = json!({
            "event": "hello", "protocol": PROTOCOL, "olang": crate::version::VERSION,
            "root": self.root.to_string_lossy(), "pid": std::process::id(),
            "ops": ["hello", "eval", "command", "commands", "complete", "deps", "drop", "expand", "fields", "release", "interrupt", "page", "ping", "record", "reload", "replay", "reread", "render", "reset", "set", "shutdown", "stats", "vars"],
            "features": ["bind", "pure", "cell", "caps", "profile", "distinct", "record", "html", "scene", "dtypes", "daterange"],
        });
        if let Some(nb) = &self.notebook {
            v["notebook"] = json!({
                "path": nb.path.to_string_lossy(),
                "title": nb.title,
                "params": nb.params.iter().map(|p| p.0.clone()).collect::<Vec<_>>(),
                "problems": nb.problems.iter().map(|d| json!({ "line": d.line, "column": d.column, "message": d.message })).collect::<Vec<_>>(),
                "refused": nb.refused,
            });
        }
        if let Some(id) = id {
            v["id"] = id.clone();
            v["ok"] = J::Bool(true);
            v.as_object_mut().map(|o| o.shift_remove("event"));
        }
        v
    }

    fn handle(&mut self, op: &str, msg: &J, running: &AtomicBool) -> J {
        let id = msg.get("id").cloned().unwrap_or(J::Null);
        let mut reply = match op {
            "hello" => return self.hello(Some(&id)),
            "eval" => {
                // a `:` command (or `!shell`, or `quit`) typed where code goes
                let code = msg
                    .get("code")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .trim();
                if msg.get("file").is_none() && msg.get("cell").is_none() && is_command_line(code) {
                    let code = code.to_string();
                    self.command(&code, &id, running)
                } else {
                    self.eval(msg, running)
                }
            }
            "command" => {
                let line = msg
                    .get("line")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                self.command(&line, &id, running)
            }
            "commands" => self.commands(msg),
            "complete" => self.complete(msg),
            "deps" => deps_reply(msg),
            "drop" => self.drop_cell(msg),
            "vars" => self.vars(),
            "fields" => self.fields(msg),
            "reread" => self.reread(msg),
            "set" => self.set_control(msg),
            "record" => self.record(msg),
            // the notebook's page for an outputs document (a published app's)
            "page" => match self.notebook_path() {
                Some(p) => {
                    let text = msg
                        .get("text")
                        .and_then(|t| t.as_str())
                        .map(str::to_string)
                        .or_else(|| std::fs::read_to_string(&p).ok())
                        .unwrap_or_default();
                    let mode = if msg.get("mode").and_then(|m| m.as_str()) == Some("app") {
                        crate::notebook_export::Mode::App
                    } else {
                        crate::notebook_export::Mode::Export
                    };
                    let doc = msg.get("doc").cloned().unwrap_or(J::Null);
                    json!({ "ok": true, "html": crate::notebook_export::page_html(&text, &p, &doc, mode) })
                }
                None => {
                    json!({ "ok": false, "error": { "kind": "protocol", "message": "a page is a notebook kernel's (--notebook)" } })
                }
            },
            "replay" => self.replay(msg),
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
                self.cells.clear();
                self.declared.clear();
                self.controls.clear();
                crate::controls::forget_all();
                self.handles.clear();
                self.evals.clear();
                self.sorts.clear();
                self.views.clear();
                json!({ "ok": true })
            }
            "shutdown" => json!({ "ok": true }),
            // where each function ran in the evaluations since the last
            // `reset: true` (or since the start): src/tier_stats.rs
            "stats" => {
                let reset = msg.get("reset").and_then(|r| r.as_bool()).unwrap_or(false);
                json!({ "ok": true, "stats": crate::tier_stats::report(reset) })
            }
            other => {
                json!({ "ok": false, "error": { "kind": "protocol", "message": format!("unknown op \"{}\"", other) } })
            }
        };
        reply["id"] = id;
        reply
    }

    // ── scopes ──────────────────────────────────────────────────────

    fn take_scope(&mut self, file: Option<&Path>) -> (String, Scope, Vec<String>) {
        // a kernel's own scope is its notebook's
        let nb = self.notebook_path();
        let file = file.or(nb.as_deref());
        let key = file
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(s) = self.scopes.remove(&key) {
            return (key, s, Vec::new());
        }
        if file.is_some() && file == nb.as_deref() {
            let (scope, notes) = self.notebook_scope();
            return (key, scope, notes);
        }
        let mut scope = Scope {
            scope: self.cmd.interpreter.new_repl_scope(file),
            uses: Vec::new(),
        };
        let mut notes = Vec::new();
        if let Some(f) = file {
            notes = self.load_declarations(&mut scope, f);
        }
        (key, scope, notes)
    }

    /// The notebook's scope, made: its params bound, each a `let` on its
    /// front matter line (an error there names the line). Notes say what
    /// could not be bound.
    fn notebook_scope(&mut self) -> (Scope, Vec<String>) {
        let nb = self.notebook.as_ref().expect("a kernel's notebook");
        let path = nb.path.clone();
        let params = nb.params.clone();
        let mut scope = Scope {
            scope: self.cmd.interpreter.new_repl_scope(Some(&path)),
            uses: Vec::new(),
        };
        let mut notes = Vec::new();
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        for (name, value, line) in params {
            let code = format!("{}let {} = {}", "\n".repeat(line as usize - 1), name, value);
            let outcome = match self.parser.parse(&code) {
                Ok(p) => self.cmd.interpreter.eval_program(p),
                Err(e) => Err(InterpreterError::RuntimeError {
                    message: parse_message(&e),
                }),
            };
            if let Err(e) = outcome {
                let _ = self.cmd.interpreter.take_error_location();
                self.unbound_params
                    .insert(name.to_string(), plain_message(&e));
                notes.push(format!(
                    "{}:{}: the param `{}` is not bound: {}",
                    short(&self.root, &path),
                    line,
                    name,
                    plain_message(&e)
                ));
            } else {
                self.unbound_params.remove(name.as_str());
            }
        }
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        (scope, notes)
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
                if matches!(
                    inner,
                    Statement::UseDecl(_) | Statement::ShareDecl(ShareDecl::Use(_))
                ) {
                    scope.uses.push(stmt_text(&lines, &starts, i));
                }
                keep.push(st.clone());
            }
        }
        let mut notes = Vec::new();
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        for st in keep {
            let line = stmt_line(&st);
            let one = crate::ast::Program {
                statements: vec![st],
            };
            if let Err(e) = self.cmd.interpreter.eval_program(one) {
                notes.push(format!(
                    "{}:{}: {}",
                    short(&self.root, file),
                    line,
                    plain_message(&e)
                ));
                let _ = self.cmd.interpreter.take_error_location();
            }
        }
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        notes
    }

    fn index_file(&mut self, file: &Path, src: &str, statements: &[Statement]) {
        let starts: Vec<u32> = statements.iter().map(stmt_line).collect();
        let total = src.lines().count() as u32;
        let mut fns = Vec::new();
        let mut skipped = Vec::new();
        for (i, st) in statements.iter().enumerate() {
            let end = starts
                .get(i + 1)
                .map(|n| n.saturating_sub(1))
                .unwrap_or(total);
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
        self.files.insert(
            file.to_path_buf(),
            FileIndex {
                stamp: file_stamp(file),
                fns,
                skipped,
            },
        );
    }

    /// The index of `file` (read again when it changed on disk).
    fn file_index(&mut self, file: &Path) -> Option<&FileIndex> {
        let fresh = self
            .files
            .get(file)
            .map(|f| f.stamp == file_stamp(file))
            .unwrap_or(false);
        if !fresh {
            // a notebook's program: its cells on the file's own lines
            let src = crate::notebook::read(file, crate::notebook::Mode::Check)
                .ok()?
                .source;
            let program = self.parser.parse(&src).ok()?;
            self.index_file(file, &src, &program.statements);
        }
        self.files.get(file)
    }

    // ── eval ────────────────────────────────────────────────────────

    fn eval(&mut self, msg: &J, running: &AtomicBool) -> J {
        let code = msg
            .get("code")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        // values for a function's parameters, for this evaluation only
        let binds: Vec<(String, String)> = msg
            .get("bind")
            .and_then(|b| b.as_object())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let pure = msg.get("pure").and_then(|p| p.as_bool()).unwrap_or(false);
        // `profile: true`: this evaluation sampled, its profile in the reply
        // (none while another profile samples the process)
        let profiling =
            msg.get("profile").and_then(|p| p.as_bool()) == Some(true) && !crate::profile::busy();
        // a cell's evaluation: what it defines is the cell's (its id a
        // string or a number)
        let cell = msg.get("cell").map(|c| match c.as_str() {
            Some(s) => s.to_string(),
            None => c.to_string(),
        });
        let asked_file = msg.get("file").is_some();
        // a kernel's evaluations are its notebook's (lines are its lines)
        let file = msg
            .get("file")
            .and_then(|f| f.as_str())
            .map(|f| self.resolve(f))
            .or_else(|| self.notebook_path());
        // a snippet is parsed where it stands, after as many line breaks:
        // a line past its file's end (or past any file) padded gigabytes
        let line = asked_line(msg).min(
            file.as_deref()
                .and_then(|f| std::fs::read_to_string(f).ok())
                // its buffer may be longer than the disk's text: room for that
                .map(|t| t.lines().count() + 10_000)
                .unwrap_or(LINE_MAX),
        );
        let t0 = Instant::now();
        let (key, mut scope, notes) = self.take_scope(file.as_deref());
        // the snippet parsed where it stands in its file
        let padded = if file.is_some() && line > 1 {
            format!("{}{}", "\n".repeat(line - 1), code)
        } else {
            code.clone()
        };
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
        let uses: Vec<String> = if program.statements.iter().any(|s| {
            matches!(
                s.unwrapped(),
                Statement::UseDecl(_) | Statement::ShareDecl(ShareDecl::Use(_))
            )
        }) {
            vec![padded.clone()]
        } else {
            Vec::new()
        };
        let stats0 = self.cmd.interpreter.bytecode_tier_stats();
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        // what a cell defines, in the order it does (said with its reply)
        let mut defines: Vec<String> = Vec::new();
        // what it defines that no `use` binds: a control cell's one name
        let mut own: Vec<String> = Vec::new();
        // a cell evaluated again: what it defined before is dropped first,
        // and what it defines now is the cell's (kept whatever happens: a
        // failure halfway may have defined some of it)
        if let Some(c) = &cell {
            let slot = (key.clone(), c.clone());
            for name in self.cells.remove(&slot).unwrap_or_default() {
                self.cmd.interpreter.forget_top_level(&name);
            }
            let mut names = Vec::new();
            for st in &program.statements {
                for name in crate::notebook::defined_names(st, false) {
                    if !own.contains(&name) {
                        own.push(name);
                    }
                }
                for name in crate::notebook::defined_names(st, true) {
                    if !defines.contains(&name) {
                        defines.push(name.clone());
                    }
                    names.push(name);
                }
                if let (Statement::FunctionDecl(f), Some(fp)) = (st.unwrapped(), file.as_ref()) {
                    self.declared
                        .insert(f.name.clone(), (fp.clone(), stmt_line(st)));
                }
            }
            names.sort();
            names.dedup();
            self.cells.insert(slot, names);
        }
        // the prompt's inputs are the session's history (`:history`, `:!n`),
        // and its watched variables are watched
        let prompt = !asked_file && cell.is_none();
        let watching = prompt && self.cmd.watch_before();
        if prompt {
            self.cmd.command_history.push(code.clone());
        }
        crate::interrupt::clear();
        let _ = crate::output::take_collected();
        running.store(true, Ordering::SeqCst);
        // under no capability: a pure evaluation, or a refusal
        let kept_caps =
            if pure {
                Some(self.cmd.interpreter.replace_capabilities(Some(Arc::new(
                    crate::caps::CapTable {
                        app: crate::caps::Caps::none(),
                        deps: Vec::new(),
                        fixed: false,
                    },
                ))))
            } else {
                None
            };
        // the parameters given, each evaluated where the snippet stands
        let mut restore: Vec<(String, Option<Value>)> = Vec::new();
        let mut bind_failed: Option<(String, InterpreterError)> = None;
        for (name, src) in &binds {
            let value = match self.parser.parse(src) {
                Ok(p) => self.cmd.interpreter.eval_program(p),
                Err(e) => Err(InterpreterError::RuntimeError {
                    message: parse_message(&e),
                }),
            };
            match value {
                Ok(v) => {
                    restore.push((
                        name.clone(),
                        self.cmd.interpreter.get_environment().get(name),
                    ));
                    self.cmd.interpreter.define_variable(name.clone(), v);
                }
                Err(e) => {
                    let _ = self.cmd.interpreter.take_error_location();
                    bind_failed = Some((name.clone(), e));
                    break;
                }
            }
        }
        let _ = self.cmd.interpreter.drain_caps_trace();
        // a cell's controls are recorded as it binds them (src/controls.rs)
        if let Some(c) = &cell {
            crate::controls::begin_cell(c);
        }
        // a recorded run's cell records its inputs, a replayed one is
        // served them (never a prompt's or a pure `${expr}`'s evaluation)
        let stretch = match &cell {
            Some(c) if !pure => self.stretch_begin(c, &code, line as u32),
            _ => None,
        };
        let started = Instant::now();
        let session = profiling.then(|| {
            crate::profile::detail_enable();
            crate::profile::start(PROFILE_EVAL_US)
        });
        crate::profile::stats_sample_begin(crate::tier_stats::INTERVAL_US);
        let outcome = match bind_failed.take() {
            Some((name, e)) => Err(InterpreterError::RuntimeError {
                message: format!(
                    "the value given for `{}` fails: {}",
                    name,
                    plain_message(&e)
                ),
            }),
            None => self.cmd.interpreter.eval_program(program),
        };
        crate::profile::stats_sample_end();
        let timeline_note = stretch.map(|st| self.stretch_end(st, &outcome));
        let profile_doc = session.map(|s| {
            let mut lines = crate::profile::LineCache::default();
            self.cell_frames(s.finish_json(&mut lines))
        });
        let bound_controls = if cell.is_some() {
            crate::controls::end_cell()
        } else {
            Vec::new()
        };
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        running.store(false, Ordering::SeqCst);
        // what it reached: what the gate let through, and what it refused
        let mut reached: Vec<String> = self
            .cmd
            .interpreter
            .drain_caps_trace()
            .into_iter()
            .map(|u| cap_word(u).to_string())
            .collect();
        if let Err(e) = &outcome
            && let Some(cap) = denied_capability(&plain_message(e))
        {
            reached.push(cap);
        }
        reached.sort();
        reached.dedup();
        let interrupted = crate::interrupt::pending();
        crate::interrupt::clear();
        let location = if outcome.is_err() {
            self.cmd.interpreter.take_error_location()
        } else {
            None
        };
        // a `let`'s value is its binding's at the prompt and in a file; a
        // cell's value is the language's: `()` unless an expression ends it
        let bound = match (&outcome, &last) {
            _ if cell.is_some() => None,
            (Ok(_), Some(Statement::LetDecl(l)))
            | (Ok(_), Some(Statement::ShareDecl(ShareDecl::Let(l)))) => match &l.pattern {
                crate::ast::Pattern::Identifier(n) => Some(n.clone()),
                _ => None,
            },
            _ => None,
        };
        let bound_value = bound
            .as_ref()
            .and_then(|n| self.cmd.interpreter.get_environment().get(n));
        // the parameters taken back, the session's grant put back
        for (name, was) in restore.into_iter().rev() {
            match was {
                Some(v) => self.cmd.interpreter.define_variable(name, v),
                None => self.cmd.interpreter.remove_variable(&name),
            }
        }
        if let Some(caps) = kept_caps {
            self.cmd.interpreter.replace_capabilities(caps);
        }
        let watched = if watching && outcome.is_ok() {
            self.cmd.watch_after()
        } else {
            Vec::new()
        };
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        if outcome.is_ok() {
            scope.uses.extend(uses);
        }
        let stats1 = self.cmd.interpreter.bytecode_tier_stats();
        let printed = crate::output::take_collected();
        let (bc, nat) = match (stats0, stats1) {
            (Some(a), Some(b)) => (
                b.bytecode_calls.saturating_sub(a.bytecode_calls),
                b.jit_native_calls.saturating_sub(a.jit_native_calls),
            ),
            _ => (0, 0),
        };
        let tier = if nat > 0 {
            "native"
        } else if bc > 0 {
            "bytecode"
        } else {
            "interpreter"
        };
        let mut reply = match outcome {
            Ok(value) => {
                self.eval_seq += 1;
                let seq = self.eval_seq;
                let mut held = Vec::new();
                let shown = match (&last, bound_value) {
                    (Some(Statement::Expression(_)), _) if cell.is_some() => {
                        self.encode(&value, 0, &mut held)
                    }
                    _ if cell.is_some() => json!({ "k": "unit", "t": "Unit", "s": "()" }),
                    (_, Some(v)) => self.encode(&v, 0, &mut held),
                    (Some(Statement::FunctionDecl(f)), _)
                    | (Some(Statement::ShareDecl(ShareDecl::Function(f))), _) => json!({
                        "k": "decl", "t": "Function",
                        "s": format!("fn {}({})", f.name, f.parameters.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")) }),
                    (Some(Statement::Expression(_)), _) => self.encode(&value, 0, &mut held),
                    (Some(Statement::UseDecl(_)), _) => {
                        json!({ "k": "decl", "t": "Use", "s": "use" })
                    }
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
                if !defines.is_empty() {
                    r["defines"] = json!(defines);
                }
                // the control the cell bound: what an editor draws and `set`s
                if let Some(c) = &cell {
                    match bound_controls.first() {
                        None => {
                            self.controls.remove(c);
                        }
                        Some(b) => {
                            let name = (bound_controls.len() == 1 && own.len() == 1)
                                .then(|| own[0].clone());
                            let mut held = Vec::new();
                            r["control"] = self.control_json(b, name.as_deref(), &mut held);
                            if let Some(entry) = self.evals.back_mut() {
                                entry.1.extend(held);
                            }
                            self.controls.insert(
                                c.clone(),
                                CellControl {
                                    kind: b.kind.clone(),
                                    default: b.default.clone(),
                                    spec: b.spec.clone(),
                                    name,
                                },
                            );
                        }
                    }
                }
                r
            }
            Err(e) => {
                let mut err = self.error_json(&e, location, file.as_deref(), line, interrupted);
                // under no capability, the one it wanted makes it impure
                if pure {
                    let m = err["message"].as_str().unwrap_or("").to_string();
                    if let Some(cap) = denied_capability(&m) {
                        err["kind"] = J::String("impure".to_string());
                        err["cap"] = J::String(cap);
                    }
                }
                json!({ "ok": false, "error": err })
            }
        };
        self.scopes.insert(key, scope);
        reply["ms"] = json!(round3(ms));
        reply["server_ms"] = json!(round3(t0.elapsed().as_secs_f64() * 1000.0));
        reply["tier"] = J::String(tier.to_string());
        reply["calls"] = json!({ "bytecode": bc, "native": nat });
        reply["out"] = J::String(printed);
        reply["caps"] = json!(reached);
        if let Some(doc) = profile_doc {
            reply["profile"] = doc;
        }
        if let Some((k, note)) = timeline_note {
            reply[k] = note;
        }
        // `html`: the output as a page draws it (a published app swaps it in)
        if let Some(h) = msg
            .get("html")
            .filter(|h| !h.is_null() && h.as_bool() != Some(false))
        {
            let md = h.get("markdown").and_then(|m| m.as_bool()) == Some(true);
            let id = cell.clone().unwrap_or_default();
            // a picture's path read only inside the notebook's folder
            let folder = self
                .notebook
                .as_ref()
                .and_then(|n| n.path.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| self.root.clone());
            reply["html"] = J::String(crate::notebook_export::with_picture_root(
                Some(&folder),
                || {
                    crate::notebook_export::output_html(
                        &reply,
                        None,
                        crate::notebook_export::Mode::App,
                        &id,
                        md,
                    )
                },
            ));
            if let Some(v) = reply.get("value") {
                reply["text"] = J::String(crate::notebook_export::value_text(v));
            }
        }
        if !notes.is_empty() {
            reply["notes"] = json!(notes);
        }
        if !watched.is_empty() {
            reply["watched"] = json!(watched);
        }
        reply
    }

    // ── commands ────────────────────────────────────────────────────

    /// A `:` command run as the terminal REPL runs it (`Repl::run_command`),
    /// in the prompt's scope. What it says is streamed as it is said
    /// (`{"event": "out", "id", "text", "err"}`) and sent whole with the
    /// reply, as plain text (`out`) and styled lines (`lines`); what it
    /// found besides — a value, a type, a time, a tutorial, a place the
    /// editor has — comes as fields of its own.
    fn command(&mut self, line: &str, id: &J, running: &AtomicBool) -> J {
        let t0 = Instant::now();
        let word = line.split_whitespace().next().unwrap_or("").to_string();
        let spec = crate::repl_commands::find(&word);
        let (key, mut scope, _) = self.take_scope(None);
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        crate::interrupt::clear();
        let _ = crate::output::take_collected();
        let live: Option<crate::stdio::Live> = self.out.clone().map(|out| {
            let id = id.clone();
            Box::new(move |err: bool, text: &str| {
                out.send(&json!({ "event": "out", "id": id, "text": crate::repl_commands::strip_ansi(text), "err": err }));
            }) as crate::stdio::Live
        });
        colored::control::set_override(true);
        crate::stdio::capture_begin(live);
        running.store(true, Ordering::SeqCst);
        let started = Instant::now();
        let outcome = if spec.is_none() && line != "quit" {
            // an unknown command: the nearest of all of them
            match crate::repl_commands::suggest(&word) {
                Some(c) => println!("unknown command {} — did you mean {}?", word, c),
                None => println!(
                    "unknown command {} — :help lists them. Statements need no colon: `use collections`, `let x = 1`",
                    word
                ),
            }
            Ok(())
        } else if matches!(word.as_str(), ":end" | ":cancel") {
            println!("No multi-line input is open — {} starts one.", ":ml");
            Ok(())
        } else {
            self.cmd.run_command(line)
        };
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        running.store(false, Ordering::SeqCst);
        let interrupted = crate::interrupt::pending();
        crate::interrupt::clear();
        let said = crate::stdio::capture_end();
        colored::control::unset_override();
        // what a program printed outside the capture (another thread's)
        let stray = crate::output::take_collected();
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        // `:run` leaves the module context on its file
        if word == ":run" {
            self.cmd
                .interpreter
                .set_current_file(&self.root.join("olang.toml"));
        }
        let mut text = String::new();
        for (_, t) in &said {
            text.push_str(t);
        }
        text.push_str(&stray);
        let lines: Vec<J> = crate::repl_commands::styled_lines(&text)
            .into_iter()
            .map(|l| {
                J::Array(
                    l.into_iter()
                        .map(|r| {
                            let mut o = json!({ "s": r.text });
                            if let Some(c) = r.color {
                                o["c"] = J::String(c.to_string());
                            }
                            if r.bold {
                                o["b"] = J::Bool(true);
                            }
                            if r.dim {
                                o["d"] = J::Bool(true);
                            }
                            o
                        })
                        .collect(),
                )
            })
            .collect();
        let mut reply = json!({
            "ok": true,
            "command": spec.map(|c| c.name).unwrap_or(if line == "quit" { ":quit" } else { "" }),
            "out": crate::repl_commands::strip_ansi(&text),
            "lines": lines,
            "ms": round3(ms),
        });
        if spec.is_none() && line != "quit" {
            reply["unknown"] = J::Bool(true);
            if let Some(c) = crate::repl_commands::suggest(&word) {
                reply["suggest"] = J::String(c.to_string());
            }
        }
        match outcome {
            Err(e) => {
                reply["ok"] = J::Bool(false);
                reply["error"] = json!({ "kind": if interrupted { "interrupted" } else { "command" }, "message": crate::repl_commands::strip_ansi(&e.to_string()), "stack": [] });
            }
            Ok(()) if interrupted => {
                reply["interrupted"] = J::Bool(true);
            }
            Ok(()) => {}
        }
        let notes = std::mem::take(&mut self.cmd.notes);
        let mut effects: Vec<J> = Vec::new();
        let mut held = Vec::new();
        let mut values: Vec<J> = Vec::new();
        for n in notes {
            use crate::repl_commands::Note;
            match n {
                Note::Effect(e) => effects.push(J::String(e.to_string())),
                Note::Value { label, value } => {
                    let v = self.encode(&value, 0, &mut held);
                    values.push(json!({ "label": label, "value": v }));
                }
                Note::Type(t) => reply["type"] = J::String(t),
                Note::Time(t) => reply["time_ms"] = json!(round3(t)),
                Note::Bench(runs) => {
                    reply["bench"] = json!(runs.iter().map(|r| round3(*r)).collect::<Vec<_>>())
                }
                Note::Open(place) => reply["open"] = J::String(place.to_string()),
                Note::Commands => reply["commands"] = self.command_list(""),
                Note::Cwd(d) => reply["cwd"] = J::String(d),
                Note::Tutorial(t) => {
                    reply["tutorial"] = json!({
                        "name": t.name, "description": t.description, "difficulty": t.difficulty, "time": t.estimated_time,
                        "steps": t.steps.iter().map(|s| json!({ "title": s.title, "description": s.description, "code": s.code,
                            "expected": s.expected_output, "explanation": s.explanation, "hints": s.hints })).collect::<Vec<_>>(),
                    })
                }
            }
        }
        if !held.is_empty() || !values.is_empty() {
            self.eval_seq += 1;
            let seq = self.eval_seq;
            self.evals.push_back((seq, held));
            while self.evals.len() > KEEP_EVALS {
                let (old, _) = self.evals[0];
                self.drop_eval(old);
            }
            reply["eval"] = json!(seq);
        }
        if let Some(v) = values.last() {
            reply["value"] = v["value"].clone();
        }
        if !values.is_empty() {
            reply["values"] = J::Array(values);
        }
        if !effects.is_empty() {
            reply["effects"] = J::Array(effects);
        }
        reply["server_ms"] = json!(round3(t0.elapsed().as_secs_f64() * 1000.0));
        reply
    }

    /// The commands (those starting with `prefix`): name, aliases, usage,
    /// summary, the kind of argument, group, and whether an editor
    /// answers it with its own interface.
    fn command_list(&self, prefix: &str) -> J {
        J::Array(
            crate::repl_commands::COMMANDS
                .iter()
                .filter(|c| c.name.starts_with(prefix))
                .map(|c| {
                    let mut o = json!({ "name": c.name, "aliases": c.aliases, "usage": c.usage, "summary": c.summary,
                                         "arg": c.arg.name(), "group": c.group, "client": c.client });
                    if let crate::repl_commands::Arg::Words(ws) = c.arg {
                        o["words"] = json!(ws);
                    }
                    o
                })
                .collect(),
        )
    }

    fn commands(&self, msg: &J) -> J {
        let prefix = msg.get("prefix").and_then(|p| p.as_str()).unwrap_or("");
        json!({ "ok": true, "commands": self.command_list(prefix) })
    }

    /// Completion of a prompt's line at `pos` (characters): a command, its
    /// arguments (paths against the working folder, the session's names,
    /// topics, tutorials, options), or code (the session's bindings and
    /// the library's names, paths inside a string).
    fn complete(&mut self, msg: &J) -> J {
        let line = msg.get("line").and_then(|l| l.as_str()).unwrap_or("");
        let chars = msg
            .get("pos")
            .and_then(|p| p.as_u64())
            .map(|p| p as usize)
            .unwrap_or(line.chars().count());
        let pos = line
            .char_indices()
            .nth(chars)
            .map(|(i, _)| i)
            .unwrap_or(line.len());
        let (key, mut scope, _) = self.take_scope(None);
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        let user = self.cmd.interpreter.get_user_variables();
        let mut vars: Vec<(String, String)> = user
            .iter()
            .filter(|(_, v)| !matches!(v, Value::Struct { type_name, .. } if type_name.as_str() == "Module"))
            .map(|(n, v)| (n.clone(), crate::repl::Repl::type_name_of(v).to_string()))
            .collect();
        let fns: Vec<String> = user
            .iter()
            .filter(|(_, v)| matches!(v, Value::Function(_)))
            .map(|(n, _)| n.clone())
            .collect();
        drop(user);
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        vars.sort();
        if self.library.is_none() {
            self.library = Some(self.cmd.library_names());
        }
        let names = crate::repl_commands::Names {
            vars,
            fns,
            library: self.library.clone().unwrap_or_default(),
            topics: self.cmd.help().get_categories(),
            tutorials: self
                .cmd
                .help()
                .get_tutorials()
                .iter()
                .map(|t| t.name.clone())
                .collect(),
            cwd: std::env::current_dir().unwrap_or_else(|_| self.root.clone()),
        };
        let (start, items) = crate::repl_commands::complete(line, pos, &names);
        let start_chars = line[..start].chars().count();
        // the command being typed, for help as it is typed
        let word = line.split_whitespace().next().unwrap_or("");
        let about = if line.trim_start().starts_with(':') {
            crate::repl_commands::find(word).map(|c| json!({ "name": c.name, "usage": c.usage, "summary": c.summary, "client": c.client }))
        } else {
            None
        };
        json!({ "ok": true, "start": start_chars, "items": items.iter().map(|i| json!({ "label": i.label, "kind": i.kind, "detail": i.detail })).collect::<Vec<_>>(),
                "command": about })
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
        let mut kind = if interrupted || crate::interrupt::is_interrupt(&message) {
            "interrupted"
        } else {
            "runtime"
        };
        if kind == "interrupted" {
            message = "interrupted".to_string();
        }
        let (line, col, efile, stack, hint) = match &loc {
            Some(l) => {
                let f = l
                    .file
                    .as_ref()
                    .map(|f| self.resolve(f))
                    .or_else(|| file.map(|f| f.to_path_buf()));
                (l.line, l.column, f, l.call_stack.clone(), l.hint.clone())
            }
            None => (
                at_line as u32,
                1,
                file.map(|f| f.to_path_buf()),
                Vec::new(),
                None,
            ),
        };
        // a parameter of the function the snippet stands in, not bound
        // here: the REPL evaluates code as written (SPEC §18.1 is open)
        let mut within: Option<(String, u32, Vec<String>)> = None;
        let in_notebook = file.is_some_and(|f| self.notebook.as_ref().is_some_and(|n| n.path == f));
        let param_why = match e {
            InterpreterError::UndefinedVariable { name } => {
                self.unbound_params.get(name.as_str()).cloned()
            }
            _ => None,
        };
        if let (InterpreterError::UndefinedVariable { name }, Some(f)) = (e, file)
            && let Some(ix) = self.file_index(f)
        {
            if let Some((fname, a, _, ps)) = ix.fns.iter().find(|(_, a, b, ps)| {
                (*a as usize) < at_line && at_line <= *b as usize && ps.contains(name)
            }) {
                message = format!("`{}` is a parameter of `{}` — not bound here", name, fname);
                kind = "unbound";
                within = Some((fname.clone(), *a, ps.clone()));
            } else if let Some(why) = &param_why {
                // a param whose value did not bind: said, with why
                message = format!("the param `{}` is not bound: {}", name, why);
                kind = "unbound";
            } else if let Some((_, l)) = ix.skipped.iter().find(|(n, _)| n == name) {
                let l = *l;
                // a kernel's: the cell that defines it is to be run
                message = if in_notebook {
                    format!(
                        "`{}` is defined at line {}, in a cell not run yet — run that cell first",
                        name, l
                    )
                } else {
                    format!(
                        "`{}` is a top-level binding that runs code — evaluate its line ({}) first",
                        name, l
                    )
                };
                kind = "unbound";
            }
        }
        // A runaway recursion's stack is a hundred thousand frames: its
        // first and last ones are what tell, and each frame is looked up
        // below (the reply was megabytes, and slow to make).
        let total = stack.len();
        let kept: Vec<&String> = if total > STACK_HEAD + STACK_TAIL {
            stack
                .iter()
                .take(STACK_HEAD)
                .chain(stack.iter().skip(total - STACK_TAIL))
                .collect()
        } else {
            stack.iter().collect()
        };
        let mut frames: Vec<J> = kept
            .into_iter()
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
        if total > STACK_HEAD + STACK_TAIL {
            let left_out = total - STACK_HEAD - STACK_TAIL;
            frames.insert(
                STACK_HEAD,
                json!({ "name": format!("… {} frames more", left_out), "file": J::Null, "line": J::Null }),
            );
        }
        let mut out = json!({ "kind": kind, "message": message, "file": efile.map(|f| f.to_string_lossy().into_owned()),
                "line": line, "col": col, "stack": frames, "hint": hint });
        // the function the snippet stands in: what `bind` would give
        if let Some((fname, fline, params)) = within {
            out["fn"] = J::String(fname);
            out["fn_line"] = json!(fline);
            out["params"] = json!(params);
        }
        out
    }

    /// Where function `name` is declared: in `near`'s file, else the
    /// function value's own file.
    fn locate_fn(&mut self, name: &str, near: Option<&Path>) -> Option<(PathBuf, u32)> {
        let bare = name.rsplit('.').next().unwrap_or(name).to_string();
        // a cell's function, where the cell last declared it
        if let Some(at) = self.declared.get(&bare) {
            return Some(at.clone());
        }
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
        let l = self
            .file_index(&path)?
            .fns
            .iter()
            .find(|(n, ..)| *n == bare)?
            .1;
        Some((path, l))
    }

    fn resolve(&self, f: &str) -> PathBuf {
        let p = Path::new(f);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        }
    }

    // ── reload ──────────────────────────────────────────────────────

    fn reload(&mut self, msg: &J, running: &AtomicBool) -> J {
        let t0 = Instant::now();
        let Some(path) = msg
            .get("path")
            .and_then(|p| p.as_str())
            .map(|p| self.resolve(p))
        else {
            return json!({ "ok": false, "error": { "kind": "protocol", "message": "reload needs a path" } });
        };
        let src = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                return json!({ "ok": false, "error": { "kind": "io", "message": e.to_string(), "file": path.to_string_lossy() } });
            }
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
            self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
            for u in &uses {
                if let Ok(p) = self.parser.parse(u) {
                    // only the `use` statements of what was evaluated
                    let only: Vec<Statement> = p
                        .statements
                        .into_iter()
                        .filter(|s| {
                            matches!(
                                s.unwrapped(),
                                Statement::UseDecl(_) | Statement::ShareDecl(ShareDecl::Use(_))
                            )
                        })
                        .collect();
                    if let Err(e) = self
                        .cmd
                        .interpreter
                        .eval_program(crate::ast::Program { statements: only })
                    {
                        problems.push(plain_message(&e));
                        let _ = self.cmd.interpreter.take_error_location();
                    }
                }
            }
            self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
            self.scopes.insert(k, scope);
        }
        running.store(false, Ordering::SeqCst);
        self.files.remove(&path);
        // compiled code calls its callees directly (the native tier links
        // a group of functions together): start the tiers afresh, so
        // what runs next is compiled from what is loaded now
        if self.tiers {
            self.cmd.interpreter.enable_bytecode_tier(1, false);
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
                self.views.retain(|k, _| k.0 != h);
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
                // a chart's description: what an editor draws natively
                let chart = chart_of(s);
                if n > TEXT_CHARS || chart.is_some() {
                    r["h"] = json!(self.hold(v, held));
                }
                if let Some(p) = self.image_path(s) {
                    r["img"] = J::String(p);
                }
                if let Some(svg) = svg_of(s) {
                    r["svg"] = svg;
                }
                if let Some(mut c) = chart {
                    decimate_chart(&mut c, None);
                    r["chart"] = c;
                }
                r
            }
            Value::Ok(inner) | Value::Err(inner) => {
                let ok = matches!(v, Value::Ok(_));
                json!({ "k": "result", "t": t, "ok": ok, "v": self.encode(inner, depth, held) })
            }
            Value::List(items) | Value::Tuple(items) => {
                let kind = if matches!(v, Value::List(_)) {
                    "list"
                } else {
                    "tuple"
                };
                let h = self.hold(v, held);
                let mut r = json!({ "k": kind, "t": t, "n": items.len(), "h": h, "s": preview(v) });
                if depth < 2 {
                    let first: Vec<J> = items
                        .iter()
                        .take(FIRST_ITEMS)
                        .map(|x| self.encode(x, depth + 1, held))
                        .collect();
                    r["items"] = J::Array(first);
                }
                if let Some(cols) = table_columns(items) {
                    r["table"] = json!({ "cols": cols });
                } else if let Some(series) = series_of(items) {
                    r["series"] = series;
                }
                r
            }
            Value::Range {
                start,
                end,
                inclusive,
            } => {
                let n = (if *inclusive {
                    end - start + 1
                } else {
                    end - start
                })
                .max(0);
                json!({ "k": "range", "t": t, "n": n, "s": v.to_string() })
            }
            Value::Map(_) if crate::ods::gg::is_spec(v) => self.encode_gg(v, None, held),
            Value::Map(m) => {
                if let Some(role) = loom_view_role(m) {
                    let h = self.hold(v, held);
                    return json!({ "k": "view", "t": "View", "role": role, "h": h, "s": format!("a Loom {} view", role) });
                }
                let h = self.hold(v, held);
                let mut r = json!({ "k": "map", "t": t, "n": m.len(), "h": h, "s": preview(v) });
                if depth < 2 {
                    let keys: Vec<&String> = m.keys().collect();
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
                let mut r =
                    json!({ "k": "record", "t": t, "n": fields.len(), "h": h, "s": preview(v) });
                if depth < 2 {
                    let keys: Vec<&String> = fields.keys().collect();
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
            Value::EnumConstructor(_) | Value::TypeInfo(_) => {
                json!({ "k": "fn", "t": t, "s": preview(v) })
            }
            // A Matrix shows as the Frame of its columns (`0`, `1`, …),
            // which every table view already reads.
            Value::Native(_) if let Some(f) = crate::ods::matrix_frame(v) => {
                let mut r = self.encode_ods(&f, held);
                if let Some((rows, cols)) = crate::ods::matrix::matrix_of(v).map(|m| m.shape()) {
                    r["s"] = J::String(format!("Matrix[{} x {}]", rows, cols));
                }
                r
            }
            Value::Native(_) if ods_columns(v).is_some() => self.encode_ods(v, held),
            Value::Native(_) => match crate::stdlib::bytes::bytes_of(v) {
                Ok(b) => {
                    let h = self.hold(v, held);
                    let mut r = json!({ "k": "bytes", "t": "Bytes", "n": b.len(), "h": h, "s": format!("{} bytes", b.len()) });
                    if let Some((fmt, w, ht)) = image_kind(b) {
                        r["img"] = json!({ "format": fmt, "w": w, "h": ht });
                        if b.len() <= 16 * 1024 * 1024 {
                            r["png"] =
                                J::String(base64::engine::general_purpose::STANDARD.encode(b));
                        }
                    }
                    r
                }
                Err(_) => json!({ "k": "native", "t": t, "s": preview(v) }),
            },
        }
    }

    /// A Frame or a Series as a table (docs/tooling.md, "Values"): its
    /// columns' names and types, its rows' count, and its first rows'
    /// cells as text (a null as `null`); `expand` with `table` reads the
    /// rest by range, sorted.
    /// A gg plot (the map `use gg` builds) as a chart: its SVG (`svg`, for
    /// a picture and an export) and its scene (`chart`, `type: "scene"`,
    /// what an editor draws natively, thinned for the screen), held by
    /// handle so a zoom renders it again (`expand` with `chart`).
    fn encode_gg(&mut self, v: &Value, zoom: Option<(f64, f64, Option<usize>)>, held: &mut Vec<u64>) -> J {
        let h = self.hold(v, held);
        match crate::ods::gg::scene_of(v, zoom) {
            Ok(scene) => {
                let svg = scene.to_svg();
                let chart: J = serde_json::from_str(&scene.to_json(&olang_ods::gg::SceneJsonOptions::default()))
                    .unwrap_or(J::Null);
                let mut r = json!({ "k": "str", "t": "Plot", "n": svg.chars().count(), "h": h,
                                    "s": svg.chars().take(TEXT_CHARS).collect::<String>(), "chart": chart });
                if let Some(sv) = svg_of(&svg) {
                    r["svg"] = sv;
                }
                r
            }
            Err(e) => json!({ "k": "str", "t": "Plot", "n": e.chars().count(), "s": e, "h": h }),
        }
    }

    fn encode_ods(&mut self, v: &Value, held: &mut Vec<u64>) -> J {
        let cols = ods_columns(v).unwrap_or_default();
        let frame = crate::ods::frame_of(v).is_some();
        let n = cols.first().map(|(_, c)| c.len()).unwrap_or(0);
        let names: Vec<&str> = cols.iter().map(|(name, _)| name.as_str()).collect();
        let types: Vec<String> = cols.iter().map(|(_, c)| c.dtype().to_string()).collect();
        let rows: Vec<J> = (0..n.min(FIRST_ITEMS)).map(|i| ods_row(&cols, i)).collect();
        let s = if frame {
            format!("Frame[{} x {}]", n, cols.len())
        } else {
            format!(
                "Series[{}] of {}",
                n,
                types.first().map(String::as_str).unwrap_or("Float")
            )
        };
        let h = self.hold(v, held);
        let series: Vec<&olang_ods::Series> = cols.iter().map(|(_, c)| *c).collect();
        let mut table = json!({ "cols": names, "types": types, "sum": summaries(&series) });
        if n > SUMMARY_ROWS {
            table["sampled"] = json!(SUMMARY_ROWS);
        }
        let mut r = json!({ "k": if frame { "frame" } else { "series" }, "t": v.type_name(), "n": n, "h": h, "s": s,
                "table": table, "rows": rows });
        if !frame
            && let Some((_, col)) = cols.first()
            && let Some(st) = numeric_summary(col)
        {
            r["stats"] = st;
        }
        r
    }

    /// A string that names a picture on disk: its absolute path.
    fn image_path(&self, s: &str) -> Option<String> {
        if s.len() > 4096 || s.contains('\n') {
            return None;
        }
        let lower = s.to_ascii_lowercase();
        let pic = [".png", ".jpg", ".jpeg", ".gif", ".webp"]
            .iter()
            .any(|e| lower.ends_with(e));
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
        let count =
            (msg.get("count").and_then(|s| s.as_u64()).unwrap_or(100) as usize).min(EXPAND_MAX);
        let mut held = Vec::new();
        let reply = match &v {
            Value::Map(_) if crate::ods::gg::is_spec(&v) && msg.get("chart").is_some() => {
                let bound = |k: &str| msg["chart"].get(k).and_then(|x| x.as_f64());
                let zoom = match (bound("x0"), bound("x1")) {
                    (Some(a), Some(b)) => Some((a, b, msg["chart"].get("panel").and_then(|p| p.as_u64()).map(|p| p as usize))),
                    _ => None,
                };
                match crate::ods::gg::scene_of(&v, zoom) {
                    Ok(scene) => {
                        let c: J = serde_json::from_str(&scene.to_json(&olang_ods::gg::SceneJsonOptions::default())).unwrap_or(J::Null);
                        json!({ "ok": true, "chart": c })
                    }
                    Err(e) => json!({ "ok": false, "error": { "kind": "error", "message": e } }),
                }
            }
            Value::Map(_) if crate::ods::gg::is_spec(&v) => match crate::ods::gg::scene_of(&v, None) {
                Ok(scene) => {
                    let text: String = scene.to_svg().chars().skip(start).take(count.max(1) * 100).collect();
                    json!({ "ok": true, "s": text })
                }
                Err(e) => json!({ "ok": false, "error": { "kind": "error", "message": e } }),
            },
            Value::String(s) if msg.get("chart").is_some() => match chart_of(s) {
                Some(mut c) => {
                    let bound = |k: &str| msg["chart"].get(k).and_then(|x| x.as_f64());
                    let lo = bound("x0").unwrap_or(f64::NEG_INFINITY);
                    let hi = bound("x1").unwrap_or(f64::INFINITY);
                    decimate_chart(&mut c, Some((lo.min(hi), hi.max(lo))));
                    json!({ "ok": true, "chart": c })
                }
                None => {
                    json!({ "ok": false, "error": { "kind": "protocol", "message": "that value is no chart (a plot chart's SVG)" } })
                }
            },
            Value::String(s) => {
                let text: String = s.chars().skip(start).take(count.max(1) * 100).collect();
                json!({ "ok": true, "s": text })
            }
            Value::List(items) | Value::Tuple(items) => {
                if msg.get("table").and_then(|t| t.as_bool()) == Some(true) {
                    let cols = table_columns(items).unwrap_or_default();
                    let order = match msg
                        .get("sort")
                        .and_then(|s| s.get("col"))
                        .and_then(|c| c.as_u64())
                    {
                        Some(c) => {
                            let desc = msg["sort"]
                                .get("desc")
                                .and_then(|d| d.as_bool())
                                .unwrap_or(false);
                            Some(self.sorted(h, items, &cols, c as usize, desc))
                        }
                        None => None,
                    };
                    let n = items.len();
                    let rows: Vec<J> = (start..(start + count).min(n))
                        .map(|i| {
                            let at = order.as_ref().map(|o| o[i]).unwrap_or(i);
                            let cells: Vec<J> = cols
                                .iter()
                                .map(|c| J::String(cell_text(&items[at], c)))
                                .collect();
                            json!({ "i": at, "c": cells })
                        })
                        .collect();
                    json!({ "ok": true, "n": n, "total": n, "cols": cols, "rows": rows })
                } else {
                    let n = items.len();
                    let out: Vec<J> = (start..(start + count).min(n))
                        .map(|i| json!({ "i": i, "v": self.encode(&items[i], 1, &mut held) }))
                        .collect();
                    json!({ "ok": true, "n": n, "items": out })
                }
            }
            Value::Map(m) => {
                let keys: Vec<&String> = m.keys().collect();
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
                let keys: Vec<&String> = fields.keys().collect();
                let n = keys.len();
                let out: Vec<J> = keys
                    .into_iter()
                    .skip(start)
                    .take(count)
                    .map(|k| json!({ "key": k, "v": self.encode(&fields[k], 1, &mut held) }))
                    .collect();
                json!({ "ok": true, "n": n, "entries": out })
            }
            Value::Native(_) if ods_columns(&v).is_some() && msg.get("distinct").is_some() => {
                let cols = ods_columns(&v).unwrap_or_default();
                distinct_values(&cols, &msg["distinct"], start, count)
            }
            Value::Native(_) if ods_columns(&v).is_some() => {
                let cols = ods_columns(&v).unwrap_or_default();
                let total = cols.first().map(|(_, c)| c.len()).unwrap_or(0);
                let sort = msg
                    .get("sort")
                    .and_then(|s| s.get("col"))
                    .and_then(|c| c.as_u64())
                    .map(|c| {
                        let desc = msg["sort"]
                            .get("desc")
                            .and_then(|d| d.as_bool())
                            .unwrap_or(false);
                        (c as usize, desc)
                    });
                let filters = match msg.get("filters") {
                    None | Some(J::Null) => Vec::new(),
                    Some(J::Array(fs)) => match read_filters(fs, &cols) {
                        Ok(f) => f,
                        Err(m) => {
                            return json!({ "ok": false, "error": { "kind": "protocol", "message": m } });
                        }
                    },
                    Some(_) => {
                        return json!({ "ok": false, "error": { "kind": "protocol", "message": "filters is a list of {col, range | values | contains | nulls}" } });
                    }
                };
                // the view's rows: the frame's own, or its filtered (and
                // sorted) order, cached by handle
                let order: Option<Arc<Vec<usize>>> = if filters.is_empty() {
                    sort.map(|(c, desc)| self.sorted_ods(h, &cols, c, desc))
                } else {
                    let key = (
                        h,
                        json!([msg["filters"], msg.get("sort").cloned().unwrap_or(J::Null)])
                            .to_string(),
                    );
                    match self.views.get(&key) {
                        Some(o) => Some(o.clone()),
                        None => {
                            let keep = filter_mask(&filters, &cols, total);
                            let o: Vec<usize> = match sort {
                                Some((c, desc)) => self
                                    .sorted_ods(h, &cols, c, desc)
                                    .iter()
                                    .copied()
                                    .filter(|&i| keep[i])
                                    .collect(),
                                None => (0..total).filter(|&i| keep[i]).collect(),
                            };
                            let o = Arc::new(o);
                            self.views.insert(key, o.clone());
                            Some(o)
                        }
                    }
                };
                let n = order.as_ref().map(|o| o.len()).unwrap_or(total);
                let rows: Vec<J> = (start..(start + count).min(n))
                    .map(|i| ods_row(&cols, order.as_ref().map(|o| o[i]).unwrap_or(i)))
                    .collect();
                let names: Vec<&str> = cols.iter().map(|(name, _)| name.as_str()).collect();
                let mut r =
                    json!({ "ok": true, "n": n, "total": total, "cols": names, "rows": rows });
                if msg.get("summary").and_then(|s| s.as_bool()) == Some(true) {
                    let sum = if filters.is_empty() {
                        let series: Vec<&olang_ods::Series> =
                            cols.iter().map(|(_, c)| *c).collect();
                        summaries(&series)
                    } else {
                        // the view's rows (its first SUMMARY_ROWS), gathered
                        let o = order.as_ref().expect("a filtered view has an order");
                        let mut idx: Vec<i64> =
                            o.iter().take(SUMMARY_ROWS).map(|&i| i as i64).collect();
                        // a sort does not change a summary but its sample's rows
                        if o.len() > SUMMARY_ROWS {
                            idx.sort_unstable();
                        }
                        let take = olang_ods::Series::from_i64(idx);
                        let gathered: Vec<olang_ods::Series> = cols
                            .iter()
                            .map(|(_, c)| c.take(&take).unwrap_or_else(|_| (*c).clone()))
                            .collect();
                        summaries(&gathered.iter().collect::<Vec<_>>())
                    };
                    r["sum"] = sum;
                    if n > SUMMARY_ROWS {
                        r["sampled"] = json!(SUMMARY_ROWS);
                    }
                }
                r
            }
            Value::Native(_) => match crate::stdlib::bytes::bytes_of(&v) {
                Ok(b) => {
                    json!({ "ok": true, "n": b.len(), "b64": base64::engine::general_purpose::STANDARD.encode(b) })
                }
                Err(_) => {
                    json!({ "ok": false, "error": { "kind": "protocol", "message": "nothing to expand" } })
                }
            },
            _ => {
                json!({ "ok": false, "error": { "kind": "protocol", "message": "nothing to expand" } })
            }
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

    /// A Frame's (or a Series') rows ordered by column `c`, its nulls last
    /// either way (cached by handle).
    fn sorted_ods(
        &mut self,
        h: u64,
        cols: &[(String, &olang_ods::Series)],
        c: usize,
        desc: bool,
    ) -> Arc<Vec<usize>> {
        use olang_ods::Scalar;
        use std::cmp::Ordering;
        if let Some(o) = self.sorts.get(&(h, c, desc)) {
            return o.clone();
        }
        let n = cols.first().map(|(_, s)| s.len()).unwrap_or(0);
        let mut order: Vec<usize> = (0..n).collect();
        if let Some((_, col)) = cols.get(c) {
            let keys: Vec<Scalar> = (0..n).map(|i| col.scalar_at(i)).collect();
            order.sort_by(|a, b| {
                let (x, y) = (&keys[*a], &keys[*b]);
                let o = match (x, y) {
                    (Scalar::Null, Scalar::Null) => return Ordering::Equal,
                    (Scalar::Null, _) => return Ordering::Greater,
                    (_, Scalar::Null) => return Ordering::Less,
                    (Scalar::F64(p), Scalar::F64(q)) => p.total_cmp(q),
                    (Scalar::I64(p), Scalar::I64(q)) => p.cmp(q),
                    (Scalar::Bool(p), Scalar::Bool(q)) => p.cmp(q),
                    (Scalar::Str(p), Scalar::Str(q)) => p.cmp(q),
                    (Scalar::Date(p), Scalar::Date(q)) => p.cmp(q),
                    (Scalar::Datetime(p), Scalar::Datetime(q)) => p.cmp(q),
                    _ => Ordering::Equal,
                };
                if desc { o.reverse() } else { o }
            });
        }
        let o = Arc::new(order);
        self.sorts.insert((h, c, desc), o.clone());
        o
    }

    /// A table's rows ordered by column `c` (cached by handle).
    fn sorted(
        &mut self,
        h: u64,
        items: &[Value],
        cols: &[String],
        c: usize,
        desc: bool,
    ) -> Arc<Vec<usize>> {
        if let Some(o) = self.sorts.get(&(h, c, desc)) {
            return o.clone();
        }
        let col = cols.get(c).cloned().unwrap_or_default();
        let keys: Vec<Value> = items
            .iter()
            .map(|r| field_of(r, &col).unwrap_or(Value::Unit))
            .collect();
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
            let opts = crate::pkg::InstallOptions {
                offline: true,
                ..Default::default()
            };
            let (map, _missing) = crate::pkg::install_lenient(&lroot, &opts);
            let mut map: HashMap<_, _> = map.into_iter().collect();
            map.insert("loom".to_string(), lroot.clone());
            self.cmd.interpreter.add_to_dependency_map(map);
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
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.cmd.interpreter.define_global("__studio_view", view);
        self.cmd
            .interpreter
            .define_global("__studio_opts", json_to_value(&opts));
        crate::interrupt::clear();
        running.store(true, Ordering::SeqCst);
        let program = self
            .parser
            .parse("use loom.lib.test { render_view as __studio_render_view }\n__studio_render_view(__studio_view, __studio_opts)");
        let outcome = match program {
            Ok(p) => self.cmd.interpreter.eval_program(p),
            Err(e) => Err(InterpreterError::RuntimeError {
                message: parse_message(&e),
            }),
        };
        running.store(false, Ordering::SeqCst);
        let _ = self.cmd.interpreter.take_error_location();
        self.cmd.interpreter.swap_repl_scope(&mut scope.scope);
        self.scopes.insert(key, scope);
        let _ = crate::output::take_collected();
        let ms = round3(t0.elapsed().as_secs_f64() * 1000.0);
        match outcome {
            Ok(Value::Map(ref m)) => {
                let png = m
                    .get("png")
                    .and_then(|p| crate::stdlib::bytes::bytes_of(p).ok().map(|b| b.to_vec()));
                let num = |k: &str| match m.get(k) {
                    Some(Value::Integer(i)) => *i as f64,
                    Some(Value::Float(f)) => *f,
                    _ => 0.0,
                };
                match png {
                    Some(b) => {
                        json!({ "ok": true, "png": base64::engine::general_purpose::STANDARD.encode(&b),
                                       "width": num("width"), "height": num("height"), "scale": opts["scale"], "ms": ms })
                    }
                    None => {
                        json!({ "ok": false, "ms": ms, "error": { "kind": "render", "message": "render_view answered no picture" } })
                    }
                }
            }
            Ok(other) => {
                json!({ "ok": false, "ms": ms, "error": { "kind": "render", "message": format!("render_view answered {}", other.type_name()) } })
            }
            Err(e) => {
                let mut message = plain_message(&e);
                if message.contains("loom")
                    && (message.contains("not found") || message.contains("Module"))
                {
                    message = format!(
                        "{} — the project has no `loom` dependency; pass its path as `loom`",
                        message
                    );
                }
                json!({ "ok": false, "ms": ms, "error": { "kind": "render", "message": message } })
            }
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────

/// Whether prompt input is a REPL command: `:word`, `!shell`, `quit`.
/// A value as olang source that makes it again, as far as a literal does
/// (a string quoted, anything else as it displays).
fn literal_text(v: &Value) -> String {
    match v {
        Value::String(s) => serde_json::to_string(s.as_str()).unwrap_or_default(),
        other => other.to_string(),
    }
}

fn is_command_line(code: &str) -> bool {
    (code.starts_with(':')
        && code
            .chars()
            .nth(1)
            .is_some_and(|c| c.is_alphabetic() || c == '!'))
        || (code.starts_with('!') && code.len() > 1 && !code.starts_with("!="))
        || code == "quit"
}

/// The capability a refusal names (`capability 'fs' denied: …`), if the
/// message is one.
fn denied_capability(message: &str) -> Option<String> {
    let at = message.find("capability '")? + "capability '".len();
    let rest = &message[at..];
    let end = rest.find('\'')?;
    rest[end..]
        .starts_with("' denied")
        .then(|| rest[..end].to_string())
}

/// `deps`: each cell's defined and used names, read from its text without
/// running it (`cells`: `[{cell, code, line}]`, each line the cell's first
/// in its notebook). A cell that does not parse says its error instead.
fn deps_reply(msg: &J) -> J {
    let cells = msg
        .get("cells")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let out: Vec<J> = cells
        .iter()
        .map(|c| {
            let code = c.get("code").and_then(|x| x.as_str()).unwrap_or("");
            let line = asked_line(c) as u32;
            let mut r = match crate::notebook::cell_deps(code, line) {
                Ok(d) => json!({
                    "ok": true,
                    "defines": d.defines.iter().map(|(n, l, col, by_use)| {
                        let mut x = json!({ "name": n, "line": l, "col": col });
                        if *by_use {
                            x["use"] = J::Bool(true);
                        }
                        x
                    }).collect::<Vec<_>>(),
                    "uses": d.uses.iter().map(|(n, l, col)| json!({ "name": n, "line": l, "col": col })).collect::<Vec<_>>(),
                }),
                Err(e) => {
                    let (l, col) = parse_position(&e);
                    json!({ "ok": false, "error": { "kind": "parse", "message": parse_message(&e), "line": l, "col": col } })
                }
            };
            if let Some(id) = c.get("cell") {
                r["cell"] = id.clone();
            }
            r
        })
        .collect();
    json!({ "ok": true, "cells": out })
}

/// A value's size in memory, roughly (bytes): its scalars, its strings'
/// text, a container's items (a long one measured on its first thousand
/// and scaled), a Frame's columns, Bytes' length.
fn approx_bytes(v: &Value) -> u64 {
    fn of(v: &Value, depth: usize) -> u64 {
        if depth > 6 {
            return 16;
        }
        let scaled = |n: usize, f: &mut dyn FnMut(usize) -> u64| -> u64 {
            let k = n.min(1000);
            let sum: u64 = (0..k).map(&mut *f).sum();
            if k == 0 { 0 } else { sum * n as u64 / k as u64 }
        };
        match v {
            Value::Integer(_) | Value::Float(_) | Value::Boolean(_) | Value::Unit => 16,
            Value::String(s) => 24 + s.len() as u64,
            Value::List(items) | Value::Tuple(items) => {
                24 + scaled(items.len(), &mut |i| of(&items[i], depth + 1))
            }
            Value::Map(m) => {
                let pairs: Vec<(&String, &Value)> = m.iter().collect();
                48 + scaled(pairs.len(), &mut |i| {
                    24 + pairs[i].0.len() as u64 + of(pairs[i].1, depth + 1)
                })
            }
            Value::Struct { fields, .. } => {
                let pairs: Vec<(&String, &Value)> = fields.iter().collect();
                48 + scaled(pairs.len(), &mut |i| {
                    24 + pairs[i].0.len() as u64 + of(pairs[i].1, depth + 1)
                })
            }
            Value::Ok(inner) | Value::Err(inner) => 16 + of(inner, depth + 1),
            Value::Native(_) => {
                if let Some(cols) = ods_columns(v) {
                    return cols
                        .iter()
                        .map(|(name, c)| name.len() as u64 + series_bytes(c))
                        .sum();
                }
                match crate::stdlib::bytes::bytes_of(v) {
                    Ok(b) => 24 + b.len() as u64,
                    Err(_) => 64,
                }
            }
            _ => 64,
        }
    }
    of(v, 0)
}

/// An `ods` column's size: its values (a text column's spans and text,
/// measured on its first ten thousand when longer), its nulls' bits.
fn series_bytes(c: &olang_ods::Series) -> u64 {
    use olang_ods::Series;
    let n = c.len() as u64;
    let bits = n.div_ceil(8);
    match c {
        Series::F64 { .. } | Series::I64 { .. } | Series::Datetime { .. } => 8 * n + bits,
        Series::Date { .. } => 4 * n + bits,
        Series::Cat { codes, levels, .. } => {
            let text: u64 = levels.iter().map(|l| 8 + l.len() as u64).sum();
            4 * codes.len() as u64 + text + bits
        }
        Series::Bool { .. } => n + bits,
        Series::Str { col, .. } => {
            let k = col.len().min(10_000);
            let text: u64 = (0..k).map(|i| col.get(i).len() as u64).sum();
            let text = if k == 0 { 0 } else { text * n / k as u64 };
            8 * n + text + bits
        }
    }
}

/// A capability's name as the grant writes it.
fn cap_word(u: crate::caps::CapUse) -> &'static str {
    use crate::caps::CapUse;
    match u {
        CapUse::FsRead | CapUse::FsWrite => "fs",
        CapUse::Net => "net",
        CapUse::Db => "db",
        CapUse::Proc => "proc",
        CapUse::Env => "env",
    }
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn short(root: &Path, f: &Path) -> String {
    f.strip_prefix(root)
        .unwrap_or(f)
        .to_string_lossy()
        .into_owned()
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
    let b = starts
        .get(i + 1)
        .map(|n| (*n as usize).saturating_sub(1))
        .unwrap_or(lines.len())
        .clamp(a, lines.len());
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
            '(' if prev.is_alphanumeric() || prev == '_' || prev == ')' || prev == ']' => {
                return true;
            }
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
    "window",
    "group",
    "separator",
    "text",
    "heading",
    "button",
    "input",
    "textarea",
    "checkbox",
    "list",
    "table",
    "dialog",
    "popover",
    "menu",
    "select",
    "toast",
    "tabs",
    "segmented",
    "tab_bar",
    "split",
    "log",
    "tree",
    "canvas",
    "image",
    "region",
    "memo",
    "slider",
    "progress",
    "icon",
    "link",
    "toggle",
    "radio",
];

/// A Loom view node (`#{ role, key, props, children }`): its role.
fn loom_view_role(m: &crate::ast::ValueMap) -> Option<String> {
    if m.len() != 4
        || !m.contains_key("props")
        || !m.contains_key("children")
        || !m.contains_key("key")
    {
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
        let keys: Vec<&String> = fields.keys().collect();
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
        _ => a
            .compare_for_sort(b)
            .unwrap_or_else(|| a.to_string().cmp(&b.to_string())),
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

/// A Frame's columns (or a Series as one column, `value`): each name and
/// its series.
/// The most fields `fields` answers (a wider value says how many more).
const FIELDS_MAX: usize = 500;

/// A value's kind and its fields as `fields` answers them: a Frame's
/// columns and their types, a map's keys and a record's fields with their
/// values' types (in the order they were set), a list of maps' columns
/// (each typed by the first item holding it); anything else none.
fn value_fields(v: &Value) -> (&'static str, Vec<(String, String)>) {
    if let Some(f) = crate::ods::frame_of(v) {
        let cols = f
            .names()
            .iter()
            .cloned()
            .zip(f.columns().iter().map(|c| c.dtype().to_string()))
            .collect();
        return ("frame", cols);
    }
    if crate::ods::series_of(v).is_some() {
        return ("series", Vec::new());
    }
    match v {
        // a map of functions is a module (`ods`, a `use`d package), not data
        Value::Map(m)
            if !m.is_empty()
                && m.values()
                    .all(|x| matches!(x, Value::Builtin(_) | Value::Function(_))) =>
        {
            ("module", Vec::new())
        }
        Value::Struct { fields, .. }
            if !fields.is_empty()
                && fields
                    .values()
                    .all(|x| matches!(x, Value::Builtin(_) | Value::Function(_))) =>
        {
            ("module", Vec::new())
        }
        Value::Map(m) if loom_view_role(m).is_none() => (
            "map",
            m.iter()
                .map(|(k, x)| (k.clone(), x.type_name().to_string()))
                .collect(),
        ),
        Value::Struct { fields, .. } => (
            "record",
            fields
                .iter()
                .map(|(k, x)| (k.clone(), x.type_name().to_string()))
                .collect(),
        ),
        Value::List(items) => match table_columns(items) {
            Some(cols) => {
                let typed = cols
                    .into_iter()
                    .map(|c| {
                        let t = items
                            .iter()
                            .take(TABLE_SAMPLE)
                            .find_map(|it| match it {
                                Value::Map(m) => m.get(&c).map(|x| x.type_name().to_string()),
                                Value::Struct { fields, .. } => {
                                    fields.get(&c).map(|x| x.type_name().to_string())
                                }
                                _ => None,
                            })
                            .unwrap_or_default();
                        (c, t)
                    })
                    .collect();
                ("table", typed)
            }
            None => ("list", Vec::new()),
        },
        _ => ("none", Vec::new()),
    }
}

fn ods_columns(v: &Value) -> Option<Vec<(String, &olang_ods::Series)>> {
    if let Some(f) = crate::ods::frame_of(v) {
        return Some(f.names().iter().cloned().zip(f.columns().iter()).collect());
    }
    crate::ods::series_of(v).map(|s| vec![("value".to_string(), s)])
}

/// Bins of a numeric Series' histogram.
const HIST_BINS: usize = 24;

/// A numeric Series' summary: its values' count, nulls, mean, least and
/// most, and a histogram of HIST_BINS bins over that range.
fn numeric_summary(col: &olang_ods::Series) -> Option<J> {
    use olang_ods::Scalar;
    let n = col.len();
    let mut xs: Vec<f64> = Vec::with_capacity(n);
    for i in 0..n {
        match col.scalar_at(i) {
            Scalar::F64(x) if x.is_finite() => xs.push(x),
            Scalar::I64(x) => xs.push(x as f64),
            Scalar::F64(_) | Scalar::Null => {}
            _ => return None,
        }
    }
    if xs.is_empty() {
        return None;
    }
    let lo = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    let mut bins = vec![0u64; HIST_BINS];
    let span = hi - lo;
    for x in &xs {
        let b = if span > 0.0 {
            (((x - lo) / span) * HIST_BINS as f64) as usize
        } else {
            0
        };
        bins[b.min(HIST_BINS - 1)] += 1;
    }
    Some(
        json!({ "count": xs.len(), "nulls": n - xs.len(), "mean": mean, "min": lo, "max": hi, "hist": bins }),
    )
}

/// How often an evaluation asked for its profile is sampled (µs): a cell
/// runs for milliseconds, so finer than `olang profile`'s 1 kHz.
const PROFILE_EVAL_US: u64 = 100;

/// `expand`'s `distinct`: a column's distinct values with their counts,
/// the commonest first (then by their text), a page of them (`start`,
/// `count`); `contains` keeps the values holding the text
/// (case-sensitive, as a filter's). Read over the column's first
/// `SUMMARY_ROWS` (`sampled` past them). A null is a value (`null`).
fn distinct_values(cols: &[(String, &olang_ods::Series)], d: &J, start: usize, count: usize) -> J {
    let Some(c) = d.get("col").and_then(|c| c.as_u64()).map(|c| c as usize) else {
        return json!({ "ok": false, "error": { "kind": "protocol", "message": "distinct needs a col (a column's index)" } });
    };
    let Some((name, col)) = cols.get(c) else {
        return json!({ "ok": false, "error": { "kind": "protocol", "message": format!("distinct's col {} is past the table's {} columns", c, cols.len()) } });
    };
    let contains = d
        .get("contains")
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty());
    let rows = col.len().min(SUMMARY_ROWS);
    let mut counts: HashMap<Option<String>, u64> = HashMap::new();
    for i in 0..rows {
        let t = crate::ods::scalar_text(col.scalar_at(i));
        *counts.entry(t).or_insert(0) += 1;
    }
    let mut all: Vec<(Option<String>, u64)> = counts
        .into_iter()
        .filter(|(t, _)| match (contains, t) {
            (None, _) => true,
            (Some(q), Some(t)) => t.contains(q),
            (Some(_), None) => false,
        })
        .collect();
    all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let n = all.len();
    let values: Vec<J> = all
        .into_iter()
        .skip(start)
        .take(count)
        .map(|(t, k)| json!([t, k]))
        .collect();
    let mut r = json!({ "ok": true, "col": name, "n": n, "start": start, "values": values });
    if col.len() > SUMMARY_ROWS {
        r["sampled"] = json!(SUMMARY_ROWS);
    }
    r
}

/// Rows a column's summary reads at most (a longer column is summarised
/// over its first, and says it was sampled).
const SUMMARY_ROWS: usize = 1_000_000;
/// Bins of a summary's histogram.
const SUMMARY_BINS: usize = 20;
/// Values a summary's `top` names.
const SUMMARY_TOP: usize = 6;

/// Each column's summary (docs/tooling.md, "Values"): its nulls and
/// distinct values, a number's least, most, mean and histogram, any other
/// column's most frequent values as their cells' text. Long columns are
/// summarised side by side.
fn summaries(cols: &[&olang_ods::Series]) -> J {
    let one = |c: &olang_ods::Series| -> J {
        let s = c.summarize(SUMMARY_ROWS, SUMMARY_BINS, SUMMARY_TOP);
        let mut o = json!({ "nulls": s.nulls, "distinct": s.distinct });
        // A date column's histogram over its days (or microseconds), its
        // ends written as dates (the `dtypes` feature).
        let date_text = |x: f64| match c.dtype() {
            olang_ods::DType::Date => olang_ods::temporal::format_date(x as i64),
            _ => olang_ods::temporal::format_datetime(x as i64),
        };
        if matches!(c.dtype(), olang_ods::DType::Date | olang_ods::DType::Datetime) {
            if let Some((lo, hi, _)) = s.range {
                o["min"] = json!(date_text(lo));
                o["max"] = json!(date_text(hi));
            }
            o["hist"] = json!(s.hist);
        } else if s.numeric {
            if let Some((lo, hi, mean)) = s.range {
                o["min"] = json!(lo);
                o["max"] = json!(hi);
                o["mean"] = json!(mean);
            }
            o["hist"] = json!(s.hist);
        } else {
            o["top"] = J::Array(
                s.top
                    .iter()
                    .map(|(row, count)| {
                        json!([
                            crate::ods::scalar_text(c.scalar_at(*row)).unwrap_or_default(),
                            count
                        ])
                    })
                    .collect(),
            );
        }
        o
    };
    let long = cols.first().map(|c| c.len()).unwrap_or(0) >= 100_000;
    if long && cols.len() > 1 {
        let out: Vec<J> = std::thread::scope(|sc| {
            let hs: Vec<_> = cols.iter().map(|c| sc.spawn(move || one(c))).collect();
            hs.into_iter()
                .map(|h| h.join().unwrap_or(J::Null))
                .collect()
        });
        J::Array(out)
    } else {
        J::Array(cols.iter().map(|c| one(c)).collect())
    }
}

/// One of `expand`'s filters, read.
enum Filter {
    Range(usize, Option<f64>, Option<f64>),
    Values(usize, Vec<Option<String>>),
    Contains(usize, String),
    Nulls(usize, bool),
}

/// `expand`'s `filters` read against a table's columns: a protocol error
/// names what is wrong.
fn read_filters(fs: &[J], cols: &[(String, &olang_ods::Series)]) -> Result<Vec<Filter>, String> {
    use olang_ods::DType;
    let mut out = Vec::new();
    for f in fs {
        let Some(c) = f.get("col").and_then(|c| c.as_u64()).map(|c| c as usize) else {
            return Err("a filter needs a col (a column's index)".to_string());
        };
        let Some((name, col)) = cols.get(c) else {
            return Err(format!(
                "a filter's col {} is past the table's {} columns",
                c,
                cols.len()
            ));
        };
        if let Some(r) = f.get("range") {
            let pair = r.as_array().filter(|a| a.len() == 2).ok_or_else(|| {
                format!("the filter on `{}`: range is [lo, hi], either null", name)
            })?;
            // A date column's bounds are ISO text (the `daterange`
            // feature), read as its days or microseconds.
            let dated = matches!(col.dtype(), DType::Date | DType::Datetime);
            if !matches!(col.dtype(), DType::F64 | DType::I64) && !dated {
                return Err(format!(
                    "the filter on `{}`: a range is a number's or a date's, and `{}` is a {} column",
                    name,
                    name,
                    col.dtype()
                ));
            }
            let bound = |b: &J| -> Result<Option<f64>, String> {
                match b {
                    J::Null => Ok(None),
                    J::String(t) if dated => olang_ods::temporal::parse_iso(t.trim())
                        .map(|(us, _)| {
                            Some(if col.dtype() == DType::Date {
                                olang_ods::temporal::day_of_us(us) as f64
                            } else {
                                us as f64
                            })
                        })
                        .ok_or_else(|| {
                            format!("the filter on `{}`: \"{}\" is not an ISO date", name, t)
                        }),
                    b => b.as_f64().map(Some).ok_or_else(|| {
                        format!("the filter on `{}`: a bound is a number{}", name, if dated { " or ISO text" } else { "" })
                    }),
                }
            };
            out.push(Filter::Range(c, bound(&pair[0])?, bound(&pair[1])?));
        } else if let Some(vs) = f.get("values") {
            let vs = vs.as_array().ok_or_else(|| {
                format!(
                    "the filter on `{}`: values is a list of cells' text (null for a null)",
                    name
                )
            })?;
            out.push(Filter::Values(
                c,
                vs.iter().map(|v| v.as_str().map(str::to_string)).collect(),
            ));
        } else if let Some(t) = f.get("contains") {
            let t = t
                .as_str()
                .ok_or_else(|| format!("the filter on `{}`: contains is a String", name))?;
            out.push(Filter::Contains(c, t.to_string()));
        } else if let Some(b) = f.get("nulls") {
            let b = b.as_bool().ok_or_else(|| {
                format!(
                    "the filter on `{}`: nulls is true (only nulls) or false (no nulls)",
                    name
                )
            })?;
            out.push(Filter::Nulls(c, b));
        } else {
            return Err(format!(
                "the filter on `{}` says none of range, values, contains, nulls",
                name
            ));
        }
    }
    Ok(out)
}

/// The rows every filter keeps (they AND together).
fn filter_mask(filters: &[Filter], cols: &[(String, &olang_ods::Series)], n: usize) -> Vec<bool> {
    use olang_ods::Scalar;
    let mut keep = vec![true; n];
    for f in filters {
        match f {
            Filter::Range(c, lo, hi) => {
                let col = cols[*c].1;
                for (i, k) in keep.iter_mut().enumerate() {
                    if !*k {
                        continue;
                    }
                    let x = match col.scalar_at(i) {
                        Scalar::F64(x) => x,
                        Scalar::I64(x) => x as f64,
                        Scalar::Date(d) => d as f64,
                        Scalar::Datetime(t) => t as f64,
                        _ => {
                            *k = false;
                            continue;
                        }
                    };
                    *k = !x.is_nan() && lo.is_none_or(|lo| x >= lo) && hi.is_none_or(|hi| x <= hi);
                }
            }
            Filter::Values(c, vs) => {
                let col = cols[*c].1;
                let nulls = vs.iter().any(|v| v.is_none());
                let set: std::collections::HashSet<&str> =
                    vs.iter().flatten().map(String::as_str).collect();
                for (i, k) in keep.iter_mut().enumerate() {
                    if *k {
                        *k = match crate::ods::scalar_text(col.scalar_at(i)) {
                            Some(t) => set.contains(t.as_str()),
                            None => nulls,
                        };
                    }
                }
            }
            Filter::Contains(c, t) => {
                let col = cols[*c].1;
                for (i, k) in keep.iter_mut().enumerate() {
                    if *k {
                        *k = crate::ods::scalar_text(col.scalar_at(i))
                            .is_some_and(|text| text.contains(t.as_str()));
                    }
                }
            }
            Filter::Nulls(c, only) => {
                let col = cols[*c].1;
                for (i, k) in keep.iter_mut().enumerate() {
                    if *k {
                        *k = matches!(col.scalar_at(i), Scalar::Null) == *only;
                    }
                }
            }
        }
    }
    keep
}

/// Points a chart's xy series carries at most (more are decimated).
const CHART_POINTS: usize = 1500;

/// A string's chart description (what `plot` embeds), parsed.
fn chart_of(s: &str) -> Option<J> {
    if !s.trim_start().starts_with("<svg") {
        return None;
    }
    let text = olang_ods::plot::description_of(s)?;
    serde_json::from_str(&text).ok()
}

/// A chart's xy series cut to x in `range` (when given) and each made at
/// most CHART_POINTS points: a line's or an area's least and most y in
/// each of CHART_POINTS / 2 buckets of x, in x order; a scatter's points
/// at an even stride. A series decimated says its full count (`n`) and
/// `cut`.
fn decimate_chart(chart: &mut J, range: Option<(f64, f64)>) {
    if chart.get("type").and_then(|t| t.as_str()) != Some("xy") {
        return;
    }
    let Some(series) = chart.get_mut("series").and_then(|s| s.as_array_mut()) else {
        return;
    };
    for s in series.iter_mut() {
        let nums = |k: &str| -> Vec<f64> {
            s.get(k)
                .and_then(|a| a.as_array())
                .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect())
                .unwrap_or_default()
        };
        let xs = nums("x");
        let ys = nums("y");
        let colors: Option<Vec<J>> = s.get("colors").and_then(|c| c.as_array()).cloned();
        let mut idx: Vec<usize> = (0..xs.len().min(ys.len()))
            .filter(|&i| range.is_none_or(|(lo, hi)| xs[i] >= lo && xs[i] <= hi))
            .collect();
        let full = idx.len();
        let windowed = full != xs.len();
        if full > CHART_POINTS {
            let scatter = s.get("mark").and_then(|m| m.as_str()) == Some("scatter");
            idx = if scatter {
                let step = full.div_ceil(CHART_POINTS);
                idx.into_iter().step_by(step).collect()
            } else {
                let buckets = CHART_POINTS / 2;
                let lo = idx.iter().map(|&i| xs[i]).fold(f64::INFINITY, f64::min);
                let hi = idx.iter().map(|&i| xs[i]).fold(f64::NEG_INFINITY, f64::max);
                let span = if hi > lo { hi - lo } else { 1.0 };
                // each bucket's (least y, most y) points
                let mut picks: Vec<Option<(usize, usize)>> = vec![None; buckets];
                for &i in &idx {
                    let b = ((((xs[i] - lo) / span) * buckets as f64) as usize).min(buckets - 1);
                    let p = picks[b].get_or_insert((i, i));
                    if ys[i] < ys[p.0] {
                        p.0 = i;
                    }
                    if ys[i] > ys[p.1] {
                        p.1 = i;
                    }
                }
                let mut out = Vec::with_capacity(CHART_POINTS);
                for (a, b) in picks.into_iter().flatten() {
                    let (a, b) = if xs[a] <= xs[b] { (a, b) } else { (b, a) };
                    out.push(a);
                    if b != a {
                        out.push(b);
                    }
                }
                out
            };
        }
        if windowed || full > CHART_POINTS {
            s["x"] = json!(idx.iter().map(|&i| xs[i]).collect::<Vec<_>>());
            s["y"] = json!(idx.iter().map(|&i| ys[i]).collect::<Vec<_>>());
            if let Some(cs) = &colors {
                s["colors"] = J::Array(idx.iter().filter_map(|&i| cs.get(i).cloned()).collect());
            }
        }
        if full > CHART_POINTS {
            s["n"] = json!(full);
            s["cut"] = J::Bool(true);
        }
    }
}

/// Row `i` of a table of columns: `{"i", "c": [text or null…]}`.
fn ods_row(cols: &[(String, &olang_ods::Series)], i: usize) -> J {
    let cells: Vec<J> = cols
        .iter()
        .map(|(_, c)| match crate::ods::scalar_text(c.scalar_at(i)) {
            Some(t) => J::String(t),
            None => J::Null,
        })
        .collect();
    json!({ "i": i, "c": cells })
}

/// A string that is an SVG document (what `plot` answers): its size (the
/// `width` and `height` of its root, else its `viewBox`'s), its
/// background (the fill of a first `rect` as wide as it), and its text.
fn svg_of(s: &str) -> Option<J> {
    let t = s.trim_start();
    if !t.starts_with("<svg") || !s.trim_end().ends_with("</svg>") || s.len() > 8 * 1024 * 1024 {
        return None;
    }
    let open = &t[..t.find('>')?];
    let attr = |tag: &str, name: &str| -> Option<String> {
        let at = tag.find(&format!(" {}=\"", name))? + name.len() + 3;
        let end = tag[at..].find('"')?;
        Some(tag[at..at + end].to_string())
    };
    let num = |x: Option<String>| x.and_then(|v| v.trim_end_matches("px").parse::<f64>().ok());
    let vb: Vec<f64> = attr(open, "viewBox")
        .map(|v| {
            v.split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    let w = num(attr(open, "width")).or_else(|| vb.get(2).copied())?;
    let h = num(attr(open, "height")).or_else(|| vb.get(3).copied())?;
    let mut r = json!({ "w": w, "h": h, "text": s });
    let mut rest = &t[open.len()..];
    // a chart's description stands before its background
    if let Some(r) = rest.strip_prefix('>')
        && r.starts_with("<metadata")
        && let Some(e) = r.find("</metadata>")
    {
        rest = &r[e + "</metadata>".len() - 1..];
    }
    if let Some(at) = rest.find("<rect")
        && rest[..at].trim_start_matches('>').trim().is_empty()
    {
        let tag = &rest[at..at + rest[at..].find('>').unwrap_or(0)];
        if num(attr(tag, "width")) == Some(w)
            && let Some(fill) = attr(tag, "fill")
        {
            r["bg"] = J::String(fill);
        }
    }
    Some(r)
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
        assert_eq!(
            table_columns(&rows),
            Some(vec!["b".to_string(), "a".to_string()])
        );
    }
}

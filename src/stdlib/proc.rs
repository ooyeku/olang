//! `proc` — spawn and drive child processes with streaming I/O, chain
//! them into pipelines, and read exit codes.
//!
//! Where [`os.exec`](os.rs) runs a command to completion and hands back
//! its whole output at once, `proc` keeps the child *live*: feed its
//! stdin, read its stdout a line at a time, wait for its exit code, or
//! kill it. It is the primitive for long-running tools — REPLs you drive,
//! servers you supervise, filters you stream through.
//!
//!   let p = unwrap(proc.spawn("grep", ["olang"]))
//!   proc.write_line(p, "olang rules")
//!   proc.write_line(p, "nope")
//!   proc.close_stdin(p)
//!   println(unwrap(proc.read_line(p)))     // "olang rules"
//!   proc.wait(p)
//!
//! `proc.pipeline` runs several commands with each one's stdout wired to
//! the next one's stdin — the shell's `a | b | c` in a single call, with
//! every stage's exit code reported.
//!
//! Handles are `Process { id }` structs into a process-wide registry, the
//! same pattern as [`chan`](chan.rs) and [`db`](db.rs). stderr is drained
//! by a background thread into a buffer, and stdout by another into a line
//! channel, so a chatty child never deadlocks against a caller that is
//! only reading one stream.

use crate::ast::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock};

/// One live child. `child` is the process itself; `stdin` is its input
/// pipe (dropped to signal EOF); `out_rx` receives stdout one line at a
/// time from a drainer thread; `stderr` is filled by another drainer.
/// Draining both streams off-thread is what keeps a child that writes a
/// lot to one stream from blocking while the caller reads the other.
struct Proc {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
    out_rx: Mutex<Receiver<String>>,
    stderr: Arc<Mutex<String>>,
    pid: u32,
}

static NEXT_ID: AtomicI64 = AtomicI64::new(1);
static PROCS: OnceLock<Mutex<HashMap<i64, Arc<Proc>>>> = OnceLock::new();

fn procs() -> &'static Mutex<HashMap<i64, Arc<Proc>>> {
    PROCS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn create_proc_module() -> Value {
    let mut module = HashMap::new();
    // Arities are advisory: module-prefixed builtins dispatch straight to
    // their handler, which validates its own argument count (so `spawn`
    // and `pipeline` each accept an optional trailing options map).
    for (name, arity) in [
        ("spawn", 2),
        ("write", 2),
        ("write_line", 2),
        ("close_stdin", 1),
        ("read_line", 1),
        ("read_all", 1),
        ("stderr", 1),
        ("wait", 1),
        ("kill", 1),
        ("pid", 1),
        ("pipeline", 1),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("proc.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

pub fn call_proc_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "spawn" => proc_spawn(args),
        "write" => proc_write(args, false),
        "write_line" => proc_write(args, true),
        "close_stdin" => proc_close_stdin(args),
        "read_line" => proc_read_line(args),
        "read_all" => proc_read_all(args),
        "stderr" => proc_stderr(args),
        "wait" => proc_wait(args),
        "kill" => proc_kill(args),
        "pid" => proc_pid(args),
        "pipeline" => proc_pipeline(args),
        _ => Err(format!("Unknown proc function: {}", name).into()),
    }
}

fn handle(id: i64) -> Value {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), Value::Integer(id));
    Value::Struct {
        type_name: "Process".to_string(),
        fields: std::sync::Arc::new(fields),
    }
}

fn err(msg: impl Into<String>) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg.into()))))
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

fn unit_ok() -> Value {
    ok(Value::Unit)
}

/// Pull a live child back out of a `Process { id }` handle.
fn proc_of(value: &Value) -> Result<Arc<Proc>, Box<dyn std::error::Error>> {
    let id = match value {
        Value::Struct { type_name, fields } if type_name == "Process" => match fields.get("id") {
            Some(Value::Integer(id)) => *id,
            _ => return Err("proc: malformed process handle".into()),
        },
        _ => return Err("proc: expected a Process handle".into()),
    };
    procs()
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| "proc: process handle is no longer valid".into())
}

/// A program string.
fn as_program(v: &Value, who: &str) -> Result<String, Value> {
    match v {
        Value::String(s) => Ok(s.as_ref().clone()),
        _ => Err(err(format!("{}: program must be a string", who))),
    }
}

/// A list of string arguments.
fn as_args(v: &Value, who: &str) -> Result<Vec<String>, Value> {
    match v {
        Value::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items.iter() {
                match item {
                    Value::String(s) => out.push(s.as_ref().clone()),
                    other => {
                        return Err(err(format!(
                            "{}: arguments must be strings, found {}",
                            who,
                            other.type_name()
                        )));
                    }
                }
            }
            Ok(out)
        }
        _ => Err(err(format!("{}: arguments must be a list of strings", who))),
    }
}

/// Parse the shared options map (`cwd`, `env`, and for pipelines `stdin`).
/// Returns (cwd, env, stdin) — any subset may be present.
#[allow(clippy::type_complexity)]
fn parse_opts(
    value: &Value,
    who: &str,
) -> Result<(Option<String>, Vec<(String, String)>, Option<String>), Value> {
    let fields = match value {
        Value::Map(m) => m.as_ref().clone(),
        Value::Struct { fields, .. } => fields.as_ref().clone(),
        _ => return Err(err(format!("{}: options must be a map or object", who))),
    };
    let mut cwd = None;
    let mut env = Vec::new();
    let mut stdin = None;
    for (key, val) in &fields {
        match (key.as_str(), val) {
            ("cwd", Value::String(s)) => cwd = Some(s.as_ref().clone()),
            ("stdin", Value::String(s)) => stdin = Some(s.as_ref().clone()),
            ("env", Value::Map(m)) => {
                for (k, v) in m.iter() {
                    match v {
                        Value::String(s) => env.push((k.clone(), s.as_ref().clone())),
                        other => {
                            return Err(err(format!(
                                "{}: env values must be strings, got {}",
                                who,
                                other.type_name()
                            )));
                        }
                    }
                }
            }
            (other, _) => {
                return Err(err(format!(
                    "{}: unknown option '{}' (supported: cwd, env{})",
                    who,
                    other,
                    if who.contains("pipeline") {
                        ", stdin"
                    } else {
                        ""
                    }
                )));
            }
        }
    }
    Ok((cwd, env, stdin))
}

/// `proc.spawn(program, args)` / `proc.spawn(program, args, opts)` — start
/// a child with its stdin, stdout, and stderr piped, and return a live
/// `Process` handle. Both output streams begin draining immediately.
fn proc_spawn(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 && args.len() != 3 {
        return Ok(err(
            "spawn expects (program, args) or (program, args, opts)",
        ));
    }
    let program = match as_program(&args[0], "spawn") {
        Ok(p) => p,
        Err(e) => return Ok(e),
    };
    let cargs = match as_args(&args[1], "spawn") {
        Ok(a) => a,
        Err(e) => return Ok(e),
    };
    let (cwd, env) = match args.get(2) {
        Some(opts) => match parse_opts(opts, "spawn") {
            Ok((c, e, _)) => (c, e),
            Err(e) => return Ok(e),
        },
        None => (None, Vec::new()),
    };

    let mut command = Command::new(&program);
    command
        .args(&cargs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &cwd {
        command.current_dir(dir);
    }
    for (k, v) in &env {
        command.env(k, v);
    }

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return Ok(err(format!("spawn: failed to start '{}': {}", program, e))),
    };
    let pid = child.id();

    // Drain stdout into a line channel: read_line pops from the channel,
    // never from the pipe, so the child can outrun a slow reader.
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break; // receiver gone; stop draining
                        }
                    }
                    Err(_) => break,
                }
            }
            // tx dropped here → read_line sees EOF
        });
    }

    // Drain stderr into a shared buffer so it can never fill and block the
    // child while the caller is busy reading stdout.
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    if let Some(mut se) = child.stderr.take() {
        let sink = stderr_buf.clone();
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = se.read_to_string(&mut buf);
            *sink.lock().unwrap() = buf;
        });
    }

    let stdin = child.stdin.take();
    let proc = Arc::new(Proc {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        out_rx: Mutex::new(rx),
        stderr: stderr_buf,
        pid,
    });
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    procs().lock().unwrap().insert(id, proc);
    Ok(ok(handle(id)))
}

/// `proc.write(p, s)` / `proc.write_line(p, s)` — feed the child's stdin.
fn proc_write(args: Vec<Value>, newline: bool) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Ok(err("write expects (process, string)"));
    }
    let p = proc_of(&args[0])?;
    let s = match &args[1] {
        Value::String(s) => s.as_ref().clone(),
        other => {
            return Ok(err(format!(
                "write: value must be a string, got {}",
                other.type_name()
            )));
        }
    };
    let mut guard = p.stdin.lock().unwrap();
    match guard.as_mut() {
        Some(stdin) => {
            let res = stdin
                .write_all(s.as_bytes())
                .and_then(|_| {
                    if newline {
                        stdin.write_all(b"\n")
                    } else {
                        Ok(())
                    }
                })
                .and_then(|_| stdin.flush());
            match res {
                Ok(()) => Ok(unit_ok()),
                Err(e) => Ok(err(format!("write: {}", e))),
            }
        }
        None => Ok(err("write: stdin is already closed")),
    }
}

/// `proc.close_stdin(p)` — drop the child's stdin, signalling end-of-input
/// so a filter can finish. Idempotent.
fn proc_close_stdin(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(err("close_stdin expects (process)"));
    }
    let p = proc_of(&args[0])?;
    *p.stdin.lock().unwrap() = None;
    Ok(unit_ok())
}

/// `proc.read_line(p)` — the next line of the child's stdout (without its
/// newline), or `Err("eof")` once stdout closes. Blocks until a line is
/// available.
fn proc_read_line(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(err("read_line expects (process)"));
    }
    let p = proc_of(&args[0])?;
    let rx = p.out_rx.lock().unwrap();
    match rx.recv() {
        Ok(line) => Ok(ok(Value::String(Arc::new(line)))),
        Err(_) => Ok(err("eof")),
    }
}

/// `proc.read_all(p)` — the rest of the child's stdout as one string
/// (lines rejoined with `\n`). Blocks until stdout closes.
fn proc_read_all(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(err("read_all expects (process)"));
    }
    let p = proc_of(&args[0])?;
    let rx = p.out_rx.lock().unwrap();
    let mut lines = Vec::new();
    while let Ok(line) = rx.recv() {
        lines.push(line);
    }
    Ok(ok(Value::String(Arc::new(lines.join("\n")))))
}

/// `proc.stderr(p)` — everything the child has written to stderr. Complete
/// once the child exits; read it after `proc.wait`.
fn proc_stderr(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(err("stderr expects (process)"));
    }
    let p = proc_of(&args[0])?;
    let s = p.stderr.lock().unwrap().clone();
    Ok(ok(Value::String(Arc::new(s))))
}

/// `proc.wait(p)` — block until the child exits, returning `#{ code }`
/// (the exit code, or -1 if it was killed by a signal). Safe to call
/// without draining stdout first: the drainer threads keep the pipes
/// empty, so the child can always make progress to exit.
fn proc_wait(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(err("wait expects (process)"));
    }
    let p = proc_of(&args[0])?;
    // Close stdin first if still open, so a filter waiting on more input
    // can reach EOF and exit instead of hanging the wait forever.
    *p.stdin.lock().unwrap() = None;
    let mut child = p.child.lock().unwrap();
    match child.wait() {
        Ok(status) => {
            let code = status.code().unwrap_or(-1) as i64;
            let mut fields = HashMap::new();
            fields.insert("code".to_string(), Value::Integer(code));
            Ok(ok(Value::Struct {
                type_name: "Exit".to_string(),
                fields: std::sync::Arc::new(fields),
            }))
        }
        Err(e) => Ok(err(format!("wait: {}", e))),
    }
}

/// `proc.kill(p)` — terminate the child immediately (SIGKILL). Killing an
/// already-exited child is not an error.
fn proc_kill(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(err("kill expects (process)"));
    }
    let p = proc_of(&args[0])?;
    let _ = p.child.lock().unwrap().kill();
    Ok(unit_ok())
}

/// `proc.pid(p)` — the child's OS process id.
fn proc_pid(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Ok(err("pid expects (process)"));
    }
    let p = proc_of(&args[0])?;
    Ok(ok(Value::Integer(p.pid as i64)))
}

/// `proc.pipeline(stages)` / `proc.pipeline(stages, opts)` — run a chain of
/// commands with each one's stdout wired to the next one's stdin, the
/// shell's `a | b | c`. Each stage is a list `[program, arg, arg, …]`.
/// Returns `#{ code, stdout, stderr, codes }`: the last stage's stdout,
/// every stage's stderr concatenated, the last stage's exit code, and the
/// per-stage codes. `opts.stdin` feeds the first stage.
fn proc_pipeline(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.is_empty() || args.len() > 2 {
        return Ok(err("pipeline expects (stages) or (stages, opts)"));
    }
    let stage_list = match &args[0] {
        Value::List(items) => items,
        _ => {
            return Ok(err(
                "pipeline: stages must be a list of [program, ...args] lists",
            ));
        }
    };
    if stage_list.is_empty() {
        return Ok(err("pipeline: needs at least one stage"));
    }
    // Each stage: a non-empty list whose head is the program.
    let mut stages: Vec<(String, Vec<String>)> = Vec::with_capacity(stage_list.len());
    for stage in stage_list.iter() {
        let parts = match stage {
            Value::List(p) if !p.is_empty() => p,
            _ => {
                return Ok(err(
                    "pipeline: each stage must be a non-empty [program, ...args] list",
                ));
            }
        };
        let program = match as_program(&parts[0], "pipeline") {
            Ok(p) => p,
            Err(e) => return Ok(e),
        };
        let mut cargs = Vec::with_capacity(parts.len() - 1);
        for a in parts.iter().skip(1) {
            match a {
                Value::String(s) => cargs.push(s.as_ref().clone()),
                other => {
                    return Ok(err(format!(
                        "pipeline: stage arguments must be strings, found {}",
                        other.type_name()
                    )));
                }
            }
        }
        stages.push((program, cargs));
    }
    let (cwd, env, mut stdin_data) = match args.get(1) {
        Some(opts) => match parse_opts(opts, "pipeline") {
            Ok(parts) => parts,
            Err(e) => return Ok(e),
        },
        None => (None, Vec::new(), None),
    };

    // Spawn every stage, threading stdout → stdin down the chain.
    let n = stages.len();
    let mut children: Vec<Child> = Vec::with_capacity(n);
    let mut prev_out = None;
    for (i, (program, cargs)) in stages.iter().enumerate() {
        let mut command = Command::new(program);
        command
            .args(cargs)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &cwd {
            command.current_dir(dir);
        }
        for (k, v) in &env {
            command.env(k, v);
        }
        if i == 0 {
            command.stdin(if stdin_data.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        } else {
            command.stdin(Stdio::from(prev_out.take().unwrap()));
        }
        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                for mut c in children {
                    let _ = c.kill();
                }
                return Ok(err(format!(
                    "pipeline: failed to start stage {} '{}': {}",
                    i, program, e
                )));
            }
        };
        prev_out = child.stdout.take();
        children.push(child);
    }

    // Feed the first stage's stdin from a thread, so writing the input and
    // reading the final output happen concurrently (a large input would
    // otherwise deadlock against an unread final stdout).
    let feeder = stdin_data.take().map(|input| {
        let stdin = children[0].stdin.take();
        std::thread::spawn(move || {
            if let Some(mut s) = stdin {
                let _ = s.write_all(input.as_bytes());
            }
        })
    });

    // Drain every stage's stderr off-thread — same deadlock-avoidance.
    let mut stderr_threads = Vec::with_capacity(n);
    for child in &mut children {
        let se = child.stderr.take();
        stderr_threads.push(std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(mut s) = se {
                let _ = s.read_to_string(&mut buf);
            }
            buf
        }));
    }

    // Read the last stage's stdout on this thread.
    let mut final_out = String::new();
    if let Some(mut out) = prev_out.take() {
        let _ = out.read_to_string(&mut final_out);
    }

    // Collect exit codes in order.
    let mut codes = Vec::with_capacity(n);
    for child in &mut children {
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1) as i64;
        codes.push(code);
    }
    if let Some(f) = feeder {
        let _ = f.join();
    }
    let mut stderr_all = String::new();
    for t in stderr_threads {
        if let Ok(s) = t.join() {
            stderr_all.push_str(&s);
        }
    }

    let final_code = *codes.last().unwrap_or(&-1);
    let mut fields = HashMap::new();
    fields.insert("code".to_string(), Value::Integer(final_code));
    fields.insert("stdout".to_string(), Value::String(Arc::new(final_out)));
    fields.insert("stderr".to_string(), Value::String(Arc::new(stderr_all)));
    fields.insert(
        "codes".to_string(),
        Value::List(
            codes
                .into_iter()
                .map(Value::Integer)
                .collect::<Vec<_>>()
                .into(),
        ),
    );
    Ok(ok(Value::Struct {
        type_name: "Pipeline".to_string(),
        fields: std::sync::Arc::new(fields),
    }))
}

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
    /// stdout as it drains: a line each (its end removed) — or, spawned
    /// with `bytes: true`, the raw chunks as they came.
    out_rx: Mutex<Receiver<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// `bytes: true`: stdout and stderr answer Bytes, not text.
    bytes: bool,
    pid: u32,
    /// Spawned as the leader of its own process group (`#{ group: true }`),
    /// so `kill(p, #{ tree: true })` can end everything it started.
    group: bool,
}

/// Start `command` as the leader of a new process group (a new process
/// group on Windows too), so the whole tree it starts can be signalled
/// at once with [`kill_tree`].
pub(crate) fn set_own_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = command;
}

/// Kill `child` and every process in the group it leads (spawned through
/// [`set_own_group`]): SIGKILL to the group on Unix, `taskkill /T /F` on
/// Windows. The child itself is killed too, so this is never weaker than
/// `Child::kill`. A group already gone is not an error.
pub(crate) fn kill_tree(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        if pid > 0 && pid <= i32::MAX as u32 {
            // SAFETY: kill(2) with a negative pid signals the process
            // group; it touches no memory of ours.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// A child's environment as the options say: cleared first
/// (`clear_env`), then `env`'s variables set and its `()`s removed.
pub(crate) fn apply_env(
    command: &mut Command,
    clear: bool,
    set: &[(String, String)],
    removed: &[String],
) {
    if clear {
        command.env_clear();
    }
    for (k, v) in set {
        command.env(k, v);
    }
    for k in removed {
        command.env_remove(k);
    }
}

/// The `Command` for `program` as a child would find it, looked up on
/// PATH here, in the parent, through PATH's absolute folders only. An
/// empty entry (`a::b`, a trailing `:`) or a relative one (`.`, `bin`) is
/// resolved by `execvp` from the child's folder once it has moved there
/// — so `os.exec(["git", …], #{ cwd: repo, env: #{ PATH: … } })` ran the
/// repository's own `git`. A name with a slash is taken as it is. The
/// child still sees its plain name as `argv[0]`. A name found in no
/// absolute folder is not found (`No such file or directory`), never
/// left for the child to look up.
pub(crate) fn command_for(
    program: &str,
    clear: bool,
    set: &[(String, String)],
    removed: &[String],
) -> std::io::Result<Command> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        if program.contains('/') || program.is_empty() {
            return Ok(Command::new(program));
        }
        // the PATH the child gets: the options' own, else none when the
        // environment is cleared or PATH removed, else this process's
        let path: Option<String> = match set.iter().rev().find(|(k, _)| k == "PATH") {
            Some((_, v)) => Some(v.clone()),
            None if clear || removed.iter().any(|k| k == "PATH") => None,
            None => std::env::var("PATH").ok(),
        };
        // execvp's own default when there is no PATH
        let path = path.unwrap_or_else(|| "/usr/bin:/bin".to_string());
        match crate::caps::find_on_path(program, &path) {
            Some(cand) => {
                let mut c = Command::new(&cand);
                c.arg0(program);
                Ok(c)
            }
            None => Err(std::io::Error::from_raw_os_error(2)),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (clear, set, removed);
        Ok(Command::new(program))
    }
}

/// A `cwd` option that names no directory: said as such, before the
/// spawn would blame the program ("failed to start 'sh': No such file
/// or directory").
pub(crate) fn missing_dir(who: &str, cwd: Option<&str>) -> Option<String> {
    let dir = cwd?;
    if std::path::Path::new(dir).is_dir() {
        return None;
    }
    Some(if std::path::Path::new(dir).exists() {
        format!("{}: cwd '{}' is not a directory", who, dir)
    } else {
        format!("{}: cwd '{}' does not exist", who, dir)
    })
}

static NEXT_ID: AtomicI64 = AtomicI64::new(1);
static PROCS: OnceLock<Mutex<HashMap<i64, Arc<Proc>>>> = OnceLock::new();

fn procs() -> &'static Mutex<HashMap<i64, Arc<Proc>>> {
    PROCS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn create_proc_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    // Arities are advisory: module-prefixed builtins dispatch straight to
    // their handler, which validates its own argument count (so `spawn`
    // and `pipeline` each accept an optional trailing options map).
    for (name, arity) in [
        ("spawn", 1),
        ("write", 2),
        ("write_line", 2),
        ("write_frame", 2),
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
        "write_frame" => proc_write_frame(args),
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
    let mut fields = crate::ast::ValueMap::default();
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

/// The options `spawn` and `pipeline` take. `stdin` is the pipeline's
/// only; `group` is spawn's only.
#[derive(Default)]
struct Opts {
    cwd: Option<String>,
    env: Vec<(String, String)>,
    /// `env: #{ name: () }`: the variable removed from the child's
    env_removed: Vec<String>,
    /// `clear_env: true`: the child starts with no variables but `env`'s
    clear_env: bool,
    stdin: Option<String>,
    group: bool,
    /// `framing: "content-length"`: stdout is messages framed by
    /// `Content-Length` headers (the language server protocol's), each
    /// one read whole by `read_line`.
    framed: bool,
    /// `bytes: true`: stdout and stderr as Bytes, exactly as written
    /// (text replaces invalid UTF-8 with U+FFFD).
    bytes: bool,
}

/// Parse the shared options map (`cwd`, `env`, and for pipelines `stdin`,
/// for spawn `group`).
fn parse_opts(value: &Value, who: &str) -> Result<Opts, Value> {
    let fields = match value {
        Value::Map(m) => m.as_ref().clone(),
        Value::Struct { fields, .. } => fields.as_ref().clone(),
        _ => return Err(err(format!("{}: options must be a map or object", who))),
    };
    let pipeline = who.contains("pipeline");
    let mut opts = Opts::default();
    for (key, val) in &fields {
        match (key.as_str(), val) {
            ("cwd", Value::String(s)) => opts.cwd = Some(s.as_ref().clone()),
            ("stdin", Value::String(s)) if pipeline => opts.stdin = Some(s.as_ref().clone()),
            ("group", Value::Boolean(b)) if !pipeline => opts.group = *b,
            ("bytes", Value::Boolean(b)) => opts.bytes = *b,
            ("framing", Value::String(f)) if !pipeline => match f.as_str() {
                "content-length" => opts.framed = true,
                "lines" => opts.framed = false,
                other => {
                    return Err(err(format!(
                        "{}: framing must be \"lines\" or \"content-length\", got \"{}\"",
                        who, other
                    )));
                }
            },
            ("env", Value::Map(m)) => {
                for (k, v) in m.iter() {
                    match v {
                        Value::String(s) => opts.env.push((k.clone(), s.as_ref().clone())),
                        Value::Unit => opts.env_removed.push(k.clone()),
                        other => {
                            return Err(err(format!(
                                "{}: env values must be strings (or () to remove one), got {}",
                                who,
                                other.type_name()
                            )));
                        }
                    }
                }
            }
            ("clear_env", Value::Boolean(b)) => opts.clear_env = *b,
            (other, _) => {
                return Err(err(format!(
                    "{}: unknown or mistyped option '{}' (supported: cwd, env, clear_env, bytes, {})",
                    who,
                    other,
                    if pipeline { "stdin" } else { "group, framing" }
                )));
            }
        }
    }
    Ok(opts)
}

/// `proc.spawn(argv)` / `proc.spawn(argv, opts)` — start a child with
/// its stdin, stdout, and stderr piped, and return a live `Process`
/// handle. Both output streams begin draining immediately.
fn proc_spawn(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    // the program and its arguments as one list, as `pty.spawn` and a
    // pipeline stage take them
    let args: Vec<Value> = match args.first() {
        Some(Value::List(argv)) if args.len() <= 2 => {
            let Some(first) = argv.first() else {
                return Err(misuse("spawn: argv is empty"));
            };
            let mut out = vec![first.clone(), Value::List(Arc::new(argv[1..].to_vec()))];
            out.extend(args.into_iter().skip(1));
            out
        }
        _ => {
            return Err(misuse(
                "spawn expects (argv) or (argv, opts): the program and its arguments as one list",
            ));
        }
    };
    let program = match as_program(&args[0], "spawn") {
        Ok(p) => p,
        Err(e) => return Err(raised(e)),
    };
    let cargs = match as_args(&args[1], "spawn") {
        Ok(a) => a,
        Err(e) => return Err(raised(e)),
    };
    let Opts {
        cwd,
        env,
        env_removed,
        clear_env,
        group,
        framed,
        bytes,
        ..
    } = match args.get(2) {
        Some(opts) => match parse_opts(opts, "spawn") {
            Ok(o) => o,
            Err(e) => return Err(raised(e)),
        },
        None => Opts::default(),
    };
    if let Some(msg) = missing_dir("spawn", cwd.as_deref()) {
        return Ok(err(msg));
    }

    let mut command = match command_for(&program, clear_env, &env, &env_removed) {
        Ok(c) => c,
        Err(e) => return Ok(err(format!("spawn: failed to start '{}': {}", program, e))),
    };
    command
        .args(&cargs)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &cwd {
        command.current_dir(dir);
    }
    apply_env(&mut command, clear_env, &env, &env_removed);
    if group {
        set_own_group(&mut command);
    }

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return Ok(err(format!("spawn: failed to start '{}': {}", program, e))),
    };
    let pid = child.id();

    // Drain stdout into a line channel: read_line pops from the channel,
    // never from the pipe, so the child can outrun a slow reader.
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    // `framed` first: the pipe is taken only by the branch that drains it
    // (taking it in the test of the first branch left a plain child's
    // stdout undrained — no lines, no EOF until it exited).
    if framed && let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut r = BufReader::new(out);
            while let Some(body) = read_frame(&mut r) {
                if tx.send(body.into_bytes()).is_err() {
                    break;
                }
            }
        });
    } else if bytes && let Some(mut out) = child.stdout.take() {
        // raw chunks, as they come: `read_all` joins them
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = out.read(&mut chunk) {
                if n == 0 || tx.send(chunk[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
    } else if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            // a line at a time, its `\n` (and a `\r` before it) removed;
            // bytes that are not UTF-8 never stop the drain (they become
            // U+FFFD when the line is read as text)
            let mut r = BufReader::new(out);
            loop {
                let mut line = Vec::new();
                match r.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if line.last() == Some(&b'\n') {
                            line.pop();
                            if line.last() == Some(&b'\r') {
                                line.pop();
                            }
                        }
                        if tx.send(line).is_err() {
                            break; // receiver gone; stop draining
                        }
                    }
                }
            }
            // tx dropped here → read_line sees EOF
        });
    }

    // Drain stderr into a shared buffer so it can never fill and block the
    // child while the caller is busy reading stdout.
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));
    if let Some(mut se) = child.stderr.take() {
        let sink = stderr_buf.clone();
        std::thread::spawn(move || {
            *sink.lock().unwrap() = drain_bounded(&mut se, STDERR_KEEP);
        });
    }

    let stdin = child.stdin.take();
    let proc = Arc::new(Proc {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        out_rx: Mutex::new(rx),
        stderr: stderr_buf,
        bytes,
        pid,
        group,
    });
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    procs().lock().unwrap().insert(id, proc);
    Ok(ok(handle(id)))
}

/// One message framed by `Content-Length` headers: its body, or `None`
/// at the end of the stream (or a frame that cannot be read).
fn read_frame(r: &mut impl BufRead) -> Option<String> {
    let mut len: Option<usize> = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let t = line.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            if len.is_some() {
                break;
            }
            continue;
        }
        if let Some((k, v)) = t.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().ok();
        }
    }
    let mut buf = vec![0u8; len?];
    r.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// `proc.write_frame(p, body)` — feed the child's stdin one message,
/// framed by a `Content-Length` header (its length in bytes), as the
/// language server protocol frames them.
fn proc_write_frame(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse("write_frame expects (process, string)"));
    }
    let Value::String(body) = &args[1] else {
        return Err(misuse(format!(
            "write_frame: value must be a string, got {}",
            args[1].type_name()
        )));
    };
    let framed = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    proc_write(
        vec![args[0].clone(), Value::String(Arc::new(framed))],
        false,
    )
}

/// `proc.write(p, s)` / `proc.write_line(p, s)` — feed the child's stdin.
fn proc_write(args: Vec<Value>, newline: bool) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err(misuse("write expects (process, string)"));
    }
    let p = proc_of(&args[0])?;
    let s = match &args[1] {
        Value::String(s) => s.as_ref().clone(),
        other => {
            return Err(misuse(format!(
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
        return Err(misuse("close_stdin expects (process)"));
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
        return Err(misuse("read_line expects (process)"));
    }
    let p = proc_of(&args[0])?;
    if p.bytes {
        return Err(
            "proc.read_line: a process spawned with bytes: true is read whole, with proc.read_all"
                .into(),
        );
    }
    let rx = p.out_rx.lock().unwrap();
    match rx.recv() {
        Ok(line) => Ok(ok(Value::String(Arc::new(
            String::from_utf8_lossy(&line).into_owned(),
        )))),
        Err(_) => Ok(err("eof")),
    }
}

/// `proc.read_all(p)` — the rest of the child's stdout as one string
/// (lines rejoined with `\n`). Blocks until stdout closes.
fn proc_read_all(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse("read_all expects (process)"));
    }
    let p = proc_of(&args[0])?;
    let rx = p.out_rx.lock().unwrap();
    if p.bytes {
        let mut all = Vec::new();
        while let Ok(chunk) = rx.recv() {
            all.extend_from_slice(&chunk);
        }
        return Ok(ok(crate::stdlib::bytes::to_value(all)));
    }
    let mut lines = Vec::new();
    while let Ok(line) = rx.recv() {
        lines.push(String::from_utf8_lossy(&line).into_owned());
    }
    Ok(ok(Value::String(Arc::new(lines.join("\n")))))
}

/// How much of a child's stderr is kept: a child that floods it (a run
/// printing in a loop, a REPL left going) costs this, not all it wrote.
pub(crate) const STDERR_KEEP: usize = 4 << 20;

/// Read `r` to its end, keeping its last `keep` bytes (from a line's start
/// where one is near): what was dropped is said by a first line,
/// `[… N bytes of stderr dropped …]`.
pub(crate) fn drain_bounded(r: &mut impl Read, keep: usize) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    let mut dropped: u64 = 0;
    let mut chunk = [0u8; 65536];
    loop {
        match r.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > keep + keep / 4 {
                    let mut cut = buf.len() - keep;
                    // from the next line's start, when one is close
                    if let Some(i) = buf[cut..].iter().take(4096).position(|&b| b == b'\n') {
                        cut += i + 1;
                    }
                    buf.drain(..cut);
                    dropped += cut as u64;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if buf.len() > keep {
        let cut = buf.len() - keep;
        buf.drain(..cut);
        dropped += cut as u64;
    }
    if dropped == 0 {
        return buf;
    }
    let mut out = format!("[… {dropped} bytes of stderr dropped …]\n").into_bytes();
    out.extend_from_slice(&buf);
    out
}

/// `proc.stderr(p)` — everything the child has written to stderr. Complete
/// once the child exits; read it after `proc.wait`.
fn proc_stderr(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse("stderr expects (process)"));
    }
    let p = proc_of(&args[0])?;
    let s = p.stderr.lock().unwrap().clone();
    Ok(ok(text_or_bytes(s, p.bytes)))
}

/// Output as the caller asked for it: Bytes as written, or text with any
/// bytes that are not UTF-8 replaced by U+FFFD.
pub(crate) fn text_or_bytes(raw: Vec<u8>, bytes: bool) -> Value {
    if bytes {
        crate::stdlib::bytes::to_value(raw)
    } else {
        Value::String(Arc::new(String::from_utf8_lossy(&raw).into_owned()))
    }
}

/// A misuse of a `proc` function: raised, with the module named.
fn misuse(msg: impl Into<String>) -> Box<dyn std::error::Error> {
    let msg = msg.into();
    if msg.starts_with("proc.") {
        msg.into()
    } else {
        format!("proc.{msg}").into()
    }
}

/// An options or argument problem a helper answered as `Err(text)`,
/// raised instead (misuse raises).
fn raised(v: Value) -> Box<dyn std::error::Error> {
    match &v {
        Value::Err(inner) => match inner.as_ref() {
            Value::String(t) => misuse(t.as_ref().clone()),
            other => misuse(other.to_string()),
        },
        other => misuse(other.to_string()),
    }
}

/// An exit status as `(code, signal)`: the exit code and `()`, or `()` and
/// the signal that ended the process.
pub(crate) fn exit_parts(status: &std::process::ExitStatus) -> (Value, Value) {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal: Option<i32> = None;
    (
        status
            .code()
            .map(|c| Value::Integer(c as i64))
            .unwrap_or(Value::Unit),
        signal
            .map(|s| Value::Integer(s as i64))
            .unwrap_or(Value::Unit),
    )
}

/// `proc.wait(p)` — block until the child exits, returning `{ code,
/// signal }` (the exit code and `()`, or `()` and the signal that ended
/// it). Safe to call
/// without draining stdout first: the drainer threads keep the pipes
/// empty, so the child can always make progress to exit.
fn proc_wait(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse("wait expects (process)"));
    }
    let p = proc_of(&args[0])?;
    // Close stdin first if still open, so a filter waiting on more input
    // can reach EOF and exit instead of hanging the wait forever.
    *p.stdin.lock().unwrap() = None;
    let mut child = p.child.lock().unwrap();
    match child.wait() {
        Ok(status) => {
            let (code, signal) = exit_parts(&status);
            let mut fields = crate::ast::ValueMap::default();
            fields.insert("code".to_string(), code);
            fields.insert("signal".to_string(), signal);
            Ok(ok(Value::Struct {
                type_name: "Object".to_string(),
                fields: std::sync::Arc::new(fields),
            }))
        }
        Err(e) => Ok(err(format!("wait: {}", e))),
    }
}

/// `proc.kill(p)` — terminate the child immediately (SIGKILL). Killing an
/// already-exited child is not an error. `proc.kill(p, #{ tree: true })`
/// ends everything the child started too: the child must have been
/// spawned with `#{ group: true }`, and its whole process group is
/// killed (`taskkill /T` on Windows).
/// A signal as `proc.kill` and `pty.kill` take one: a name (`"TERM"`,
/// `"SIGTERM"`, …) or a number.
pub(crate) fn signal_of(v: &Value, who: &str) -> Result<i32, String> {
    #[cfg(unix)]
    let named = |n: &str| -> Option<i32> {
        Some(match n {
            "HUP" => libc::SIGHUP,
            "INT" => libc::SIGINT,
            "TERM" => libc::SIGTERM,
            "KILL" => libc::SIGKILL,
            "QUIT" => libc::SIGQUIT,
            "CONT" => libc::SIGCONT,
            "STOP" => libc::SIGSTOP,
            "USR1" => libc::SIGUSR1,
            "USR2" => libc::SIGUSR2,
            _ => return None,
        })
    };
    #[cfg(not(unix))]
    let named = |n: &str| -> Option<i32> { (n == "KILL").then_some(9) };
    match v {
        Value::String(t) => {
            let n = t.trim_start_matches("SIG").to_ascii_uppercase();
            named(&n).ok_or_else(|| {
                format!("{who}: unknown signal '{n}' (HUP, INT, TERM, KILL, QUIT, CONT, STOP, USR1, USR2)")
            })
        }
        Value::Integer(i) if *i > 0 && *i < 65 => Ok(*i as i32),
        other => Err(format!(
            "{who}: a signal is a name (\"TERM\") or a number, got {}",
            other.type_name()
        )),
    }
}

fn proc_kill(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.is_empty() || args.len() > 2 {
        return Err(
            "proc.kill expects (process), (process, signal) or (process, #{ tree, signal })".into(),
        );
    }
    let p = proc_of(&args[0])?;
    let mut tree = false;
    // KILL unless a signal is named: `proc.kill(p, "TERM")` asks it to end
    let mut signal: Option<i32> = None;
    if let Some(opts) = args.get(1) {
        let fields = match opts {
            Value::Map(m) => m.as_ref().clone(),
            Value::Struct { fields, .. } => fields.as_ref().clone(),
            sig @ (Value::String(_) | Value::Integer(_)) => {
                let mut f = crate::ast::ValueMap::default();
                f.insert("signal".to_string(), sig.clone());
                f
            }
            other => {
                return Err(format!(
                    "proc.kill: takes a signal or #{{ tree, signal }}, got {}",
                    other.type_name()
                )
                .into());
            }
        };
        for (key, val) in &fields {
            match (key.as_str(), val) {
                ("tree", Value::Boolean(b)) => tree = *b,
                ("signal", v) => signal = Some(signal_of(v, "proc.kill")?),
                (other, _) => {
                    return Err(format!(
                        "proc.kill: unknown or mistyped option '{}' (supported: tree, signal)",
                        other
                    )
                    .into());
                }
            }
        }
    }
    #[cfg(unix)]
    if let Some(sig) = signal {
        if tree && !p.group {
            return Err(misuse(
                "kill: a tree kill needs a process spawned with #{ group: true } \
                 (proc.spawn(argv, #{ group: true }))",
            ));
        }
        let pid = p.pid as i32;
        if pid > 0 {
            // SAFETY: kill(2) touches no memory of ours; a negative pid
            // signals the process group.
            unsafe {
                libc::kill(if tree { -pid } else { pid }, sig);
            }
        }
        return Ok(unit_ok());
    }
    #[cfg(not(unix))]
    let _ = signal;
    if tree && !p.group {
        return Err(misuse(
            "kill: a tree kill needs a process spawned with #{ group: true } \
             (proc.spawn(argv, #{ group: true }))",
        ));
    }
    let mut child = p.child.lock().unwrap();
    if tree {
        kill_tree(&mut child);
    } else {
        let _ = child.kill();
    }
    Ok(unit_ok())
}

/// `proc.pid(p)` — the child's OS process id.
fn proc_pid(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err(misuse("pid expects (process)"));
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
        return Err(misuse("pipeline expects (stages) or (stages, opts)"));
    }
    let stage_list = match &args[0] {
        Value::List(items) => items,
        _ => {
            return Err(misuse(
                "pipeline: stages must be a list of [program, ...args] lists",
            ));
        }
    };
    if stage_list.is_empty() {
        return Err(misuse("pipeline: needs at least one stage"));
    }
    // Each stage: a non-empty list whose head is the program.
    let mut stages: Vec<(String, Vec<String>)> = Vec::with_capacity(stage_list.len());
    for stage in stage_list.iter() {
        let parts = match stage {
            Value::List(p) if !p.is_empty() => p,
            _ => {
                return Err(misuse(
                    "pipeline: each stage must be a non-empty [program, ...args] list",
                ));
            }
        };
        let program = match as_program(&parts[0], "pipeline") {
            Ok(p) => p,
            Err(e) => return Err(raised(e)),
        };
        let mut cargs = Vec::with_capacity(parts.len() - 1);
        for a in parts.iter().skip(1) {
            match a {
                Value::String(s) => cargs.push(s.as_ref().clone()),
                other => {
                    return Err(misuse(format!(
                        "pipeline: stage arguments must be strings, found {}",
                        other.type_name()
                    )));
                }
            }
        }
        stages.push((program, cargs));
    }
    let Opts {
        cwd,
        env,
        env_removed,
        clear_env,
        stdin: mut stdin_data,
        bytes,
        ..
    } = match args.get(1) {
        Some(opts) => match parse_opts(opts, "pipeline") {
            Ok(o) => o,
            Err(e) => return Err(raised(e)),
        },
        None => Opts::default(),
    };
    if let Some(msg) = missing_dir("pipeline", cwd.as_deref()) {
        return Ok(err(msg));
    }

    // Spawn every stage, threading stdout → stdin down the chain.
    let n = stages.len();
    let mut children: Vec<Child> = Vec::with_capacity(n);
    let mut prev_out = None;
    for (i, (program, cargs)) in stages.iter().enumerate() {
        let mut command = match command_for(program, clear_env, &env, &env_removed) {
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
        command
            .args(cargs)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(dir) = &cwd {
            command.current_dir(dir);
        }
        apply_env(&mut command, clear_env, &env, &env_removed);
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
        stderr_threads.push(std::thread::spawn(move || match se {
            Some(mut s) => drain_bounded(&mut s, STDERR_KEEP),
            None => Vec::new(),
        }));
    }

    // Read the last stage's stdout on this thread.
    let mut final_out = Vec::new();
    if let Some(mut out) = prev_out.take() {
        let _ = out.read_to_end(&mut final_out);
    }

    // Collect exit codes in order.
    let mut codes = Vec::with_capacity(n);
    for child in &mut children {
        // a stage ended by a signal (or not waited for) has no code: ()
        let code = child
            .wait()
            .ok()
            .map(|s| exit_parts(&s).0)
            .unwrap_or(Value::Unit);
        codes.push(code);
    }
    if let Some(f) = feeder {
        let _ = f.join();
    }
    let mut stderr_all = Vec::new();
    for t in stderr_threads {
        if let Ok(s) = t.join() {
            stderr_all.extend_from_slice(&s);
        }
    }

    let final_code = codes.last().cloned().unwrap_or(Value::Unit);
    let mut fields = crate::ast::ValueMap::default();
    fields.insert("code".to_string(), final_code);
    fields.insert("stdout".to_string(), text_or_bytes(final_out, bytes));
    fields.insert("stderr".to_string(), text_or_bytes(stderr_all, bytes));
    fields.insert("codes".to_string(), Value::List(codes.into()));
    Ok(ok(Value::Struct {
        type_name: "Object".to_string(),
        fields: std::sync::Arc::new(fields),
    }))
}

#[cfg(test)]
mod stderr_bound_tests {
    use super::*;

    #[test]
    fn a_flood_of_stderr_keeps_its_last_part_and_says_what_went() {
        let mut flood: Vec<u8> = Vec::new();
        for i in 0..200_000 {
            flood.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        let total = flood.len();
        let out = drain_bounded(&mut std::io::Cursor::new(flood), 64 * 1024);
        let text = String::from_utf8(out).unwrap();
        assert!(text.len() < 64 * 1024 + 100, "{}", text.len());
        assert!(text.starts_with("[… "), "{}", &text[..40]);
        assert!(text.ends_with("line 199999\n"));
        // the kept part starts at a line
        let second = text.lines().nth(1).unwrap();
        assert!(second.starts_with("line "), "{second}");
        let dropped: usize = text[5..].split(' ').next().unwrap().parse().unwrap();
        assert_eq!(
            dropped + text.len() - text.lines().next().unwrap().len() - 1,
            total
        );
        // a quiet child: all of it, nothing said
        let small = drain_bounded(&mut std::io::Cursor::new(b"oops\n".to_vec()), 64 * 1024);
        assert_eq!(small, b"oops\n");
    }
}

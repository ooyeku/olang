//! `pty` — a child process on a pseudo-terminal: a shell, an editor, any
//! program that wants a terminal (`vim`, `less`, `top`).
//!
//! Where [`proc`](proc.rs) pipes a child's stdin and stdout, `pty` gives it
//! a terminal of its own: the child is a session leader with the pty's
//! slave as its controlling terminal (so job control, `^C` and window
//! sizes work), and the program holds the master side.
//!
//!   let p = unwrap(pty.spawn(["/bin/zsh", "-l"], #{ "cwd": "/tmp", "rows": 24, "cols": 80 }))
//!   pty.write(p, "echo hi\r")
//!   match pty.read(p, #{ "timeout_ms": 500 }) { Ok(b) => …, Err(e) => … }   // Bytes, () on a timeout
//!   pty.resize(p, 40, 120)            // TIOCSWINSZ: the foreground job hears SIGWINCH
//!   pty.kill(p)                       // SIGHUP to the session's process groups
//!   pty.wait(p, 1000)                 // #{ code, signal } once it ended, () while it runs
//!   pty.close(p)                      // hung up, the master closed, the child reaped
//!
//! `pty.read` is meant for a reader task of its own: it waits up to
//! `timeout_ms` for output, then keeps reading while more arrives within
//! `settle_ms` (up to `max` bytes), so a burst comes back as one value.
//! Writes go through a writer thread, so a large paste never blocks the
//! caller. Handles are `Pty { id }` structs into a registry, as `proc`'s
//! are. macOS and Linux; elsewhere `pty.spawn` answers an `Err`.

use crate::ast::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

type Res = Result<Value, Box<dyn std::error::Error>>;

fn s(t: &str) -> Value {
    Value::String(Arc::new(t.to_string()))
}
fn err(msg: impl Into<String>) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg.into()))))
}
fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}
fn obj(type_name: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut m = crate::ast::ValueMap::default();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Struct {
        type_name: type_name.to_string(),
        fields: Arc::new(m),
    }
}
#[allow(dead_code)]
fn vmap(fields: Vec<(&str, Value)>) -> Value {
    let mut m = crate::ast::ValueMap::default();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(m))
}

pub fn create_pty_module() -> Value {
    let mut module = crate::ast::ValueMap::default();
    for (name, arity) in [
        ("spawn", 2),
        ("read", 2),
        ("write", 2),
        ("resize", 3),
        ("kill", 2),
        ("wait", 2),
        ("pid", 1),
        ("foreground", 1),
        ("cwd", 1),
        ("close", 1),
        ("shell", 0),
        ("available", 0),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("pty.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

pub fn call_pty_function(name: &str, args: Vec<Value>) -> Res {
    #[cfg(unix)]
    {
        imp::call(name, args)
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        match name {
            "available" => Ok(Value::Boolean(false)),
            "shell" => Ok(Value::Unit),
            _ => Ok(err(format!("pty.{name}: pseudo-terminals need macOS or Linux"))),
        }
    }
}

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn handle(id: i64) -> Value {
    let mut fields = crate::ast::ValueMap::default();
    fields.insert("id".to_string(), Value::Integer(id));
    Value::Struct {
        type_name: "Pty".to_string(),
        fields: Arc::new(fields),
    }
}

fn id_of(v: &Value) -> Result<i64, String> {
    match v {
        Value::Struct { type_name, fields } if type_name == "Pty" => match fields.get("id") {
            Some(Value::Integer(i)) => Ok(*i),
            _ => Err("pty: a malformed Pty handle".into()),
        },
        other => Err(format!("pty: expected a Pty handle, got {}", other.type_name())),
    }
}

fn num_opt(m: &crate::ast::ValueMap, k: &str) -> Option<i64> {
    match m.get(k) {
        Some(Value::Integer(i)) => Some(*i),
        Some(Value::Float(f)) => Some(*f as i64),
        _ => None,
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[cfg(target_os = "macos")]
    const TIOCSCTTY: libc::c_ulong = 0x2000_7461;
    #[cfg(target_os = "macos")]
    const TIOCSWINSZ: libc::c_ulong = 0x8008_7467;
    #[cfg(not(target_os = "macos"))]
    const TIOCSCTTY: libc::c_ulong = libc::TIOCSCTTY as libc::c_ulong;
    #[cfg(not(target_os = "macos"))]
    const TIOCSWINSZ: libc::c_ulong = libc::TIOCSWINSZ as libc::c_ulong;

    /// One live child on a pseudo-terminal.
    struct Pty {
        master: OwnedFd,
        child: Mutex<Child>,
        pid: i32,
        /// The writer thread's queue; `None` once closed.
        tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
        /// What the child ended with, once seen.
        exit: Mutex<Option<(Option<i32>, Option<i32>)>>,
        /// The master answered end of file (the slave side is gone).
        eof: std::sync::atomic::AtomicBool,
    }

    fn ptys() -> &'static Mutex<HashMap<i64, Arc<Pty>>> {
        static PTYS: OnceLock<Mutex<HashMap<i64, Arc<Pty>>>> = OnceLock::new();
        PTYS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn pty_of(v: &Value) -> Result<Arc<Pty>, String> {
        let id = id_of(v)?;
        ptys()
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| "pty: this Pty was closed".to_string())
    }

    fn winsize(rows: i64, cols: i64) -> libc::winsize {
        libc::winsize {
            ws_row: rows.clamp(1, 10_000) as u16,
            ws_col: cols.clamp(1, 10_000) as u16,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }

    fn cloexec(fd: RawFd) {
        // SAFETY: fcntl on a descriptor we own.
        unsafe {
            let f = libc::fcntl(fd, libc::F_GETFD);
            if f >= 0 {
                libc::fcntl(fd, libc::F_SETFD, f | libc::FD_CLOEXEC);
            }
        }
    }

    fn nonblocking(fd: RawFd) {
        // SAFETY: fcntl on a descriptor we own.
        unsafe {
            let f = libc::fcntl(fd, libc::F_GETFL);
            if f >= 0 {
                libc::fcntl(fd, libc::F_SETFL, f | libc::O_NONBLOCK);
            }
        }
    }

    pub fn call(name: &str, args: Vec<Value>) -> Res {
        match name {
            "spawn" => spawn(args),
            "read" => read(args),
            "write" => write(args),
            "resize" => resize(args),
            "kill" => kill(args),
            "wait" => wait(args),
            "pid" => {
                let p = match pty_of(args.first().unwrap_or(&Value::Unit)) { Ok(p) => p, Err(e) => return Ok(err(e)) };
                Ok(Value::Integer(p.pid as i64))
            }
            "foreground" => foreground(args),
            "cwd" => cwd(args),
            "close" => close(args),
            "shell" => Ok(login_shell()),
            "available" => Ok(Value::Boolean(true)),
            _ => Err(format!("Unknown pty function: {name}").into()),
        }
    }

    /// The user's login shell: `$SHELL`, else the user database's, else
    /// /bin/zsh (a program started by launchd may have no `$SHELL`).
    fn login_shell() -> Value {
        if let Ok(sh) = std::env::var("SHELL")
            && !sh.is_empty()
            && std::path::Path::new(&sh).exists()
        {
            return s(&sh);
        }
        // SAFETY: getpwuid answers a pointer into static storage (or null);
        // its shell is copied out at once.
        unsafe {
            let pw = libc::getpwuid(libc::getuid());
            if !pw.is_null() && !(*pw).pw_shell.is_null() {
                let c = std::ffi::CStr::from_ptr((*pw).pw_shell);
                if let Ok(t) = c.to_str()
                    && !t.is_empty()
                {
                    return s(t);
                }
            }
        }
        s("/bin/zsh")
    }

    fn spawn(args: Vec<Value>) -> Res {
        if args.is_empty() || args.len() > 2 {
            return Err(crate::stdlib::misuse::arity("pty.spawn", "1 or 2", args.len()));
        }
        let argv: Vec<String> = match &args[0] {
            Value::List(l) => l
                .iter()
                .map(|v| match v {
                    Value::String(t) => t.as_ref().clone(),
                    other => format!("{other}"),
                })
                .collect(),
            other => {
                return Err(crate::stdlib::misuse::arg_type("pty.spawn", "argv", "a list of strings", other));
            }
        };
        if argv.is_empty() {
            return Ok(err("pty.spawn: an empty argv"));
        }
        let empty = crate::ast::ValueMap::default();
        let o = match args.get(1) {
            None | Some(Value::Unit) => &empty,
            Some(Value::Map(m)) => m.as_ref(),
            Some(Value::Struct { fields, .. }) => fields.as_ref(),
            Some(other) => return Err(crate::stdlib::misuse::arg_type("pty.spawn", "opts", "a map", other)),
        };
        for k in o.keys() {
            if !matches!(k.as_str(), "cwd" | "env" | "rows" | "cols" | "clear_env") {
                return Err(format!("pty.spawn: unknown option '{k}' (cwd, env, rows, cols, clear_env)").into());
            }
        }
        let rows = num_opt(o, "rows").unwrap_or(24);
        let cols = num_opt(o, "cols").unwrap_or(80);
        let cwd = match o.get("cwd") {
            Some(Value::String(t)) => Some(t.as_ref().clone()),
            _ => None,
        };
        if let Some(msg) = crate::stdlib::proc::missing_dir("pty.spawn", cwd.as_deref()) {
            return Ok(err(msg));
        }

        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let mut ws = winsize(rows, cols);
        // SAFETY: openpty writes two descriptors; the name and termios
        // pointers may be null.
        let r = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) };
        if r != 0 {
            return Ok(err(format!("pty.spawn: openpty failed: {}", std::io::Error::last_os_error())));
        }
        // SAFETY: openpty answered two descriptors we now own.
        let master = unsafe { OwnedFd::from_raw_fd(master) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        // neither end leaks into another child this process starts
        cloexec(master.as_raw_fd());
        cloexec(slave.as_raw_fd());

        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        if matches!(o.get("clear_env"), Some(Value::Boolean(true))) {
            cmd.env_clear();
        }
        if let Some(Value::Map(env)) = o.get("env") {
            for (k, v) in env.iter() {
                match v {
                    Value::Unit => {
                        cmd.env_remove(k);
                    }
                    Value::String(t) => {
                        cmd.env(k, t.as_str());
                    }
                    other => {
                        cmd.env(k, format!("{other}"));
                    }
                }
            }
        }
        if let Some(d) = &cwd {
            cmd.current_dir(d);
        }
        let dup = |fd: &OwnedFd| fd.try_clone().map(Stdio::from);
        let (i, out, e) = match (dup(&slave), dup(&slave), dup(&slave)) {
            (Ok(a), Ok(b), Ok(c)) => (a, b, c),
            _ => return Ok(err("pty.spawn: could not duplicate the terminal")),
        };
        cmd.stdin(i).stdout(out).stderr(e);
        // SAFETY: only async-signal-safe calls between fork and exec: a new
        // session, the slave (already fd 0) made its controlling terminal,
        // and the signals a shell expects at their defaults.
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for sig in [
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGHUP,
                    libc::SIGTERM,
                    libc::SIGTSTP,
                    libc::SIGTTIN,
                    libc::SIGTTOU,
                    libc::SIGCHLD,
                    libc::SIGWINCH,
                    libc::SIGPIPE,
                    libc::SIGALRM,
                    libc::SIGUSR1,
                    libc::SIGUSR2,
                ] {
                    libc::signal(sig, libc::SIG_DFL);
                }
                Ok(())
            });
        }
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Ok(err(format!("pty.spawn: failed to start '{}': {}", argv[0], e))),
        };
        drop(slave);
        nonblocking(master.as_raw_fd());
        let pid = child.id() as i32;

        // the writer: a queue drained into the master, waiting for room
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let wfd = match master.try_clone() {
            Ok(f) => f,
            Err(e) => return Ok(err(format!("pty.spawn: {e}"))),
        };
        std::thread::Builder::new()
            .name("pty-writer".into())
            .spawn(move || {
                while let Ok(buf) = rx.recv() {
                    if !write_all(wfd.as_raw_fd(), &buf) {
                        break;
                    }
                }
            })
            .ok();

        let p = Arc::new(Pty {
            master,
            child: Mutex::new(child),
            pid,
            tx: Mutex::new(Some(tx)),
            exit: Mutex::new(None),
            eof: std::sync::atomic::AtomicBool::new(false),
        });
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        ptys().lock().unwrap().insert(id, p);
        Ok(ok(handle(id)))
    }

    /// Write all of `buf` to a non-blocking descriptor, waiting for room;
    /// false once the other side is gone.
    fn write_all(fd: RawFd, mut buf: &[u8]) -> bool {
        while !buf.is_empty() {
            // SAFETY: writing from a live slice to a descriptor we hold.
            let n = unsafe { libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len()) };
            if n > 0 {
                buf = &buf[n as usize..];
                continue;
            }
            let e = std::io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => {
                    let mut pfd = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                    // SAFETY: one pollfd.
                    let r = unsafe { libc::poll(&mut pfd, 1, 1000) };
                    if r < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                        return false;
                    }
                    if r > 0 && pfd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 && pfd.revents & libc::POLLOUT == 0 {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }

    /// Read what is there now (up to `room` bytes) into `out`: the bytes
    /// read, or `None` at the end (the slave side closed).
    fn read_now(fd: RawFd, out: &mut Vec<u8>, room: usize) -> Option<usize> {
        let mut buf = [0u8; 65536];
        let mut total = 0;
        while total < room {
            let want = buf.len().min(room - total);
            // SAFETY: reading into a stack buffer of `want` bytes.
            let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, want) };
            if n > 0 {
                out.extend_from_slice(&buf[..n as usize]);
                total += n as usize;
                continue;
            }
            if n == 0 {
                return if total > 0 { Some(total) } else { None };
            }
            match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::EAGAIN) => return Some(total),
                Some(libc::EINTR) => continue,
                // EIO: the last process on the slave side is gone (macOS, Linux)
                _ => return if total > 0 { Some(total) } else { None },
            }
        }
        Some(total)
    }

    fn poll_in(fd: RawFd, ms: i64) -> i32 {
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd.
        let r = unsafe { libc::poll(&mut pfd, 1, ms.clamp(0, i32::MAX as i64) as i32) };
        if r > 0 && pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 { 1 } else { r.min(0) }
    }

    /// `pty.read(p, opts?)`: Ok(Bytes) with what arrived, Ok(()) when
    /// nothing did within `timeout_ms` (default 1000), Err("eof") once the
    /// child's side is closed. Having read something, it reads on while
    /// more comes within `settle_ms` (default 2), up to `max` bytes
    /// (default 1 MiB).
    fn read(args: Vec<Value>) -> Res {
        let p = match pty_of(args.first().unwrap_or(&Value::Unit)) { Ok(p) => p, Err(e) => return Ok(err(e)) };
        let empty = crate::ast::ValueMap::default();
        let o = match args.get(1) {
            Some(Value::Map(m)) => m.as_ref(),
            _ => &empty,
        };
        let timeout = num_opt(o, "timeout_ms").unwrap_or(1000);
        let settle = num_opt(o, "settle_ms").unwrap_or(2);
        let max = num_opt(o, "max").unwrap_or(1 << 20).max(1) as usize;
        if p.eof.load(Ordering::SeqCst) {
            return Ok(err("eof"));
        }
        let fd = p.master.as_raw_fd();
        let mut out = Vec::new();
        let r = poll_in(fd, timeout);
        if r <= 0 {
            return Ok(ok(Value::Unit));
        }
        let deadline = Instant::now() + Duration::from_millis(50);
        loop {
            let room = max - out.len();
            match read_now(fd, &mut out, room) {
                None => {
                    p.eof.store(true, Ordering::SeqCst);
                    break;
                }
                Some(_) => {}
            }
            if out.len() >= max || Instant::now() >= deadline || settle <= 0 {
                break;
            }
            if poll_in(fd, settle) <= 0 {
                break;
            }
        }
        if out.is_empty() {
            return Ok(if p.eof.load(Ordering::SeqCst) { err("eof") } else { ok(Value::Unit) });
        }
        Ok(ok(crate::stdlib::bytes::to_value(out)))
    }

    fn write(args: Vec<Value>) -> Res {
        if args.len() != 2 {
            return Err(crate::stdlib::misuse::arity("pty.write", "2", args.len()));
        }
        let p = match pty_of(&args[0]) { Ok(p) => p, Err(e) => return Ok(err(e)) };
        let data = match &args[1] {
            Value::String(t) => t.as_bytes().to_vec(),
            other => match crate::stdlib::bytes::bytes_of(other) {
                Ok(b) => b.to_vec(),
                Err(_) => return Err(crate::stdlib::misuse::arg_type("pty.write", "data", "a String or Bytes", other)),
            },
        };
        let tx = p.tx.lock().unwrap();
        match tx.as_ref() {
            Some(t) if t.send(data).is_ok() => Ok(ok(Value::Unit)),
            _ => Ok(err("pty.write: the terminal is closed")),
        }
    }

    fn resize(args: Vec<Value>) -> Res {
        if args.len() != 3 {
            return Err(crate::stdlib::misuse::arity("pty.resize", "3", args.len()));
        }
        let p = match pty_of(&args[0]) { Ok(p) => p, Err(e) => return Ok(err(e)) };
        let n = |v: &Value| match v {
            Value::Integer(i) => *i,
            Value::Float(f) => *f as i64,
            _ => 0,
        };
        let ws = winsize(n(&args[1]), n(&args[2]));
        // SAFETY: TIOCSWINSZ reads one winsize; the kernel sends SIGWINCH
        // to the terminal's foreground process group.
        let r = unsafe { libc::ioctl(p.master.as_raw_fd(), TIOCSWINSZ as _, &ws) };
        if r < 0 {
            return Ok(err(format!("pty.resize: {}", std::io::Error::last_os_error())));
        }
        Ok(ok(Value::Unit))
    }

    fn signal_of(v: Option<&Value>) -> Result<i32, String> {
        Ok(match v {
            None | Some(Value::Unit) => libc::SIGHUP,
            Some(Value::String(t)) => match t.trim_start_matches("SIG").to_ascii_uppercase().as_str() {
                "HUP" => libc::SIGHUP,
                "INT" => libc::SIGINT,
                "TERM" => libc::SIGTERM,
                "KILL" => libc::SIGKILL,
                "QUIT" => libc::SIGQUIT,
                "CONT" => libc::SIGCONT,
                "STOP" => libc::SIGSTOP,
                other => return Err(format!("pty.kill: unknown signal '{other}' (HUP, INT, TERM, KILL, QUIT, CONT, STOP)")),
            },
            Some(Value::Integer(i)) => *i as i32,
            Some(other) => return Err(format!("pty.kill: a signal is a name or a number, got {}", other.type_name())),
        })
    }

    /// The process groups to signal: the shell's and the terminal's
    /// foreground job's.
    fn groups(p: &Pty) -> Vec<i32> {
        let mut gs = vec![p.pid];
        // SAFETY: tcgetpgrp on the master we hold.
        let fg = unsafe { libc::tcgetpgrp(p.master.as_raw_fd()) };
        if fg > 0 && fg != p.pid {
            gs.push(fg);
        }
        gs
    }

    fn signal_groups(p: &Pty, sig: i32) {
        for g in groups(p) {
            // SAFETY: kill(2) with a negative pid signals a process group.
            unsafe {
                libc::kill(-g, sig);
            }
        }
    }

    /// `pty.kill(p, sig?)`: the signal (SIGHUP by default) to the shell's
    /// process group and the foreground job's.
    fn kill(args: Vec<Value>) -> Res {
        let p = match pty_of(args.first().unwrap_or(&Value::Unit)) { Ok(p) => p, Err(e) => return Ok(err(e)) };
        let sig = signal_of(args.get(1))?;
        signal_groups(&p, sig);
        Ok(ok(Value::Unit))
    }

    fn status(p: &Pty) -> Option<(Option<i32>, Option<i32>)> {
        if let Some(e) = *p.exit.lock().unwrap() {
            return Some(e);
        }
        let mut c = p.child.lock().unwrap();
        match c.try_wait() {
            Ok(Some(st)) => {
                use std::os::unix::process::ExitStatusExt;
                let e = (st.code(), st.signal());
                *p.exit.lock().unwrap() = Some(e);
                Some(e)
            }
            _ => None,
        }
    }

    fn exit_value(e: (Option<i32>, Option<i32>)) -> Value {
        obj("Exit", vec![
            ("code", e.0.map(|c| Value::Integer(c as i64)).unwrap_or(Value::Unit)),
            ("signal", e.1.map(|c| Value::Integer(c as i64)).unwrap_or(Value::Unit)),
        ])
    }

    /// `pty.wait(p, timeout_ms?)`: `#{ code, signal }` once the child
    /// ended (waiting up to `timeout_ms`, default 0), `()` while it runs.
    fn wait(args: Vec<Value>) -> Res {
        let p = match pty_of(args.first().unwrap_or(&Value::Unit)) { Ok(p) => p, Err(e) => return Ok(err(e)) };
        let ms = match args.get(1) {
            Some(Value::Integer(i)) => *i,
            Some(Value::Float(f)) => *f as i64,
            _ => 0,
        };
        let until = Instant::now() + Duration::from_millis(ms.max(0) as u64);
        loop {
            if let Some(e) = status(&p) {
                return Ok(ok(exit_value(e)));
            }
            if Instant::now() >= until {
                return Ok(ok(Value::Unit));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn proc_name(pid: i32) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            let mut buf = [0u8; 256];
            // SAFETY: proc_name fills at most `buf.len()` bytes.
            let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32) };
            if n > 0 {
                return Some(String::from_utf8_lossy(&buf[..n as usize]).into_owned());
            }
            None
        }
        #[cfg(not(target_os = "macos"))]
        {
            std::fs::read_to_string(format!("/proc/{pid}/comm")).ok().map(|t| t.trim().to_string())
        }
    }

    fn proc_cwd(pid: i32) -> Option<String> {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: proc_pidinfo fills one proc_vnodepathinfo.
            unsafe {
                let mut vpi: libc::proc_vnodepathinfo = std::mem::zeroed();
                let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
                let n = libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, &mut vpi as *mut _ as *mut libc::c_void, size);
                if n != size {
                    return None;
                }
                let raw = &vpi.pvi_cdir.vip_path;
                let bytes: Vec<u8> = raw.iter().flat_map(|row| row.iter()).take_while(|c| **c != 0).map(|c| *c as u8).collect();
                if bytes.is_empty() { None } else { Some(String::from_utf8_lossy(&bytes).into_owned()) }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            std::fs::read_link(format!("/proc/{pid}/cwd")).ok().map(|p| p.to_string_lossy().into_owned())
        }
    }

    /// `pty.foreground(p)`: the terminal's foreground job —
    /// `#{ pid, name, shell }` (`shell` true when it is the shell itself,
    /// at its prompt) — or `()` once the child is gone.
    fn foreground(args: Vec<Value>) -> Res {
        let p = match pty_of(args.first().unwrap_or(&Value::Unit)) { Ok(p) => p, Err(e) => return Ok(err(e)) };
        if status(&p).is_some() {
            return Ok(Value::Unit);
        }
        // SAFETY: tcgetpgrp on the master we hold.
        let fg = unsafe { libc::tcgetpgrp(p.master.as_raw_fd()) };
        let pid = if fg > 0 { fg } else { p.pid };
        Ok(obj("Foreground", vec![
            ("pid", Value::Integer(pid as i64)),
            ("name", proc_name(pid).map(|n| s(&n)).unwrap_or(Value::Unit)),
            ("shell", Value::Boolean(pid == p.pid)),
        ]))
    }

    /// `pty.cwd(p)`: the foreground job's working directory (the shell's
    /// at its prompt), or `()` when it cannot be read.
    fn cwd(args: Vec<Value>) -> Res {
        let p = match pty_of(args.first().unwrap_or(&Value::Unit)) { Ok(p) => p, Err(e) => return Ok(err(e)) };
        // SAFETY: tcgetpgrp on the master we hold.
        let fg = unsafe { libc::tcgetpgrp(p.master.as_raw_fd()) };
        let pid = if fg > 0 { fg } else { p.pid };
        Ok(proc_cwd(pid).or_else(|| proc_cwd(p.pid)).map(|c| s(&c)).unwrap_or(Value::Unit))
    }

    /// `pty.close(p)`: hang up (SIGHUP to the shell's and the foreground
    /// job's groups), close the master, and reap the child — after a
    /// grace of 500 ms, SIGKILL to what is left. Answers at once; the
    /// handle is no longer valid.
    fn close(args: Vec<Value>) -> Res {
        let id = id_of(args.first().unwrap_or(&Value::Unit))?;
        let p = match ptys().lock().unwrap().remove(&id) {
            Some(p) => p,
            None => return Ok(ok(Value::Unit)),
        };
        let gs = groups(&p);
        if status(&p).is_none() {
            signal_groups(&p, libc::SIGHUP);
        }
        p.tx.lock().unwrap().take();
        std::thread::Builder::new()
            .name("pty-reaper".into())
            .spawn(move || {
                // What the child writes as it ends is read and dropped: a
                // session leader's exit waits for its terminal's output to
                // drain, so a master nobody reads keeps a hung-up shell
                // exiting forever (macOS: `?Es`).
                let fd = p.master.as_raw_fd();
                let drain = || {
                    let mut buf = [0u8; 4096];
                    // SAFETY: read(2) into our buffer from the master we
                    // hold, non-blocking.
                    while unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) } > 0 {}
                };
                let wait_for = |ms: u64| {
                    let until = Instant::now() + Duration::from_millis(ms);
                    loop {
                        drain();
                        if status(&p).is_some() || Instant::now() >= until {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                };
                wait_for(500);
                if status(&p).is_none() {
                    for g in &gs {
                        // SAFETY: kill(2) of a process group.
                        unsafe {
                            libc::kill(-g, libc::SIGKILL);
                        }
                    }
                    wait_for(5000);
                }
                let pid = p.pid;
                let reaped = status(&p).is_some();
                // the master closes as the last reference goes
                drop(p);
                if !reaped {
                    // its terminal gone, it can end now: reaped, not a zombie
                    // SAFETY: waitpid(2) on our own child.
                    unsafe {
                        libc::waitpid(pid, std::ptr::null_mut(), 0);
                    }
                }
            })
            .ok();
        Ok(ok(Value::Unit))
    }
}

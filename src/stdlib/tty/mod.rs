//! `tty` — the terminal as an interactive device: raw mode, the
//! alternate screen, decoded key / mouse / paste / focus / resize /
//! signal events on a channel, unbuffered writes, capability queries,
//! and a restore guard that gives the terminal back however the process
//! ends. It is the layer a full-screen program (Heddle) stands on; the
//! line-oriented `os.is_tty` / `os.read_line` and the `term` styling
//! helpers stay as they are.
//!
//!   let h = unwrap(tty.enter(#{ "alt_screen": true, "hide_cursor": true }))
//!   let events = tty.events(h)
//!   tty.write(h, "press q to quit\r\n")
//!   ...
//!   tty.leave(h)
//!
//! One handle is entered at a time. Everything `enter` changes is
//! recorded in the restore guard ([`guard`]) before it is changed, and
//! the guard restores it on `leave`, on the program's end (a raise
//! included, before the error is printed), on `os.exit`, on a panic, and
//! on SIGTERM / SIGHUP / SIGINT.
//!
//! Input: on Unix a reader thread decodes stdin with [`decode`] (its
//! header says why that is not crossterm's parser); on Windows crossterm
//! reads the console's input records. Either way the events are the same
//! maps, and replies to `tty.query` never reach the events channel.

pub mod decode;
pub mod guard;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use crate::ast::Value;
use crate::stdlib::chan::ChannelSender;
use decode::Input;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

type Res = Result<Value, Box<dyn std::error::Error>>;

pub fn create_tty_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("enter", 1),
        ("leave", 1),
        ("events", 1),
        ("size", 0),
        ("write", 2),
        ("query", 3),
        ("is_tty", 0),
        ("suspend", 1),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("tty.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: Arc::new(module),
    }
}

pub fn call_tty_function(name: &str, args: Vec<Value>) -> Res {
    match name {
        "enter" => tty_enter(args),
        "leave" => tty_leave(args),
        "events" => tty_events(args),
        "size" => tty_size(args),
        "write" => tty_write(args),
        "query" => tty_query(args),
        "is_tty" => tty_is_tty(args),
        "suspend" => tty_suspend(args),
        _ => Err(format!("Unknown tty function: {}", name).into()),
    }
}

/// Restore the terminal if a `tty` handle is entered: stop the reader,
/// undo every mode, close the events channel. Idempotent, and a no-op
/// for a program that never entered. The CLI calls it when a program's
/// evaluation returns (before an uncaught error is printed), and
/// `os.exit` before it exits.
pub fn restore_terminal() {
    // try_lock: this runs on exit paths, and must never wait on a
    // session lock a stuck thread holds — the guard alone suffices.
    if let Ok(mut slot) = SESSION.try_lock()
        && let Some(session) = slot.take()
    {
        session.shutdown();
        return;
    }
    guard::restore();
}

// ── options ──────────────────────────────────────────────────────────

/// What `tty.enter` turns on. `raw` defaults to on; everything else to off.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub raw: bool,
    pub alt_screen: bool,
    pub mouse: bool,
    pub paste: bool,
    pub focus: bool,
    pub kitty_keys: bool,
    pub hide_cursor: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            raw: true,
            alt_screen: false,
            mouse: false,
            paste: false,
            focus: false,
            kitty_keys: false,
            hide_cursor: false,
        }
    }
}

const OPTION_NAMES: &[&str] = &[
    "raw",
    "alt_screen",
    "mouse",
    "paste",
    "focus",
    "kitty_keys",
    "hide_cursor",
];

impl Options {
    fn parse(v: Option<&Value>) -> Result<Options, Box<dyn std::error::Error>> {
        let mut o = Options::default();
        let map = match v {
            None | Some(Value::Unit) => return Ok(o),
            Some(Value::Map(m)) => m,
            Some(Value::Struct { type_name, fields }) if type_name == "Object" => fields,
            Some(other) => {
                return Err(crate::stdlib::misuse::arg_type(
                    "tty.enter",
                    "opts",
                    "a map of Bool options",
                    other,
                ));
            }
        };
        for (k, v) in map.iter() {
            let b = match v {
                Value::Boolean(b) => *b,
                other => {
                    return Err(crate::stdlib::misuse::arg_type(
                        "tty.enter",
                        &format!("option \"{k}\""),
                        "a Bool",
                        other,
                    ));
                }
            };
            match k.as_str() {
                "raw" => o.raw = b,
                "alt_screen" => o.alt_screen = b,
                "mouse" => o.mouse = b,
                "paste" => o.paste = b,
                "focus" => o.focus = b,
                "kitty_keys" => o.kitty_keys = b,
                "hide_cursor" => o.hide_cursor = b,
                other => {
                    return Err(crate::stdlib::misuse::arg_value(
                        "tty.enter",
                        &format!(
                            "unknown option \"{other}\" (the options are {})",
                            OPTION_NAMES.join(", ")
                        ),
                    ));
                }
            }
        }
        Ok(o)
    }

    /// The bytes that turn the options' modes on.
    pub fn enter_bytes(&self) -> String {
        let mut s = String::new();
        if self.alt_screen {
            s.push_str("\x1b[?1049h");
        }
        if self.hide_cursor {
            s.push_str("\x1b[?25l");
        }
        // Mouse reporting is VT on Unix; on Windows it is a console mode
        // (windows.rs), and the kitty protocol does not apply.
        #[cfg(not(windows))]
        {
            if self.mouse {
                // Clicks, drags, all motion; SGR coordinates (no 223 limit).
                s.push_str("\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
            }
            if self.kitty_keys {
                // Push "disambiguate escape codes" onto the kitty keyboard
                // stack; a terminal without the protocol ignores it.
                s.push_str("\x1b[>1u");
            }
        }
        if self.paste {
            s.push_str("\x1b[?2004h");
        }
        if self.focus {
            s.push_str("\x1b[?1004h");
        }
        s
    }

    /// The bytes that undo `enter_bytes`, in reverse order — plus an end
    /// to any synchronized update and an attribute reset, so a program
    /// that dies mid-frame does not leave the terminal frozen or coloured,
    /// and the cursor shown whether or not `hide_cursor` hid it.
    pub fn leave_bytes(&self) -> String {
        let mut s = String::from("\x1b[?2026l\x1b[0m");
        if self.focus {
            s.push_str("\x1b[?1004l");
        }
        if self.paste {
            s.push_str("\x1b[?2004l");
        }
        #[cfg(not(windows))]
        {
            if self.kitty_keys {
                s.push_str("\x1b[<u");
            }
            if self.mouse {
                s.push_str("\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l");
            }
        }
        s.push_str("\x1b[?25h");
        if self.alt_screen {
            s.push_str("\x1b[?1049l");
        }
        s
    }
}

// ── the session ──────────────────────────────────────────────────────

/// The one entered terminal.
struct Session {
    id: i64,
    opts: Options,
    events: Value,
    sender: ChannelSender,
    stop: Arc<AtomicBool>,
    query: Arc<QuerySlot>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Session {
    /// Stop the reader, restore the terminal, close the channel.
    fn shutdown(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        unix::wake();
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        guard::restore();
        self.sender.close();
    }
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static NEXT_ID: AtomicI64 = AtomicI64::new(1);

/// A query waiting for its reply: the reader hands replies here.
#[derive(Default)]
struct QuerySlot {
    waiting: Mutex<Option<crossbeam_channel::Sender<String>>>,
    /// Held for a whole query, so two never interleave.
    one_at_a_time: Mutex<()>,
}

/// What the reader thread needs.
pub(crate) struct ReaderCtx {
    stop: Arc<AtomicBool>,
    sender: ChannelSender,
    query: Arc<QuerySlot>,
}

impl ReaderCtx {
    fn send(&self, v: Value) {
        let _ = self.sender.send(v);
    }

    #[cfg_attr(windows, allow(dead_code))]
    fn querying(&self) -> bool {
        self.query
            .waiting
            .lock()
            .map(|w| w.is_some())
            .unwrap_or(false)
    }

    /// Route decoded input: replies to a waiting query (or nowhere),
    /// everything else to the events channel.
    fn deliver(&self, inputs: Vec<Input>) {
        for input in inputs {
            match input {
                Input::Reply(r) => {
                    if let Ok(w) = self.query.waiting.lock()
                        && let Some(tx) = w.as_ref()
                    {
                        let _ = tx.send(r);
                    }
                }
                other => self.send(input_value(&other)),
            }
        }
    }

    fn resize(&self, cols: u16, rows: u16) {
        self.send(event(
            "resize",
            vec![
                ("cols", Value::Integer(cols as i64)),
                ("rows", Value::Integer(rows as i64)),
            ],
        ));
    }

    fn signal(&self, name: &str) {
        self.send(event("signal", vec![("name", s(name))]));
    }

    /// Input ended (the terminal hung up): say so, and close the channel.
    fn eof(&self) {
        self.signal("eof");
        self.sender.close();
    }

    /// Turn the signals the guard noted into events.
    #[cfg(unix)]
    fn signals(&self, bits: u32) {
        let mut resized = bits & guard::SIG_WINCH != 0;
        if bits & guard::SIG_TSTP != 0 {
            self.signal("tstp");
        }
        if bits & guard::SIG_CONT != 0 {
            guard::resume();
            self.signal("cont");
            resized = true;
        }
        if resized && let Ok((c, r)) = unix::size() {
            self.resize(c, r);
        }
        for (bit, name) in [
            (guard::SIG_TERM, "term"),
            (guard::SIG_HUP, "hup"),
            (guard::SIG_INT, "int"),
        ] {
            if bits & bit != 0 {
                self.signal(name);
            }
        }
    }
}

// ── values ───────────────────────────────────────────────────────────

fn s(text: &str) -> Value {
    Value::String(Arc::new(text.to_string()))
}

fn event(kind: &str, fields: Vec<(&str, Value)>) -> Value {
    let mut m = HashMap::new();
    m.insert("kind".to_string(), s(kind));
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(m))
}

/// The event map for one decoded input (public so the tests can pin the
/// shapes).
pub fn input_value(input: &Input) -> Value {
    match input {
        Input::Key(k) => event(
            "key",
            vec![
                ("key", s(&k.key)),
                ("text", s(&k.text)),
                ("ctrl", Value::Boolean(k.ctrl)),
                ("alt", Value::Boolean(k.alt)),
                ("shift", Value::Boolean(k.shift)),
                ("super", Value::Boolean(k.super_)),
                ("chord", s(&k.chord())),
            ],
        ),
        Input::Paste(text) => event("paste", vec![("text", s(text))]),
        Input::Mouse(m) => {
            let mut mods = HashMap::new();
            mods.insert("ctrl".to_string(), Value::Boolean(m.ctrl));
            mods.insert("alt".to_string(), Value::Boolean(m.alt));
            mods.insert("shift".to_string(), Value::Boolean(m.shift));
            event(
                "mouse",
                vec![
                    ("action", s(m.action)),
                    ("button", s(m.button)),
                    ("x", Value::Integer(m.x as i64)),
                    ("y", Value::Integer(m.y as i64)),
                    ("mods", Value::Map(Arc::new(mods))),
                ],
            )
        }
        Input::Focus(on) => event("focus", vec![("on", Value::Boolean(*on))]),
        Input::Reply(r) => event("reply", vec![("text", s(r))]),
    }
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

fn err(msg: impl Into<String>) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg.into()))))
}

fn handle(id: i64) -> Value {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), Value::Integer(id));
    Value::Struct {
        type_name: "Tty".to_string(),
        fields: Arc::new(fields),
    }
}

fn id_of(function: &str, v: Option<&Value>) -> Result<i64, Box<dyn std::error::Error>> {
    match v {
        Some(Value::Struct { type_name, fields }) if type_name == "Tty" => match fields.get("id") {
            Some(Value::Integer(id)) => Ok(*id),
            _ => Err(format!("{function}: malformed Tty handle").into()),
        },
        Some(other) => Err(crate::stdlib::misuse::arg_type(
            function,
            "the handle",
            "a Tty handle (from tty.enter)",
            other,
        )),
        None => Err(crate::stdlib::misuse::arity(function, "a handle", 0)),
    }
}

fn arity(function: &str, args: &[Value], want: usize) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() != want {
        return Err(crate::stdlib::misuse::arity(
            function,
            &want.to_string(),
            args.len(),
        ));
    }
    Ok(())
}

// ── the functions ────────────────────────────────────────────────────

fn stdin_is_tty() -> bool {
    #[cfg(unix)]
    return unix::is_tty(libc::STDIN_FILENO);
    #[cfg(windows)]
    return windows::is_tty_in();
    #[cfg(not(any(unix, windows)))]
    false
}

fn stdout_is_tty() -> bool {
    #[cfg(unix)]
    return unix::is_tty(libc::STDOUT_FILENO);
    #[cfg(windows)]
    return windows::is_tty_out();
    #[cfg(not(any(unix, windows)))]
    false
}

fn tty_is_tty(args: Vec<Value>) -> Res {
    arity("tty.is_tty", &args, 0)?;
    Ok(Value::Boolean(stdin_is_tty() && stdout_is_tty()))
}

fn tty_size(args: Vec<Value>) -> Res {
    arity("tty.size", &args, 0)?;
    #[cfg(unix)]
    let size = unix::size();
    #[cfg(windows)]
    let size = windows::size();
    #[cfg(not(any(unix, windows)))]
    let size: Result<(u16, u16), String> = Err("tty.size: no terminal on this platform".into());
    Ok(match size {
        Ok((c, r)) => ok(Value::Tuple(Arc::new(vec![
            Value::Integer(c as i64),
            Value::Integer(r as i64),
        ]))),
        Err(e) => err(e),
    })
}

fn tty_enter(args: Vec<Value>) -> Res {
    if args.len() > 1 {
        return Err(crate::stdlib::misuse::arity(
            "tty.enter",
            "0 or 1",
            args.len(),
        ));
    }
    let opts = Options::parse(args.first())?;
    if !stdin_is_tty() {
        return Ok(err("tty.enter: stdin is not a terminal"));
    }
    if !stdout_is_tty() {
        return Ok(err("tty.enter: stdout is not a terminal"));
    }
    let mut slot = SESSION.lock().map_err(|_| "tty: session lock poisoned")?;
    if slot.is_some() {
        return Ok(err(
            "tty.enter: the terminal is already entered — leave the current handle first",
        ));
    }
    if let Err(e) = guard::install() {
        return Ok(err(e));
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = opts;
        return Ok(err("tty.enter: no terminal support on this platform"));
    }
    #[cfg(any(unix, windows))]
    {
        let enter = opts.enter_bytes().into_bytes();
        let leave = opts.leave_bytes().into_bytes();
        #[cfg(unix)]
        {
            let saved = match unix::get_termios() {
                Ok(t) => t,
                Err(e) => return Ok(err(e)),
            };
            let raw = opts.raw.then(|| unix::raw_of(&saved));
            // Armed before anything changes: a signal from here on
            // restores to `saved`, which is what the terminal still is.
            guard::arm(guard::Snapshot {
                leave,
                enter: enter.clone(),
                saved,
                raw,
            });
            if let Some(raw) = &raw
                && let Err(e) = unix::set_termios(raw)
            {
                guard::restore();
                return Ok(err(e));
            }
            // Anything print() left buffered goes out before the modes.
            {
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            unix::write_fd(libc::STDOUT_FILENO, &enter);
        }
        #[cfg(windows)]
        {
            let snap = guard::Snapshot {
                leave,
                enter,
                raw: opts.raw,
                mouse: opts.mouse,
            };
            windows::enter(&snap);
            guard::arm(snap);
        }
        let (events, sender) = crate::stdlib::chan::channel_with_sender();
        let stop = Arc::new(AtomicBool::new(false));
        let query = Arc::new(QuerySlot::default());
        let ctx = ReaderCtx {
            stop: stop.clone(),
            sender: sender.clone(),
            query: query.clone(),
        };
        #[cfg(unix)]
        let body = move || unix::reader(ctx);
        #[cfg(windows)]
        let body = move || windows::reader(ctx);
        let reader = match std::thread::Builder::new()
            .name("tty-reader".to_string())
            .spawn(body)
        {
            Ok(r) => r,
            Err(e) => {
                guard::restore();
                return Ok(err(format!(
                    "tty.enter: cannot start the input reader: {e}"
                )));
            }
        };
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        *slot = Some(Session {
            id,
            opts,
            events,
            sender,
            stop,
            query,
            reader: Some(reader),
        });
        Ok(ok(handle(id)))
    }
}

fn tty_leave(args: Vec<Value>) -> Res {
    arity("tty.leave", &args, 1)?;
    let id = id_of("tty.leave", args.first())?;
    let session = {
        let mut slot = SESSION.lock().map_err(|_| "tty: session lock poisoned")?;
        match slot.as_ref() {
            Some(s) if s.id == id => slot.take(),
            _ => None,
        }
    };
    if let Some(session) = session {
        session.shutdown();
    }
    Ok(Value::Unit)
}

/// Run `f` on the session `id` names, or answer `Err` naming why not.
fn with_session<T>(function: &str, id: i64, f: impl FnOnce(&Session) -> T) -> Result<T, String> {
    let slot = SESSION
        .lock()
        .map_err(|_| "tty: session lock poisoned".to_string())?;
    match slot.as_ref() {
        Some(s) if s.id == id => Ok(f(s)),
        _ => Err(format!("{function}: the handle has left the terminal")),
    }
}

fn tty_events(args: Vec<Value>) -> Res {
    arity("tty.events", &args, 1)?;
    let id = id_of("tty.events", args.first())?;
    match with_session("tty.events", id, |s| s.events.clone()) {
        Ok(ch) => Ok(ch),
        Err(e) => Err(e.into()),
    }
}

fn bytes_of(function: &str, v: &Value) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    match v {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::Native(_) => match crate::stdlib::bytes::bytes_of(v) {
            Ok(b) => Ok(b.to_vec()),
            Err(_) => Err(crate::stdlib::misuse::arg_type(
                function,
                "data",
                "a String or Bytes",
                v,
            )),
        },
        other => Err(crate::stdlib::misuse::arg_type(
            function,
            "data",
            "a String or Bytes",
            other,
        )),
    }
}

/// Write straight to the terminal: flush anything `print` buffered, then
/// the bytes, then flush — nothing waits for a newline.
fn write_now(bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    out.write_all(bytes)?;
    out.flush()
}

fn tty_write(args: Vec<Value>) -> Res {
    arity("tty.write", &args, 2)?;
    let id = id_of("tty.write", args.first())?;
    let bytes = bytes_of("tty.write", &args[1])?;
    if let Err(e) = with_session("tty.write", id, |_| ()) {
        return Ok(err(e));
    }
    // A test hook, compiled into debug builds only: the restore guard's
    // panic path needs a Rust panic an olang program can cause, and no
    // correct runtime offers one. `OLANG_TTY_TEST_PANIC=<text>` panics
    // when a program writes exactly that text (tests/tty_test.rs).
    #[cfg(debug_assertions)]
    if let Ok(trigger) = std::env::var("OLANG_TTY_TEST_PANIC")
        && trigger.as_bytes() == bytes.as_slice()
    {
        panic!("forced panic (OLANG_TTY_TEST_PANIC)");
    }
    Ok(match write_now(&bytes) {
        Ok(()) => ok(Value::Unit),
        Err(e) => err(format!("tty.write: {e}")),
    })
}

fn tty_query(args: Vec<Value>) -> Res {
    arity("tty.query", &args, 3)?;
    let id = id_of("tty.query", args.first())?;
    let seq = bytes_of("tty.query", &args[1])?;
    let timeout = match &args[2] {
        Value::Integer(ms) if *ms >= 0 => *ms as u64,
        other => {
            return Err(crate::stdlib::misuse::arg_type(
                "tty.query",
                "timeout_ms",
                "a non-negative Int",
                other,
            ));
        }
    };
    let (query, raw) = match with_session("tty.query", id, |s| (s.query.clone(), s.opts.raw)) {
        Ok(q) => q,
        Err(e) => return Ok(err(e)),
    };
    if cfg!(windows) {
        return Ok(err(
            "tty.query: unsupported on Windows (the console delivers input records, not reply bytes)",
        ));
    }
    if !raw {
        return Ok(err(
            "tty.query: needs raw mode (a cooked terminal would echo the reply and hold it for a newline)",
        ));
    }
    let _one = query
        .one_at_a_time
        .lock()
        .map_err(|_| "tty: query lock poisoned")?;
    let (tx, rx) = crossbeam_channel::unbounded::<String>();
    *query
        .waiting
        .lock()
        .map_err(|_| "tty: query lock poisoned")? = Some(tx);
    let written = write_now(&seq);
    let reply = match written {
        Err(e) => Err(format!("tty.query: {e}")),
        Ok(()) => match rx.recv_timeout(std::time::Duration::from_millis(timeout)) {
            Ok(first) => {
                // Replies decoded from the same read come along.
                let mut all = first;
                while let Ok(more) = rx.try_recv() {
                    all.push_str(&more);
                }
                Ok(all)
            }
            Err(_) => Err("timeout".to_string()),
        },
    };
    *query
        .waiting
        .lock()
        .map_err(|_| "tty: query lock poisoned")? = None;
    Ok(match reply {
        Ok(r) => ok(Value::String(Arc::new(r))),
        Err(e) => err(e),
    })
}

fn tty_suspend(args: Vec<Value>) -> Res {
    arity("tty.suspend", &args, 1)?;
    let id = id_of("tty.suspend", args.first())?;
    if let Err(e) = with_session("tty.suspend", id, |_| ()) {
        return Ok(err(e));
    }
    #[cfg(unix)]
    {
        // SIGTSTP to our process group, as the shell's ctrl+z would send:
        // the guard's action restores the terminal and stops; after `fg`
        // (SIGCONT) the process continues here and re-enters.
        // SAFETY: kill(2) on our own process group.
        unsafe {
            libc::kill(0, libc::SIGTSTP);
        }
        guard::resume();
        Ok(ok(Value::Unit))
    }
    #[cfg(not(unix))]
    Ok(err(
        "tty.suspend: unsupported on this platform (no job control)",
    ))
}

/// Whether a handle is entered right now (for tests and diagnostics).
pub fn entered() -> bool {
    guard::armed()
}

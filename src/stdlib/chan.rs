//! `chan` — typed message passing between `spawn`ed tasks. Channels are
//! multi-producer multi-consumer queues of olang values: `chan.new()`
//! makes an unbounded channel, `chan.bounded(n)` one that holds at most
//! `n` in-flight messages (senders block when it is full; `0` makes a
//! rendezvous channel where send waits for a receiver). Closing is
//! cooperative and drains: after `chan.close(c)` sends fail, but queued
//! messages are still received before `recv` reports the close.
//! Everything fallible speaks Result, so `?` and `match` drive the
//! control flow.
//!
//! A handle is a `Value::Native` that *owns* the channel through an `Arc`,
//! so the channel lives exactly as long as some olang value references it
//! and is freed when the last handle drops — a program that opens a
//! channel per tick does not accumulate them. Cloning a handle (including
//! across the `spawn` boundary) shares the one underlying channel by
//! reference count; no process-wide registry, so nothing to leak.

use crate::ast::Value;
use crate::native::{NativeHandle, NativeObject};
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};

// ─── The stall detector ──────────────────────────────────────────────
//
// A `chan.recv` with no live sender used to hang the program forever,
// silently. Detection rests on two registries and one proof:
//
// - The *census*: every thread that can run olang code — the main
//   interpreter thread, `spawn` workers, http workers, parallel-map
//   scope workers — holds a `LiveGuard` for its lifetime. An idle http
//   worker counts as live because an incoming request can wake it, so
//   a program serving traffic never aborts (external input may arrive).
// - The *parked set*: every unbounded wait (a blocking recv, a bounded
//   send against a full queue, a `task.join`) registers a park entry
//   naming its thread and site for as long as it blocks.
//
// Blocking waits are polls under the hood (a short timeout per tick).
// On each tick the waiter samples (generation, parked, live), where
// the generation counter bumps on every park and unpark. If two
// consecutive samples agree, no thread parked or unparked for a whole
// tick — and if parked == live at both, every thread that could ever
// send, receive, or finish sat blocked in an unbounded wait the entire
// time. Nothing internal can wake anyone (a wake requires a running
// thread), and no external wake exists (an http worker would count
// live but never parks) — a proven deadlock. The waiter prints every
// parked site and aborts, because "a report now" beats "a hang
// forever". `OLANG_STALL_ABORT=0` keeps the old hang for embedders.
//
// Threads outside the census (a host embedding the interpreter on its
// own thread) park and are reported, but never trigger the abort —
// the proof needs the census to be the whole world, and for them it
// is not.

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PARK_GEN: AtomicU64 = AtomicU64::new(0);
static PARK_SEQ: AtomicU64 = AtomicU64::new(0);
static PARKED: OnceLock<Mutex<HashMap<u64, ParkSite>>> = OnceLock::new();
static CHAN_SEQ: AtomicU64 = AtomicU64::new(1);

/// How often a parked thread wakes to run the stall check, and the
/// resolution of the deadlock proof (two stable consecutive ticks).
const STALL_TICK_MS: u64 = 250;

#[derive(Clone)]
struct ParkSite {
    thread: String,
    what: String,
    since: std::time::Instant,
}

fn parked() -> &'static Mutex<HashMap<u64, ParkSite>> {
    PARKED.get_or_init(|| Mutex::new(HashMap::new()))
}

thread_local! {
    /// Whether this thread holds a LiveGuard — only census threads may
    /// prove a stall (for them the census is the whole world).
    static IN_CENSUS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Held for the lifetime of a thread that runs olang code. See the
/// module comment: the stall proof is sound exactly because every such
/// thread is counted.
pub struct LiveGuard(());

/// Register the current thread in the live census.
pub fn live_guard() -> LiveGuard {
    LIVE.fetch_add(1, Ordering::SeqCst);
    IN_CENSUS.with(|c| c.set(true));
    LiveGuard(())
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
        IN_CENSUS.with(|c| c.set(false));
    }
}

/// Enter the parked set. The token unparks; the guard pattern is not
/// used because unpark order interleaves with lock scopes.
fn park(what: String) -> u64 {
    let token = PARK_SEQ.fetch_add(1, Ordering::SeqCst);
    let site = ParkSite {
        thread: std::thread::current().name().unwrap_or("?").to_string(),
        what,
        since: std::time::Instant::now(),
    };
    parked().lock().unwrap().insert(token, site);
    PARK_GEN.fetch_add(1, Ordering::SeqCst);
    token
}

fn unpark(token: u64) {
    parked().lock().unwrap().remove(&token);
    PARK_GEN.fetch_add(1, Ordering::SeqCst);
}

/// One sample of the world, taken by a parked thread on its tick.
/// `last` carries the previous tick's generation; two agreeing samples
/// with parked == live prove the stall (see the module comment).
fn stall_tick(last_gen: &mut Option<u64>) {
    if !IN_CENSUS.with(|c| c.get()) {
        return;
    }
    let generation = PARK_GEN.load(Ordering::SeqCst);
    let live = LIVE.load(Ordering::SeqCst);
    let parked_count = parked().lock().unwrap().len();
    let stable = *last_gen == Some(generation);
    *last_gen = Some(generation);
    if !(stable && live > 0 && parked_count == live) {
        return;
    }
    if matches!(
        std::env::var("OLANG_STALL_ABORT").as_deref(),
        Ok("0") | Ok("false")
    ) {
        return;
    }
    let sites = parked().lock().unwrap().clone();
    let mut lines: Vec<String> = sites
        .values()
        .map(|s| {
            format!(
                "  {}: {} (waiting {:.1}s)",
                s.thread,
                s.what,
                s.since.elapsed().as_secs_f64()
            )
        })
        .collect();
    lines.sort();
    eprintln!(
        "deadlock: every live thread is blocked on an unbounded wait — nothing can ever send\n{}\n\
         hint: bound the wait (chan.recv_timeout, task.join_timeout), or chan.close the \
         channel when the senders are done. OLANG_STALL_ABORT=0 disables this abort.",
        lines.join("\n")
    );
    std::process::exit(101);
}

/// A park entry owned by a blocking wait outside this module (today:
/// `task.join`). Ticking it runs the stall check at most once per
/// `STALL_TICK_MS`; dropping it unparks.
pub struct ParkToken {
    token: u64,
    last_gen: std::cell::Cell<Option<u64>>,
    last_tick: std::cell::Cell<std::time::Instant>,
}

/// Park the current thread under `what`. `None` on wasm, where there
/// is no second thread to be deadlocked against.
pub fn parked_token(what: String) -> Option<ParkToken> {
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    Some(ParkToken {
        token: park(what),
        last_gen: std::cell::Cell::new(None),
        last_tick: std::cell::Cell::new(std::time::Instant::now()),
    })
}

impl ParkToken {
    pub fn tick(&self) {
        if (self.last_tick.get().elapsed().as_millis() as u64) < STALL_TICK_MS {
            return;
        }
        self.last_tick.set(std::time::Instant::now());
        let mut last = self.last_gen.get();
        stall_tick(&mut last);
        self.last_gen.set(last);
    }
}

impl Drop for ParkToken {
    fn drop(&mut self) {
        unpark(self.token);
    }
}

/// The parked sites right now, for `task.parked()`: (thread, what,
/// waited ms). Advisory — a site may unpark between the snapshot and
/// its use.
pub fn parked_sites() -> Vec<(String, String, u64)> {
    let mut out: Vec<(String, String, u64)> = parked()
        .lock()
        .unwrap()
        .values()
        .map(|s| {
            (
                s.thread.clone(),
                s.what.clone(),
                s.since.elapsed().as_millis() as u64,
            )
        })
        .collect();
    out.sort();
    out
}

/// One channel's two ends. The sender is cloned out of its lock before
/// use so a blocking bounded send never holds it; the receiver stays
/// locked across a blocking recv, which is what makes concurrent
/// consumers take turns (each message goes to exactly one).
struct Chan {
    /// Sequential id, for `chan.stat` and the stall report.
    id: u64,
    tx: Mutex<Option<Tx>>,
    rx: Mutex<Receiver<Value>>,
    /// Messages queued: +1 on a successful send, -1 on a successful
    /// receive. Advisory (reads race sends), which is all `chan.stat`
    /// promises.
    depth: AtomicI64,
    recv_waiting: AtomicUsize,
    send_waiting: AtomicUsize,
}

#[derive(Clone)]
enum Tx {
    Unbounded(Sender<Value>),
    Bounded(SyncSender<Value>),
}

/// The native handle wrapping a channel. Owning the `Arc<Chan>` here — in
/// the olang value itself — is what makes channel lifetime reference-counted
/// rather than registry-pinned.
struct ChanObject(Arc<Chan>);

impl std::fmt::Debug for ChanObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<channel>")
    }
}

impl NativeObject for ChanObject {
    fn module(&self) -> &'static str {
        "chan"
    }
    fn type_name(&self) -> &'static str {
        "Channel"
    }
    fn display(&self) -> String {
        "<channel>".to_string()
    }
    fn native_eq(&self, other: &dyn NativeObject) -> bool {
        // Identity: two handles are equal iff they carry the same channel.
        other
            .as_any()
            .downcast_ref::<ChanObject>()
            .map(|o| Arc::ptr_eq(&self.0, &o.0))
            .unwrap_or(false)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn create_chan_module() -> Value {
    let mut module = HashMap::new();
    for (name, arity) in [
        ("new", 0),
        ("bounded", 1),
        ("send", 2),
        ("recv", 1),
        ("try_recv", 1),
        ("recv_timeout", 2),
        ("close", 1),
        ("stat", 1),
    ] {
        module.insert(
            name.to_string(),
            Value::Builtin(crate::ast::BuiltinFunction {
                name: format!("chan.{}", name),
                arity,
            }),
        );
    }
    Value::Struct {
        type_name: "Module".to_string(),
        fields: std::sync::Arc::new(module),
    }
}

pub fn call_chan_function(
    name: &str,
    args: Vec<Value>,
) -> Result<Value, Box<dyn std::error::Error>> {
    match name {
        "new" => chan_new(args),
        "bounded" => chan_bounded(args),
        "send" => chan_send(args),
        "recv" => chan_recv(args),
        "try_recv" => chan_try_recv(args),
        "recv_timeout" => chan_recv_timeout(args),
        "close" => chan_close(args),
        "stat" => chan_stat(args),
        _ => Err(format!("Unknown chan function: {}", name).into()),
    }
}

fn handle(chan: Arc<Chan>) -> Value {
    Value::Native(NativeHandle::new(ChanObject(chan)))
}

fn err(msg: impl Into<String>) -> Value {
    Value::Err(Box::new(Value::String(Arc::new(msg.into()))))
}

fn ok(v: Value) -> Value {
    Value::Ok(Box::new(v))
}

/// Pull the shared `Arc<Chan>` back out of a channel handle.
fn chan_of(value: &Value) -> Result<Arc<Chan>, Box<dyn std::error::Error>> {
    match value {
        Value::Native(h) => match h.0.as_any().downcast_ref::<ChanObject>() {
            Some(obj) => Ok(obj.0.clone()),
            None => Err(format!(
                "chan: expected a channel (from chan.new), got a {} handle",
                h.0.type_name()
            )
            .into()),
        },
        other => Err(format!(
            "chan: expected a channel (from chan.new), got {}",
            other.type_name()
        )
        .into()),
    }
}

fn register(tx: Tx, rx: Receiver<Value>) -> Value {
    handle(Arc::new(Chan {
        id: CHAN_SEQ.fetch_add(1, Ordering::Relaxed),
        tx: Mutex::new(Some(tx)),
        rx: Mutex::new(rx),
        depth: AtomicI64::new(0),
        recv_waiting: AtomicUsize::new(0),
        send_waiting: AtomicUsize::new(0),
    }))
}

fn chan_new(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if !args.is_empty() {
        return Err("chan.new takes no arguments".into());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    Ok(register(Tx::Unbounded(tx), rx))
}

fn chan_bounded(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let cap = match args.as_slice() {
        [Value::Integer(n)] if *n >= 0 => *n as usize,
        _ => return Err("chan.bounded expects a non-negative Int capacity".into()),
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(cap);
    Ok(register(Tx::Bounded(tx), rx))
}

fn chan_send(mut args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 2 {
        return Err("chan.send expects a channel and a value".into());
    }
    let value = args.pop().expect("len checked");
    // A channel is the one thread crossing where the value being sent is in
    // hand, so the confinement rule is enforced here — at the send, where
    // the mistake is — instead of on the receiver's first access.
    if let Some(kind) = crate::stdlib::cell::confined_within(&value) {
        return Err(format!(
            "chan.send: a {} cannot be sent through a channel — it belongs to \
             the thread that created it. Send what it holds instead \
             (`chan.send(ch, cell.get(c))` for a cell; for a reader, send the \
             rows rather than the reader)",
            kind
        )
        .into());
    }
    let chan = chan_of(&args[0])?;
    // Clone the sender out of its lock: a bounded send may block until a
    // receiver drains, and holding the lock would stall chan.close.
    let tx = chan.tx.lock().unwrap().clone();
    let Some(tx) = tx else {
        return Ok(err("channel is closed"));
    };
    match tx {
        Tx::Unbounded(t) => Ok(match t.send(value) {
            Ok(()) => {
                chan.depth.fetch_add(1, Ordering::Relaxed);
                ok(Value::Unit)
            }
            Err(_) => err("channel is closed"),
        }),
        // A bounded send against a full queue is an unbounded wait: poll
        // under a park entry so the stall detector sees it (and wasm,
        // which has no second thread to drain anything, keeps the plain
        // blocking send it always had).
        #[cfg(target_arch = "wasm32")]
        Tx::Bounded(t) => Ok(match t.send(value) {
            Ok(()) => {
                chan.depth.fetch_add(1, Ordering::Relaxed);
                ok(Value::Unit)
            }
            Err(_) => err("channel is closed"),
        }),
        #[cfg(not(target_arch = "wasm32"))]
        Tx::Bounded(t) => {
            use std::sync::mpsc::TrySendError;
            let mut value = value;
            let mut token: Option<u64> = None;
            let mut last_gen: Option<u64> = None;
            let mut last_tick = std::time::Instant::now();
            let outcome = loop {
                match t.try_send(value) {
                    Ok(()) => {
                        chan.depth.fetch_add(1, Ordering::Relaxed);
                        break ok(Value::Unit);
                    }
                    Err(TrySendError::Disconnected(_)) => break err("channel is closed"),
                    Err(TrySendError::Full(v)) => {
                        value = v;
                        if token.is_none() {
                            token = Some(park(format!("chan.send on full channel #{}", chan.id)));
                            chan.send_waiting.fetch_add(1, Ordering::Relaxed);
                        }
                        if last_tick.elapsed().as_millis() as u64 >= STALL_TICK_MS {
                            last_tick = std::time::Instant::now();
                            stall_tick(&mut last_gen);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
            };
            if let Some(t) = token {
                unpark(t);
                chan.send_waiting.fetch_sub(1, Ordering::Relaxed);
            }
            Ok(outcome)
        }
    }
}

fn chan_recv(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err("chan.recv expects a channel".into());
    }
    let chan = chan_of(&args[0])?;
    // Park before taking the receiver lock: with several consumers the
    // wait happens on the lock as often as on the queue, and both are
    // unbounded — the stall detector must see either.
    #[cfg(not(target_arch = "wasm32"))]
    {
        let token = park(format!("chan.recv on channel #{}", chan.id));
        chan.recv_waiting.fetch_add(1, Ordering::Relaxed);
        let rx = chan.rx.lock().unwrap();
        let mut last_gen: Option<u64> = None;
        let outcome = loop {
            match rx.recv_timeout(std::time::Duration::from_millis(STALL_TICK_MS)) {
                Ok(v) => {
                    chan.depth.fetch_sub(1, Ordering::Relaxed);
                    break ok(v);
                }
                Err(RecvTimeoutError::Timeout) => stall_tick(&mut last_gen),
                Err(RecvTimeoutError::Disconnected) => break err("channel is closed"),
            }
        };
        unpark(token);
        chan.recv_waiting.fetch_sub(1, Ordering::Relaxed);
        Ok(outcome)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let rx = chan.rx.lock().unwrap();
        Ok(match rx.recv() {
            Ok(v) => ok(v),
            Err(_) => err("channel is closed"),
        })
    }
}

fn chan_try_recv(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err("chan.try_recv expects a channel".into());
    }
    let chan = chan_of(&args[0])?;
    let rx = chan.rx.lock().unwrap();
    Ok(match rx.try_recv() {
        Ok(v) => {
            chan.depth.fetch_sub(1, Ordering::Relaxed);
            ok(v)
        }
        Err(TryRecvError::Empty) => err("channel is empty"),
        Err(TryRecvError::Disconnected) => err("channel is closed"),
    })
}

fn chan_recv_timeout(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    let ms = match args.as_slice() {
        [_, Value::Integer(ms)] if *ms >= 0 => *ms as u64,
        _ => return Err("chan.recv_timeout expects a channel and a non-negative Int ms".into()),
    };
    let chan = chan_of(&args[0])?;
    let rx = chan.rx.lock().unwrap();
    Ok(
        match rx.recv_timeout(std::time::Duration::from_millis(ms)) {
            Ok(v) => {
                chan.depth.fetch_sub(1, Ordering::Relaxed);
                ok(v)
            }
            Err(RecvTimeoutError::Timeout) => err("timed out"),
            Err(RecvTimeoutError::Disconnected) => err("channel is closed"),
        },
    )
}

/// Advisory channel state for the REPL and the profiler: reads race
/// concurrent sends by design, so treat the numbers as a snapshot.
fn chan_stat(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err("chan.stat expects a channel".into());
    }
    let chan = chan_of(&args[0])?;
    let mut m = HashMap::new();
    m.insert("id".to_string(), Value::Integer(chan.id as i64));
    m.insert(
        "queued".to_string(),
        Value::Integer(chan.depth.load(Ordering::Relaxed).max(0)),
    );
    m.insert(
        "closed".to_string(),
        Value::Boolean(chan.tx.lock().unwrap().is_none()),
    );
    m.insert(
        "recv_waiting".to_string(),
        Value::Integer(chan.recv_waiting.load(Ordering::Relaxed) as i64),
    );
    m.insert(
        "send_waiting".to_string(),
        Value::Integer(chan.send_waiting.load(Ordering::Relaxed) as i64),
    );
    Ok(Value::Map(Arc::new(m)))
}

/// Close the sending side. Idempotent; queued messages still drain.
/// Is this value a channel?
pub(crate) fn is_channel(value: &Value) -> bool {
    chan_of(value).is_ok()
}

/// Close a channel value from outside the module — what a dying task
/// does to the channels it was watching (`task.watch`).
pub(crate) fn close_value(value: &Value) {
    if let Ok(chan) = chan_of(value) {
        *chan.tx.lock().unwrap() = None;
    }
}

fn chan_close(args: Vec<Value>) -> Result<Value, Box<dyn std::error::Error>> {
    if args.len() != 1 {
        return Err("chan.close expects a channel".into());
    }
    let chan = chan_of(&args[0])?;
    *chan.tx.lock().unwrap() = None;
    Ok(Value::Unit)
}

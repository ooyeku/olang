//! `fs.watch`: a folder's whole tree watched for changes, as an editor's
//! file list follows a checkout changed outside it (a file made in the
//! Finder, a `git checkout`, a build's output).
//!
//!   let w = unwrap(fs.watch(root, #{ "ignore": [".git", "target"], "keep": [".git/HEAD", ".git/index"] }))
//!   match fs.watch_next(w, #{ "timeout_ms": 500 }) {
//!       Ok(b) => if b != () => for e in b.events { println(e.kind + " " + e.path) },
//!       Err(e) => …                               // closed
//!   }
//!   fs.watch_close(w)
//!
//! On macOS the operating system's FSEvents does the watching: one stream
//! for the whole tree, its own thread running the stream's run loop,
//! nothing polled. What it reports is filtered as it arrives (a path
//! under an ignored folder never leaves the callback, so a `cargo build`
//! writing thousands of files into `target/` costs a string compare
//! each) and queued; `watch_next` waits for the first change, then keeps
//! taking more while they come within `settle_ms`, so a burst is one
//! batch: each path once, said as `created`, `removed` or `modified`
//! (what the disk says when the batch is made — a rename is its old path
//! removed and its new one created), with `dir` when it is a folder.
//! When the system dropped events, or a batch holds more paths than
//! `max`, the folder to read again is in `rescan` instead.
//!
//! Handles are `Watch { id }` structs into a registry, as `pty`'s are, so
//! a reader task may hold one. Elsewhere than macOS `fs.watch` answers an
//! `Err` (a caller keeps polling).

use crate::ast::{Value, ValueMap};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

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
fn vmap(fields: Vec<(&str, Value)>) -> Value {
    let mut m = ValueMap::default();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Map(Arc::new(m))
}

fn fields_of(v: Option<&Value>, who: &str) -> Result<ValueMap, Box<dyn std::error::Error>> {
    match v {
        None | Some(Value::Unit) => Ok(ValueMap::default()),
        Some(Value::Map(m)) => Ok(m.as_ref().clone()),
        Some(Value::Struct { fields, .. }) => Ok(fields.as_ref().clone()),
        Some(other) => {
            Err(format!("{}: options must be a map, got {}", who, other.type_name()).into())
        }
    }
}

fn int_opt(
    m: &ValueMap,
    k: &str,
    who: &str,
    default: i64,
) -> Result<i64, Box<dyn std::error::Error>> {
    match m.get(k) {
        None | Some(Value::Unit) => Ok(default),
        Some(Value::Integer(i)) if *i >= 0 => Ok(*i),
        Some(Value::Float(f)) if *f >= 0.0 => Ok(*f as i64),
        Some(other) => Err(format!(
            "{}: {} must be a non-negative number, got {}",
            who,
            k,
            other.type_name()
        )
        .into()),
    }
}

fn strings_opt(
    m: &ValueMap,
    k: &str,
    who: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    match m.get(k) {
        None | Some(Value::Unit) => Ok(Vec::new()),
        Some(Value::List(xs)) => xs
            .iter()
            .map(|x| match x {
                Value::String(t) => Ok(t.as_ref().clone()),
                other => Err(format!(
                    "{}: {} must be a list of strings, got a {}",
                    who,
                    k,
                    other.type_name()
                )
                .into()),
            })
            .collect(),
        Some(other) => Err(format!(
            "{}: {} must be a list of strings, got {}",
            who,
            k,
            other.type_name()
        )
        .into()),
    }
}

// ── the filter ──────────────────────────────────────────────────────
/// What a watch passes on: folder names never reported (at any depth),
/// and paths under them reported all the same (`.git/HEAD` under an
/// ignored `.git`).
#[derive(Clone, Default)]
struct Filter {
    ignore: Vec<String>,
    keep: Vec<String>,
}

impl Filter {
    /// Whether `rel` (relative to the root, `/`-separated) is passed on.
    fn passes(&self, rel: &str) -> bool {
        if self.ignore.is_empty() || rel.is_empty() {
            return true;
        }
        let mut at = 0usize;
        for seg in rel.split('/') {
            if self.ignore.iter().any(|i| i == seg) {
                let rest = &rel[at..];
                return self.keep.iter().any(|k| {
                    rest == k
                        || (rest.starts_with(k.as_str())
                            && rest.as_bytes().get(k.len()) == Some(&b'/'))
                });
            }
            at += seg.len() + 1;
        }
        true
    }
}

// ── what the stream queues ──────────────────────────────────────────
/// One path the system reported, with what it said of it.
struct Raw {
    path: String,
    created: bool,
    renamed: bool,
    is_dir: bool,
    /// The system lost track below this path: read it again.
    rescan: bool,
}

/// The queue's room: past it, the root is read again instead.
const QUEUE_MAX: usize = 65_536;

struct Watch {
    root: String,
    rx: crossbeam_channel::Receiver<Raw>,
    /// More arrived than the queue holds: the whole root to read again.
    overflow: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    closed: AtomicBool,
}

fn watches() -> &'static Mutex<HashMap<i64, Arc<Watch>>> {
    static W: OnceLock<Mutex<HashMap<i64, Arc<Watch>>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ID: AtomicI64 = AtomicI64::new(1);

fn handle(id: i64) -> Value {
    let mut fields = ValueMap::default();
    fields.insert("id".to_string(), Value::Integer(id));
    Value::Struct {
        type_name: "Watch".to_string(),
        fields: Arc::new(fields),
    }
}

fn watch_of(
    v: Option<&Value>,
    who: &str,
) -> Result<Option<Arc<Watch>>, Box<dyn std::error::Error>> {
    let id = match v {
        Some(Value::Struct { type_name, fields }) if type_name == "Watch" => match fields.get("id")
        {
            Some(Value::Integer(i)) => *i,
            _ => return Err(format!("{}: a malformed Watch handle", who).into()),
        },
        Some(other) => {
            return Err(format!(
                "{}: expected a Watch handle, got {}",
                who,
                other.type_name()
            )
            .into());
        }
        None => return Err(format!("{}: expected a Watch handle", who).into()),
    };
    Ok(watches().lock().unwrap().get(&id).cloned())
}

/// `rel` of `path` under `root` (both without a trailing slash), or None
/// when it is not under it; `Some("")` for the root itself.
fn rel_under<'a>(root: &str, path: &'a str) -> Option<&'a str> {
    if path == root {
        return Some("");
    }
    let r = if root == "/" { "" } else { root };
    path.strip_prefix(r).and_then(|rest| rest.strip_prefix('/'))
}

// ── the builtins ────────────────────────────────────────────────────
/// `fs.watch(root, opts)`: `Ok(Watch)` watching `root`'s whole tree.
pub fn watch(args: Vec<Value>) -> Res {
    if args.is_empty() || args.len() > 2 {
        return Err(crate::stdlib::misuse::arity(
            "fs.watch",
            "1 or 2",
            args.len(),
        ));
    }
    let root_in = match &args[0] {
        Value::String(t) => t.as_ref().clone(),
        other => {
            return Err(format!(
                "fs.watch: the folder must be a string, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let o = fields_of(args.get(1), "fs.watch")?;
    for k in o.keys() {
        if !matches!(k.as_str(), "ignore" | "keep" | "latency_ms") {
            return Err(format!("fs.watch: no option {:?} (ignore, keep, latency_ms)", k).into());
        }
    }
    let filter = Filter {
        ignore: strings_opt(&o, "ignore", "fs.watch")?,
        keep: strings_opt(&o, "keep", "fs.watch")?
            .into_iter()
            .map(|k| k.trim_end_matches('/').to_string())
            .collect(),
    };
    let latency_ms = int_opt(&o, "latency_ms", "fs.watch", 50)?;
    let root = {
        let t = root_in.trim_end_matches('/');
        let t = if t.is_empty() { "/" } else { t };
        match std::path::Path::new(t).canonicalize() {
            // as the caller spelled it (absolute), so the paths said are theirs
            Ok(_) if t.starts_with('/') => t.to_string(),
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(e) => return Ok(err(format!("fs.watch: {}: {}", root_in, e))),
        }
    };
    if !std::path::Path::new(&root).is_dir() {
        return Ok(err(format!("fs.watch: {}: not a folder", root_in)));
    }
    match imp::start(&root, filter, latency_ms) {
        Ok(w) => {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            watches().lock().unwrap().insert(id, Arc::new(w));
            Ok(ok(handle(id)))
        }
        Err(e) => Ok(err(format!("fs.watch: {}", e))),
    }
}

/// `fs.watch_next(w, opts)`: the next batch of changes, `()` when none
/// came within `timeout_ms`, `Err` once the watch is closed.
pub fn watch_next(args: Vec<Value>) -> Res {
    if args.is_empty() || args.len() > 2 {
        return Err(crate::stdlib::misuse::arity(
            "fs.watch_next",
            "1 or 2",
            args.len(),
        ));
    }
    let w = match watch_of(args.first(), "fs.watch_next")? {
        Some(w) => w,
        None => return Ok(err("fs.watch_next: the watch is closed")),
    };
    let o = fields_of(args.get(1), "fs.watch_next")?;
    for k in o.keys() {
        if !matches!(k.as_str(), "timeout_ms" | "settle_ms" | "max") {
            return Err(format!(
                "fs.watch_next: no option {:?} (timeout_ms, settle_ms, max)",
                k
            )
            .into());
        }
    }
    let timeout = Duration::from_millis(int_opt(&o, "timeout_ms", "fs.watch_next", 1000)? as u64);
    let settle = Duration::from_millis(int_opt(&o, "settle_ms", "fs.watch_next", 50)? as u64);
    let max = int_opt(&o, "max", "fs.watch_next", 5000)?.max(1) as usize;
    // the first change, or nothing
    let first = match w.rx.recv_timeout(timeout) {
        Ok(r) => r,
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
            if w.overflow.swap(false, Ordering::AcqRel) {
                return Ok(ok(batch(&w.root, Vec::new(), vec![w.root.clone()])));
            }
            return Ok(if w.closed.load(Ordering::Acquire) {
                err("fs.watch_next: the watch is closed")
            } else {
                ok(Value::Unit)
            });
        }
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            return Ok(err("fs.watch_next: the watch is closed"));
        }
    };
    // then the rest of the burst: until it is quiet for `settle`, and
    // never more than a second after the first (a tree written to
    // without a pause is still said)
    let began = Instant::now();
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashMap<String, (bool, bool, bool)> = HashMap::new();
    let mut rescan: Vec<String> = Vec::new();
    let mut too_many = false;
    let mut take = |r: Raw,
                    order: &mut Vec<String>,
                    seen: &mut HashMap<String, (bool, bool, bool)>,
                    rescan: &mut Vec<String>| {
        if r.rescan {
            if !rescan.contains(&r.path) {
                rescan.push(r.path);
            }
            return;
        }
        if too_many {
            return;
        }
        match seen.get_mut(&r.path) {
            Some(e) => {
                e.0 |= r.created;
                e.1 |= r.renamed;
                e.2 |= r.is_dir;
            }
            None => {
                if order.len() >= max {
                    too_many = true;
                    return;
                }
                seen.insert(r.path.clone(), (r.created, r.renamed, r.is_dir));
                order.push(r.path);
            }
        }
    };
    take(first, &mut order, &mut seen, &mut rescan);
    let cap = Duration::from_millis(1000).max(settle);
    loop {
        let left = cap.saturating_sub(began.elapsed());
        if left.is_zero() {
            break;
        }
        match w.rx.recv_timeout(settle.min(left)) {
            Ok(r) => take(r, &mut order, &mut seen, &mut rescan),
            Err(_) => break,
        }
    }
    if w.overflow.swap(false, Ordering::AcqRel) || too_many {
        rescan = vec![w.root.clone()];
        order.clear();
    }
    let events = order
        .into_iter()
        .map(|p| {
            let (created, renamed, was_dir) = seen[&p];
            let meta = std::fs::symlink_metadata(&p);
            let (kind, dir) = match &meta {
                Err(_) => ("removed", was_dir),
                Ok(m) => (
                    if created || renamed {
                        "created"
                    } else {
                        "modified"
                    },
                    m.is_dir(),
                ),
            };
            vmap(vec![
                ("path", s(&p)),
                ("kind", s(kind)),
                ("dir", Value::Boolean(dir)),
            ])
        })
        .collect();
    Ok(ok(batch(&w.root, events, rescan)))
}

fn batch(root: &str, events: Vec<Value>, rescan: Vec<String>) -> Value {
    vmap(vec![
        ("root", s(root)),
        ("events", Value::List(Arc::new(events))),
        (
            "rescan",
            Value::List(Arc::new(rescan.iter().map(|p| s(p)).collect())),
        ),
    ])
}

/// `fs.watch_close(w)`: the stream stopped; `Ok(true)`, or `Ok(false)` for
/// one closed already.
pub fn watch_close(args: Vec<Value>) -> Res {
    if args.len() != 1 {
        return Err(crate::stdlib::misuse::arity(
            "fs.watch_close",
            "1",
            args.len(),
        ));
    }
    let id = match &args[0] {
        Value::Struct { type_name, fields } if type_name == "Watch" => match fields.get("id") {
            Some(Value::Integer(i)) => *i,
            _ => return Err("fs.watch_close: a malformed Watch handle".into()),
        },
        other => {
            return Err(format!(
                "fs.watch_close: expected a Watch handle, got {}",
                other.type_name()
            )
            .into());
        }
    };
    let w = watches().lock().unwrap().remove(&id);
    Ok(ok(Value::Boolean(match w {
        Some(w) => {
            w.closed.store(true, Ordering::Release);
            w.stop.store(true, Ordering::Release);
            true
        }
        None => false,
    })))
}

// ── macOS: FSEvents ─────────────────────────────────────────────────
#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::{CStr, CString, c_char, c_void};

    type CFRef = *const c_void;
    type FSEventStreamRef = *mut c_void;

    #[repr(C)]
    struct FSEventStreamContext {
        version: isize,
        info: *mut c_void,
        retain: *const c_void,
        release: *const c_void,
        copy_description: *const c_void,
    }

    type Callback =
        extern "C" fn(FSEventStreamRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeArrayCallBacks: c_void;
        static kCFRunLoopDefaultMode: CFRef;
        fn CFStringCreateWithCString(alloc: CFRef, s: *const c_char, encoding: u32) -> CFRef;
        fn CFArrayCreate(
            alloc: CFRef,
            values: *const CFRef,
            n: isize,
            callbacks: *const c_void,
        ) -> CFRef;
        fn CFRelease(r: CFRef);
        fn CFRunLoopGetCurrent() -> CFRef;
        fn CFRunLoopRunInMode(mode: CFRef, seconds: f64, return_after_source: u8) -> i32;
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn FSEventStreamCreate(
            alloc: CFRef,
            cb: Callback,
            ctx: *const FSEventStreamContext,
            paths: CFRef,
            since: u64,
            latency: f64,
            flags: u32,
        ) -> FSEventStreamRef;
        fn FSEventStreamScheduleWithRunLoop(s: FSEventStreamRef, rl: CFRef, mode: CFRef);
        fn FSEventStreamStart(s: FSEventStreamRef) -> u8;
        fn FSEventStreamStop(s: FSEventStreamRef);
        fn FSEventStreamInvalidate(s: FSEventStreamRef);
        fn FSEventStreamRelease(s: FSEventStreamRef);
    }

    const UTF8: u32 = 0x0800_0100;
    const SINCE_NOW: u64 = u64::MAX;
    const CREATE_WATCH_ROOT: u32 = 0x04;
    const CREATE_FILE_EVENTS: u32 = 0x10;

    const MUST_SCAN_SUBDIRS: u32 = 0x01;
    const USER_DROPPED: u32 = 0x02;
    const KERNEL_DROPPED: u32 = 0x04;
    const ROOT_CHANGED: u32 = 0x20;
    const ITEM_CREATED: u32 = 0x100;
    const ITEM_RENAMED: u32 = 0x800;
    const ITEM_IS_DIR: u32 = 0x20000;

    /// What the callback reads: owned by the stream's thread, freed after
    /// the stream is released.
    struct Ctx {
        /// The root as the system spells it (`/private/var/…` for `/var/…`).
        real: String,
        /// The root as the caller spelled it.
        given: String,
        filter: Filter,
        tx: crossbeam_channel::Sender<Raw>,
        overflow: Arc<AtomicBool>,
    }

    impl Ctx {
        fn push(&self, r: Raw) {
            if self.tx.len() >= QUEUE_MAX {
                self.overflow.store(true, Ordering::Release);
            } else {
                let _ = self.tx.send(r);
            }
        }
    }

    extern "C" fn callback(
        _s: FSEventStreamRef,
        info: *mut c_void,
        n: usize,
        paths: *mut c_void,
        flags: *const u32,
        _ids: *const u64,
    ) {
        // SAFETY: info is the Ctx the stream was made with, alive until the
        // stream is released; paths and flags are n long (no CF types asked).
        let ctx = unsafe { &*(info as *const Ctx) };
        let paths = paths as *const *const c_char;
        for i in 0..n {
            let (p, f) = unsafe {
                (
                    CStr::from_ptr(*paths.add(i)).to_string_lossy().into_owned(),
                    *flags.add(i),
                )
            };
            let p = p.trim_end_matches('/');
            let p = if p.is_empty() { "/" } else { p };
            // the caller's spelling of the path
            let rel = match rel_under(&ctx.real, p).or_else(|| rel_under(&ctx.given, p)) {
                Some(r) => r.to_string(),
                None => {
                    // the root itself moved or went (WatchRoot): read it again
                    if f & ROOT_CHANGED != 0 {
                        ctx.push(Raw {
                            path: ctx.given.clone(),
                            created: false,
                            renamed: false,
                            is_dir: true,
                            rescan: true,
                        });
                    }
                    continue;
                }
            };
            if !ctx.filter.passes(&rel) {
                continue;
            }
            let path = if rel.is_empty() {
                ctx.given.clone()
            } else if ctx.given == "/" {
                format!("/{}", rel)
            } else {
                format!("{}/{}", ctx.given, rel)
            };
            let lost = f & (MUST_SCAN_SUBDIRS | USER_DROPPED | KERNEL_DROPPED | ROOT_CHANGED) != 0;
            if lost {
                let at = if f & (USER_DROPPED | KERNEL_DROPPED) != 0 {
                    ctx.given.clone()
                } else {
                    path.clone()
                };
                ctx.push(Raw {
                    path: at,
                    created: false,
                    renamed: false,
                    is_dir: true,
                    rescan: true,
                });
                continue;
            }
            if rel.is_empty() {
                // the root's own metadata: nothing below it changed by that
                continue;
            }
            ctx.push(Raw {
                path,
                created: f & ITEM_CREATED != 0,
                renamed: f & ITEM_RENAMED != 0,
                is_dir: f & ITEM_IS_DIR != 0,
                rescan: false,
            });
        }
    }

    struct SendPtr(*mut Ctx);
    unsafe impl Send for SendPtr {}

    pub(super) fn start(root: &str, filter: Filter, latency_ms: i64) -> Result<Watch, String> {
        let real = std::fs::canonicalize(root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| root.to_string());
        let (tx, rx) = crossbeam_channel::unbounded();
        let overflow = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let ctx = Box::into_raw(Box::new(Ctx {
            real: real.clone(),
            given: root.to_string(),
            filter,
            tx,
            overflow: overflow.clone(),
        }));
        let ctx_send = SendPtr(ctx);
        let stop2 = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let latency = (latency_ms as f64 / 1000.0).max(0.0);
        let spawned = std::thread::Builder::new()
            .name("fs.watch".into())
            .spawn(move || {
                let ctx_ptr = ctx_send;
                let c = CString::new(real).unwrap_or_default();
                // SAFETY: plain CoreFoundation and FSEvents calls; every object
                // made here is released here, and the context outlives the stream.
                unsafe {
                    let path = CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), UTF8);
                    let arr = CFArrayCreate(
                        std::ptr::null(),
                        &path,
                        1,
                        &kCFTypeArrayCallBacks as *const c_void,
                    );
                    let context = FSEventStreamContext {
                        version: 0,
                        info: ctx_ptr.0 as *mut c_void,
                        retain: std::ptr::null(),
                        release: std::ptr::null(),
                        copy_description: std::ptr::null(),
                    };
                    let stream = FSEventStreamCreate(
                        std::ptr::null(),
                        callback,
                        &context,
                        arr,
                        SINCE_NOW,
                        latency,
                        CREATE_WATCH_ROOT | CREATE_FILE_EVENTS,
                    );
                    CFRelease(arr);
                    CFRelease(path);
                    if stream.is_null() {
                        let _ = ready_tx.send(Err("FSEvents refused the stream".into()));
                        drop(Box::from_raw(ctx_ptr.0));
                        return;
                    }
                    FSEventStreamScheduleWithRunLoop(
                        stream,
                        CFRunLoopGetCurrent(),
                        kCFRunLoopDefaultMode,
                    );
                    if FSEventStreamStart(stream) == 0 {
                        FSEventStreamInvalidate(stream);
                        FSEventStreamRelease(stream);
                        let _ = ready_tx.send(Err("FSEvents could not start the stream".into()));
                        drop(Box::from_raw(ctx_ptr.0));
                        return;
                    }
                    let _ = ready_tx.send(Ok(()));
                    while !stop2.load(Ordering::Acquire) {
                        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.25, 0);
                    }
                    FSEventStreamStop(stream);
                    FSEventStreamInvalidate(stream);
                    FSEventStreamRelease(stream);
                    // the sender goes with it: a reader waiting hears the end
                    drop(Box::from_raw(ctx_ptr.0));
                }
            });
        if let Err(e) = spawned {
            // SAFETY: the thread never ran, so the context is still ours.
            unsafe { drop(Box::from_raw(ctx)) };
            return Err(e.to_string());
        }
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Watch {
                root: root.to_string(),
                rx,
                overflow,
                stop,
                closed: AtomicBool::new(false),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("the watch's thread ended".into()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    pub(super) fn start(_root: &str, _filter: Filter, _latency_ms: i64) -> Result<Watch, String> {
        Err("watching a folder needs macOS (FSEvents) for now".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_drops_ignored_folders_and_keeps_what_it_names() {
        let f = Filter {
            ignore: vec![".git".into(), "target".into()],
            keep: vec![".git/HEAD".into(), ".git/refs".into()],
        };
        assert!(f.passes("src/main.ol"));
        assert!(!f.passes("target/debug/x"));
        assert!(!f.passes("a/target/x"));
        assert!(!f.passes(".git/objects/ab/cd"));
        assert!(f.passes(".git/HEAD"));
        assert!(f.passes("sub/.git/HEAD"));
        assert!(f.passes(".git/refs/heads/main"));
        assert!(!f.passes(".git/refsx"));
        assert!(f.passes("targets/x"));
    }

    #[test]
    fn rel_under_its_root() {
        assert_eq!(rel_under("/a/b", "/a/b/c"), Some("c"));
        assert_eq!(rel_under("/a/b", "/a/b"), Some(""));
        assert_eq!(rel_under("/a/b", "/a/bc"), None);
        assert_eq!(rel_under("/", "/x"), Some("x"));
    }
}

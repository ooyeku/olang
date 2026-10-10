//! Where a program's start goes: `OLANG_BOOT_TRACE=1`.
//!
//! A program on a UI framework spends its first few hundred milliseconds
//! reading modules, evaluating their declarations and compiling what its
//! first frame runs; nothing else attributes that time. With the switch
//! set, olang writes a line to stderr at each landmark — the interpreter
//! thread started, the entry file parsed, a window opened, the first frame
//! presented — each stamped with the milliseconds since the process
//! started (the kernel's clock of it, so the binary's own loading counts),
//! and at the first frame (or at exit, for a program with no window) a
//! summary: modules read (parsed or from the
//! parse cache) and evaluated, functions compiled to bytecode and to
//! native code, and how long each took.
//!
//! Off, each hook is one relaxed load of a flag decided once a process.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn flag() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("OLANG_BOOT_TRACE").is_some_and(|v| !v.is_empty() && v != "0")
    })
}

/// Whether the trace is on (decided once a process).
#[inline]
pub fn on() -> bool {
    flag()
}

/// The process's start on the wall clock, in microseconds since the
/// epoch, as the kernel recorded it (`None` where it cannot be read).
#[cfg(target_os = "macos")]
pub fn process_start_epoch_us() -> Option<u64> {
    // SAFETY: proc_pidinfo fills a plain C struct of the size given.
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let n = libc::proc_pidinfo(
            std::process::id() as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if n != size {
            return None;
        }
        Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn process_start_epoch_us() -> Option<u64> {
    None
}

/// How long this process has run, in milliseconds: from the kernel's
/// record of its start where it can be read, else from this function's
/// first use.
#[cfg(feature = "native")]
pub fn since_start_ms() -> f64 {
    static FALLBACK: OnceLock<std::time::Instant> = OnceLock::new();
    let fallback = *FALLBACK.get_or_init(std::time::Instant::now);
    if let Some(start) = process_start_epoch_us() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        if now >= start {
            return (now - start) as f64 / 1000.0;
        }
    }
    fallback.elapsed().as_secs_f64() * 1000.0
}

#[cfg(not(feature = "native"))]
pub fn since_start_ms() -> f64 {
    0.0
}

/// A landmark: one line, stamped.
pub fn mark(what: &str) {
    if on() {
        eprintln!("olang boot: {:7.1} ms  {}", since_start_ms(), what);
    }
}

/// The counters a summary reports. Times are nanoseconds.
pub enum Counter {
    /// A module's text parsed (it was not in the parse cache).
    Parsed,
    /// A module's tree read back from the parse cache.
    CacheRead,
    /// A module's tree written to the parse cache.
    CacheStored,
    /// An embedded package's module (its tree kept by the binary).
    Embedded,
    /// Loading modules, outermost only: read, parse and evaluate,
    /// nested imports included.
    ModuleLoad,
    /// A function compiled to bytecode (one attempt each).
    Compile,
    /// A group of functions compiled to native code.
    Native,
    /// A function's bytecode taken from the compile cache (its time:
    /// reading, checking and rebuilding the entry).
    CompileCacheHit,
    /// A compile cache entry whose recorded questions this run answered
    /// differently: compiled afresh.
    CompileCacheStale,
    /// A compile kept in the compile cache.
    CompileCacheStored,
}

const N: usize = 10;
static COUNTS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
static NANOS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

/// Count one event of `what` that took `nanos`.
#[inline]
pub fn add(what: Counter, nanos: u64) {
    if on() {
        let i = what as usize;
        COUNTS[i].fetch_add(1, Ordering::Relaxed);
        NANOS[i].fetch_add(nanos, Ordering::Relaxed);
    }
}

fn read(what: Counter) -> (u64, f64) {
    let i = what as usize;
    (
        COUNTS[i].load(Ordering::Relaxed),
        NANOS[i].load(Ordering::Relaxed) as f64 / 1e6,
    )
}

/// The summary line so far.
pub fn summary() -> String {
    let (parsed, parse_ms) = read(Counter::Parsed);
    let (cached, cache_ms) = read(Counter::CacheRead);
    let (stored, store_ms) = read(Counter::CacheStored);
    let (embedded, _) = read(Counter::Embedded);
    let (_, load_ms) = read(Counter::ModuleLoad);
    let (compiled, compile_ms) = read(Counter::Compile);
    let (native, native_ms) = read(Counter::Native);
    let (hits, hit_ms) = read(Counter::CompileCacheHit);
    let (stale, _) = read(Counter::CompileCacheStale);
    let (kept, _) = read(Counter::CompileCacheStored);
    format!(
        "modules {} in {:.1} ms (parsed {} in {:.1} ms, from the cache {} in {:.1} ms, cached {} in {:.1} ms, embedded {}); \
         bytecode {} compiles in {:.1} ms (from the cache {} in {:.1} ms, stale {}, kept {}); native {} groups in {:.1} ms",
        parsed + cached + embedded,
        load_ms,
        parsed,
        parse_ms,
        cached,
        cache_ms,
        stored,
        store_ms,
        embedded,
        compiled,
        compile_ms,
        hits,
        hit_ms,
        stale,
        kept,
        native,
        native_ms
    )
}

static FIRST_FRAME: AtomicBool = AtomicBool::new(false);

/// A window presented a frame: the first one is a landmark, with the
/// summary.
pub fn frame_presented() {
    if on() && !FIRST_FRAME.swap(true, Ordering::Relaxed) {
        let at = since_start_ms();
        eprintln!("olang boot: {:7.1} ms  first frame presented", at);
        eprintln!("olang boot:            {}", summary());
    }
}

/// The process is ending: a program that never presented a frame (one
/// with no window) has its summary here instead.
pub fn finished() {
    if on() && !FIRST_FRAME.swap(true, Ordering::Relaxed) {
        let at = since_start_ms();
        eprintln!("olang boot: {:7.1} ms  exit", at);
        eprintln!("olang boot:            {}", summary());
    }
}

/// Times a span into a counter when dropped (nothing when the trace is
/// off).
pub struct Span {
    what: Option<(Counter, std::time::Instant)>,
}

impl Span {
    #[inline]
    pub fn start(what: Counter) -> Span {
        Span {
            what: if on() {
                Some((what, std::time::Instant::now()))
            } else {
                None
            },
        }
    }

    /// Not counted after all.
    pub fn cancel(mut self) {
        self.what = None;
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some((what, t0)) = self.what.take() {
            add(what, t0.elapsed().as_nanos() as u64);
        }
    }
}

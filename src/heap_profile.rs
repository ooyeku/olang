//! The sampled heap profile behind `runtime.heap()` and `olang memory`:
//! what holds live memory, by the site that allocated it, cheap enough to
//! leave on in a program that runs all day (OL-610, OL-203, OL-324).
//!
//! `runtime.memory()` says how much the heap holds and whose it is
//! (program, code, values); this says *where it was allocated*. Two
//! snapshots taken hours apart, diffed, name the sites that grew.
//!
//! **Sampling.** Every allocation is a draw: the counting allocator
//! (src/memory.rs) takes its size off a per-thread countdown, and the
//! allocation that takes it past zero is sampled, the next countdown
//! drawn from an exponential distribution whose mean is the rate (one
//! sample per 512 KiB allocated by default). An allocation of `s` bytes
//! is sampled with probability `1 - exp(-s/rate)`, so it stands for
//! `s / p` bytes and `1 / p` allocations: the estimates are unbiased at
//! any size, and a large block is all but always seen. This is the
//! sampling tcmalloc and Go's heap profile use.
//!
//! **A sample** records its site: the innermost olang frames on the
//! thread's shadow stack (the profiler's, src/profile.rs, which records
//! frames while this runs), and what the allocation was part of —
//! loading the program, the runtime's code, or the program's values (the
//! same split as `runtime.memory()`). Sites are interned once each; per
//! site the live bytes and allocations and the bytes and allocations made
//! ever are atomic counters.
//!
//! **A free** must learn whether its block was sampled. Sampled blocks
//! live in a fixed table of 16 Ki buckets of four pointers, one cache
//! line each, hashed by address: a free reads its one bucket, and only a
//! sampled block's free does more (a compare-and-swap, a few atomic
//! subtractions). No lock is taken on a free, ever. A bucket found full
//! drops the sample (`dropped` counts them; at the default rate a full
//! table is a heap of tens of GB). With no sample live, a free pays one
//! relaxed load.
//!
//! **Reentrancy.** Interning a site allocates; a thread-local flag keeps
//! the profile's own allocations unsampled, so the allocator never
//! re-enters it. Everything the allocator path reads is a const
//! thread-local or a static: nothing here is made lazily inside it.
//!
//! **Tasks** (OL-609): a sample is charged to the task that made it (its
//! slot and the slot's generation), and taken off again wherever it is
//! freed, so `runtime.memory().tasks[i].held` is what that task's
//! allocations still hold, which the net figure cannot say of a thread
//! whose results other threads free.

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};

/// One sample per this many bytes allocated, on average.
pub const DEFAULT_RATE: u64 = 512 * 1024;
/// `olang profile --format json`'s rate: a short run needs more samples
/// to say what each function allocates.
pub const PROFILE_RATE: u64 = 64 * 1024;
const WAYS: usize = 4;
const BUCKET_BITS: u32 = 14;
const BUCKETS: usize = 1 << BUCKET_BITS;
const ENTRIES: usize = BUCKETS * WAYS;
/// A bucket entry being written or taken: never a real block's address.
const RESERVED: usize = 1;
const MAX_SITES: usize = 1 << 14;
/// Frames a site keeps, innermost first.
pub const DEPTH: usize = 4;
/// A count weight's fixed point (an allocation is `1 << 16`).
const ONE: f64 = 65536.0;

static ON: AtomicBool = AtomicBool::new(false);
static RATE: AtomicU64 = AtomicU64::new(DEFAULT_RATE);
/// Sampled blocks in the table now.
static SAMPLES: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
/// When sampling began (unix ms), 0 when it never did.
static STARTED_MS: AtomicU64 = AtomicU64::new(0);

#[repr(C, align(32))]
struct Bucket {
    keys: [AtomicUsize; WAYS],
}

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY_BUCKET: Bucket = Bucket {
    keys: [const { AtomicUsize::new(0) }; WAYS],
};
static KEYS: [Bucket; BUCKETS] = [EMPTY_BUCKET; BUCKETS];
/// An entry's site (bits 40..), slot (32..40) and the slot's generation.
static META: [AtomicU64; ENTRIES] = [const { AtomicU64::new(0) }; ENTRIES];
static WEIGHT_B: [AtomicU64; ENTRIES] = [const { AtomicU64::new(0) }; ENTRIES];
static WEIGHT_N: [AtomicU64; ENTRIES] = [const { AtomicU64::new(0) }; ENTRIES];

struct SiteStats {
    live_b: AtomicI64,
    live_n: AtomicI64,
    alloc_b: AtomicU64,
    alloc_n: AtomicU64,
}

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY_SITE: SiteStats = SiteStats {
    live_b: AtomicI64::new(0),
    live_n: AtomicI64::new(0),
    alloc_b: AtomicU64::new(0),
    alloc_n: AtomicU64::new(0),
};
static SITE_STATS: [SiteStats; MAX_SITES] = [EMPTY_SITE; MAX_SITES];
/// What each task slot's live samples hold (named slots only).
static HELD: [AtomicI64; crate::memory::SLOTS] =
    [const { AtomicI64::new(0) }; crate::memory::SLOTS];

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SiteKey {
    frames: [u32; DEPTH],
    depth: u8,
    scope: u8,
}

/// Site 0 is where samples go once `MAX_SITES` are taken.
struct Sites {
    index: rustc_hash::FxHashMap<SiteKey, u32>,
    keys: Vec<SiteKey>,
}

static SITES: Mutex<Option<Sites>> = Mutex::new(None);

thread_local! {
    /// Bytes until this thread's next sample.
    static COUNTDOWN: Cell<i64> = const { Cell::new(0) };
    /// The thread's random state; 0 until its first draw.
    static RNG: Cell<u64> = const { Cell::new(0) };
    /// Inside the profile's own code: its allocations are not sampled.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// Whether new allocations are being sampled. One relaxed load: the
/// allocator checks it on every allocation.
#[inline(always)]
pub fn sampling() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Whether any sampled block is live (a free must then look it up).
#[inline(always)]
pub fn has_samples() -> bool {
    SAMPLES.load(Ordering::Relaxed) != 0
}

/// The allocator's hook for a block just allocated. Out of line: the
/// allocator is inlined into every allocation in the binary (LTO), and a
/// hook inlined with it grew the code by 4 MB and slowed the interpreter
/// by 6% with the profile off.
#[inline(never)]
pub fn on_alloc(p: *mut u8, size: usize) {
    if p.is_null() {
        return;
    }
    let due = COUNTDOWN
        .try_with(|c| {
            let n = c.get() - size as i64;
            c.set(n);
            n <= 0
        })
        .unwrap_or(false);
    if due {
        sample(p, size);
    }
}

#[inline]
fn bucket_of(p: usize) -> usize {
    ((p >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - BUCKET_BITS)) as usize
}

/// The allocator's hook for a block about to be freed (or moved by a
/// realloc). Out of line, as `on_alloc`.
#[inline(never)]
pub fn on_free(p: *mut u8) {
    let p = p as usize;
    let b = bucket_of(p);
    for (w, key) in KEYS[b].keys.iter().enumerate() {
        if key.load(Ordering::Acquire) == p {
            take(b * WAYS + w, p);
            return;
        }
    }
}

#[cold]
#[inline(never)]
fn take(e: usize, p: usize) {
    let key = &KEYS[e / WAYS].keys[e % WAYS];
    if key
        .compare_exchange(p, RESERVED, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let meta = META[e].load(Ordering::Relaxed);
    let wb = WEIGHT_B[e].load(Ordering::Relaxed);
    let wn = WEIGHT_N[e].load(Ordering::Relaxed);
    key.store(0, Ordering::Release);
    SAMPLES.fetch_sub(1, Ordering::Relaxed);
    let site = (meta >> 40) as usize;
    let slot = ((meta >> 32) & 0xff) as usize;
    let generation = meta as u32;
    if let Some(s) = SITE_STATS.get(site) {
        s.live_b.fetch_sub(wb as i64, Ordering::Relaxed);
        s.live_n.fetch_sub(wn as i64, Ordering::Relaxed);
    }
    if crate::memory::slot_is(slot, generation) {
        HELD[slot].fetch_sub(wb as i64, Ordering::Relaxed);
    }
}

/// splitmix64: a fresh thread's seed, from its own address and a count.
fn seed() -> u64 {
    static N: AtomicU64 = AtomicU64::new(0x2545_F491_4F6C_DD1D);
    let local = 0u8;
    let mut z = (&local as *const u8 as u64)
        ^ N.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) | 1
}

/// Bytes to the next sample: exponential, of mean `rate`.
fn draw(rate: u64) -> i64 {
    let mut x = RNG.with(|r| r.get());
    if x == 0 {
        x = seed();
    }
    // xorshift64*
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    RNG.with(|r| r.set(x));
    let bits = x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11;
    // in (0, 1]
    let u = (bits as f64 + 1.0) / (1u64 << 53) as f64;
    ((-u.ln()) * rate as f64).max(1.0) as i64
}

#[cold]
#[inline(never)]
fn sample(p: *mut u8, size: usize) {
    if BUSY.try_with(|b| b.replace(true)).unwrap_or(true) {
        return;
    }
    let rate = RATE.load(Ordering::Relaxed).max(1);
    if RNG.with(|r| r.get()) == 0 {
        // a thread's first draw: where its first sample falls, rather
        // than its first allocation always sampled
        let left = draw(rate) - size as i64;
        COUNTDOWN.with(|c| c.set(left));
        if left > 0 {
            BUSY.with(|b| b.set(false));
            return;
        }
    }
    COUNTDOWN.with(|c| c.set(draw(rate)));
    record(p as usize, size, rate);
    BUSY.with(|b| b.set(false));
}

fn record(p: usize, size: usize, rate: u64) {
    let s = size.max(1) as f64;
    let prob = 1.0 - (-s / rate as f64).exp();
    let prob = if prob > 0.0 { prob } else { 1.0 };
    let wb = (s / prob) as u64;
    let wn = (ONE / prob) as u64;
    let mut frames = [0u32; DEPTH];
    let depth = crate::profile::peek_frames(&mut frames);
    let key = SiteKey {
        frames,
        depth: depth as u8,
        scope: crate::memory::scope_now(),
    };
    let site = site_index(key);
    let (slot, generation) = crate::memory::slot_now();
    let b = bucket_of(p);
    for (w, k) in KEYS[b].keys.iter().enumerate() {
        if k.compare_exchange(0, RESERVED, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            let e = b * WAYS + w;
            META[e].store(
                (site as u64) << 40 | (slot as u64 & 0xff) << 32 | generation as u64,
                Ordering::Relaxed,
            );
            WEIGHT_B[e].store(wb, Ordering::Relaxed);
            WEIGHT_N[e].store(wn, Ordering::Relaxed);
            let st = &SITE_STATS[site as usize];
            st.live_b.fetch_add(wb as i64, Ordering::Relaxed);
            st.live_n.fetch_add(wn as i64, Ordering::Relaxed);
            st.alloc_b.fetch_add(wb, Ordering::Relaxed);
            st.alloc_n.fetch_add(wn, Ordering::Relaxed);
            if crate::memory::slot_is(slot, generation) {
                HELD[slot].fetch_add(wb as i64, Ordering::Relaxed);
            }
            SAMPLES.fetch_add(1, Ordering::Relaxed);
            k.store(p, Ordering::Release);
            return;
        }
    }
    // the bucket is full: the allocation still counts as made
    let st = &SITE_STATS[site as usize];
    st.alloc_b.fetch_add(wb, Ordering::Relaxed);
    st.alloc_n.fetch_add(wn, Ordering::Relaxed);
    DROPPED.fetch_add(1, Ordering::Relaxed);
}

fn site_index(key: SiteKey) -> u32 {
    let mut guard = SITES.lock().unwrap_or_else(|e| e.into_inner());
    let sites = guard.get_or_insert_with(|| Sites {
        index: rustc_hash::FxHashMap::default(),
        keys: vec![SiteKey {
            frames: [0; DEPTH],
            depth: 0,
            scope: 255,
        }],
    });
    if let Some(i) = sites.index.get(&key) {
        return *i;
    }
    if sites.keys.len() >= MAX_SITES {
        return 0;
    }
    let i = sites.keys.len() as u32;
    sites.keys.push(key);
    sites.index.insert(key, i);
    i
}

/// A task claimed `slot`: what it holds starts from nothing.
pub(crate) fn task_started(slot: usize) {
    if let Some(h) = HELD.get(slot) {
        h.store(0, Ordering::Relaxed);
    }
}

/// Whether the heap profile has run in this process (its figures mean
/// something).
pub fn ran() -> bool {
    STARTED_MS.load(Ordering::Relaxed) != 0
}

/// What a live task's allocations still hold, by the samples (`None`
/// when the heap profile has not run).
pub fn held(slot: usize) -> Option<u64> {
    if STARTED_MS.load(Ordering::Relaxed) == 0 {
        return None;
    }
    Some(HELD.get(slot)?.load(Ordering::Relaxed).max(0) as u64)
}

/// Runs `f` with the profile's own allocations unsampled.
fn quiet<T>(f: impl FnOnce() -> T) -> T {
    let was = BUSY.try_with(|b| b.replace(true)).unwrap_or(true);
    let out = f();
    let _ = BUSY.try_with(|b| b.set(was));
    out
}

/// Forget every sampled block: each is taken out of the table as a free
/// would take it, so a free racing this takes it at most once.
fn clear_table() {
    for (b, bucket) in KEYS.iter().enumerate() {
        for (w, key) in bucket.keys.iter().enumerate() {
            let p = key.load(Ordering::Acquire);
            if p > RESERVED {
                take(b * WAYS + w, p);
            }
        }
    }
}

/// Begin sampling at `rate` bytes (from now: what was allocated before
/// is not seen). `Err` when it already runs.
pub fn start(rate: u64) -> Result<(), String> {
    if rate == 0 {
        return Err("the rate is bytes between samples, at least 1".into());
    }
    if ON.load(Ordering::SeqCst) {
        return Err("the heap profile is already running".into());
    }
    quiet(|| {
        clear_table();
        for s in SITE_STATS.iter() {
            s.live_b.store(0, Ordering::Relaxed);
            s.live_n.store(0, Ordering::Relaxed);
            s.alloc_b.store(0, Ordering::Relaxed);
            s.alloc_n.store(0, Ordering::Relaxed);
        }
        for h in HELD.iter() {
            h.store(0, Ordering::Relaxed);
        }
        DROPPED.store(0, Ordering::Relaxed);
        RATE.store(rate, Ordering::Relaxed);
        STARTED_MS.store(unix_ms().max(1), Ordering::Relaxed);
        crate::profile::frames_retain();
        ON.store(true, Ordering::SeqCst);
    });
    Ok(())
}

/// Stop sampling and forget the samples. `Err` when it is not running.
pub fn stop() -> Result<(), String> {
    if !ON.swap(false, Ordering::SeqCst) {
        return Err("the heap profile is not running".into());
    }
    quiet(|| {
        crate::profile::frames_release();
        clear_table();
    });
    Ok(())
}

/// `OLANG_HEAP` at start: `1` (or empty, `on`) at the default rate, or a
/// rate in bytes; and with `OLANG_HEAP_DIR`, a snapshot written there
/// every `OLANG_HEAP_EVERY` seconds (300).
pub fn start_from_env() {
    let Some(v) = std::env::var_os("OLANG_HEAP") else {
        return;
    };
    let v = v.to_string_lossy();
    let rate = match v.trim() {
        "" | "1" | "on" | "true" => DEFAULT_RATE,
        "0" | "off" | "false" => return,
        n => match parse_bytes(n) {
            Some(r) => r,
            None => {
                eprintln!("olang: OLANG_HEAP={n} is not a rate in bytes (512k, 1m, 65536); using 512k");
                DEFAULT_RATE
            }
        },
    };
    if start(rate).is_err() {
        return;
    }
    if let Some(dir) = std::env::var_os("OLANG_HEAP_DIR").filter(|d| !d.is_empty()) {
        let every = std::env::var("OLANG_HEAP_EVERY")
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|s| *s > 0.0)
            .unwrap_or(300.0);
        let keep = std::env::var("OLANG_HEAP_KEEP")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .unwrap_or(100)
            .max(2);
        let dir = std::path::PathBuf::from(dir);
        let _ = std::thread::Builder::new()
            .name("olang-heap-snapshots".into())
            .spawn(move || snapshot_loop(dir, every, keep));
    }
}

/// `512k`, `1m`, `65536`.
pub fn parse_bytes(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let (n, mul) = if let Some(n) = s.strip_suffix("kib").or(s.strip_suffix("kb")).or(s.strip_suffix('k')) {
        (n, 1024)
    } else if let Some(n) = s.strip_suffix("mib").or(s.strip_suffix("mb")).or(s.strip_suffix('m')) {
        (n, 1024 * 1024)
    } else {
        (s.as_str(), 1)
    };
    let n: u64 = n.trim().parse().ok()?;
    (n > 0).then_some(n * mul)
}

fn snapshot_loop(dir: std::path::PathBuf, every_s: f64, keep: usize) {
    let _ = std::fs::create_dir_all(&dir);
    let pid = std::process::id();
    loop {
        std::thread::sleep(std::time::Duration::from_secs_f64(every_s));
        if !sampling() {
            return;
        }
        let snap = snapshot();
        let path = dir.join(format!("heap-{pid}-{}.json", snap.t_ms));
        let _ = write_snapshot(&snap, &path);
        prune(&dir, pid, keep);
    }
}

/// Keep the first snapshot of this process and the newest `keep - 1`.
fn prune(dir: &std::path::Path, pid: u32, keep: usize) {
    let prefix = format!("heap-{pid}-");
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<std::path::PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".json"))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort();
    let extra = files.len() - keep;
    for f in files.iter().skip(1).take(extra) {
        let _ = std::fs::remove_file(f);
    }
}

pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

type AtEnd = Box<dyn FnOnce() + Send>;
static AT_END: Mutex<Option<AtEnd>> = Mutex::new(None);

/// Run `f` once when the program's body is over, before its values go
/// (`olang memory run`), or at `os.exit`.
pub fn at_program_end(f: AtEnd) {
    *AT_END.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

pub fn program_ended() {
    let f = AT_END.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(f) = f {
        f();
    }
}

// ── snapshots ───────────────────────────────────────────────────────

/// One site in a snapshot: estimates, from the samples.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Site {
    /// What names the site across snapshots of one program.
    pub key: String,
    /// The innermost olang function that is not a builtin (or what the
    /// allocation was outside of), its file and declaration line.
    pub function: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    /// The innermost frames, innermost first, builtins included.
    #[serde(default)]
    pub path: Vec<String>,
    /// `values`, `program` (loading it) or `code` (the runtime's).
    pub scope: String,
    /// Live bytes and blocks.
    pub bytes: u64,
    pub count: u64,
    /// Bytes and blocks allocated here since sampling began.
    #[serde(default)]
    pub allocated: u64,
    #[serde(default)]
    pub allocations: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskRow {
    pub name: String,
    pub net: u64,
    #[serde(default)]
    pub held: Option<u64>,
    pub allocated: u64,
    pub allocations: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub format: u32,
    pub kind: String,
    pub olang: String,
    pub pid: u32,
    /// When it was taken and when sampling began (unix ms).
    pub t_ms: u64,
    pub since_ms: u64,
    pub on: bool,
    pub rate: u64,
    /// `runtime.memory()`'s exact figures.
    pub heap: u64,
    pub program: u64,
    pub code: u64,
    pub values: u64,
    /// Allocations made and bytes asked for, ever (exact).
    pub allocated: u64,
    pub allocations: u64,
    /// What the live samples stand for (an estimate of the part of the
    /// heap allocated since sampling began), how many there are, and
    /// how many a full bucket dropped.
    pub sampled: u64,
    pub samples: u64,
    pub dropped: u64,
    pub sites: Vec<Site>,
    #[serde(default)]
    pub tasks: Vec<TaskRow>,
}

const SCOPES: [&str; 3] = ["values", "program", "code"];

/// What the heap holds now, by site, largest first.
pub fn snapshot() -> Snapshot {
    quiet(snapshot_inner)
}

fn snapshot_inner() -> Snapshot {
    let keys: Vec<SiteKey> = SITES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.keys.clone())
        .unwrap_or_default();
    let mut lines = crate::profile::LineCache::default();
    let mut names: std::collections::HashMap<u32, (String, Option<String>)> =
        std::collections::HashMap::new();
    let mut rows: Vec<Site> = Vec::new();
    let mut by_key: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut sampled = 0u64;
    for (i, key) in keys.iter().enumerate() {
        let st = &SITE_STATS[i];
        let bytes = st.live_b.load(Ordering::Relaxed).max(0) as u64;
        let count = (st.live_n.load(Ordering::Relaxed).max(0) as f64 / ONE).round() as u64;
        let allocated = st.alloc_b.load(Ordering::Relaxed);
        let allocations = (st.alloc_n.load(Ordering::Relaxed) as f64 / ONE).round() as u64;
        if allocated == 0 && bytes == 0 {
            continue;
        }
        sampled += bytes;
        let mut path = Vec::new();
        let mut inner: Option<(String, Option<String>)> = None;
        for &id in &key.frames[..key.depth as usize] {
            let (name, file) = names
                .entry(id)
                .or_insert_with(|| crate::profile::frame_key(id))
                .clone();
            if inner.is_none() && id & crate::profile::PEEK_BUILTIN == 0 {
                inner = Some((name.clone(), file.clone()));
            }
            path.push(name);
        }
        let scope = SCOPES.get(key.scope as usize).copied().unwrap_or("values");
        let (function, file, line) = match inner {
            Some((name, file)) => {
                let line = file.as_deref().and_then(|f| lines.line_of(&name, f));
                (name, file, line)
            }
            None if i == 0 => ("(more sites than the profile keeps)".to_string(), None, None),
            None => match path.first() {
                Some(b) => (b.clone(), None, None),
                None => ("(top level, or no olang frame)".to_string(), None, None),
            },
        };
        let key_text = format!(
            "{scope}|{}|{}",
            file.as_deref().unwrap_or(""),
            if path.is_empty() { function.clone() } else { path.join(";") }
        );
        if let Some(&j) = by_key.get(&key_text) {
            let r: &mut Site = &mut rows[j];
            r.bytes += bytes;
            r.count += count;
            r.allocated += allocated;
            r.allocations += allocations;
            continue;
        }
        by_key.insert(key_text.clone(), rows.len());
        rows.push(Site {
            key: key_text,
            function,
            file,
            line,
            path,
            scope: scope.to_string(),
            bytes,
            count,
            allocated,
            allocations,
        });
    }
    rows.sort_by(|a, b| {
        b.bytes
            .cmp(&a.bytes)
            .then(b.allocated.cmp(&a.allocated))
            .then_with(|| a.key.cmp(&b.key))
    });
    let (heap, program, code, values) = crate::memory::breakdown();
    let (allocations, allocated) = crate::memory::allocation_stats();
    let tasks = crate::memory::tasks()
        .into_iter()
        .map(|t| TaskRow {
            held: held(t.slot),
            name: t.name,
            net: t.net,
            allocated: t.allocated,
            allocations: t.allocations,
        })
        .collect();
    Snapshot {
        format: 1,
        kind: "heap".into(),
        olang: crate::version::VERSION.to_string(),
        pid: std::process::id(),
        t_ms: unix_ms(),
        since_ms: STARTED_MS.load(Ordering::Relaxed),
        on: sampling(),
        rate: RATE.load(Ordering::Relaxed),
        heap,
        program,
        code,
        values,
        allocated,
        allocations,
        sampled,
        samples: SAMPLES.load(Ordering::Relaxed) as u64,
        dropped: DROPPED.load(Ordering::Relaxed),
        sites: rows,
        tasks,
    }
}

/// Write a snapshot as JSON, atomically (a temporary file renamed).
pub fn write_snapshot(snap: &Snapshot, path: &std::path::Path) -> Result<(), String> {
    let text = quiet(|| serde_json::to_string(snap)).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_snapshot(path: &std::path::Path) -> Result<Snapshot, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: not a heap snapshot: {e}", path.display()))
}

/// A site's change between two snapshots.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Growth {
    pub key: String,
    pub function: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub path: Vec<String>,
    pub scope: String,
    /// Live bytes then and now, and the change.
    pub before: u64,
    pub after: u64,
    pub grew: i64,
    pub count_grew: i64,
    /// Bytes allocated here between the two (from the cumulative figures).
    pub allocated: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Diff {
    pub span_ms: i64,
    pub heap_grew: i64,
    pub values_grew: i64,
    pub code_grew: i64,
    pub program_grew: i64,
    pub sampled_grew: i64,
    /// Bytes allocated between the two (exact).
    pub allocated: u64,
    /// Every site that changed, the most grown first.
    pub sites: Vec<Growth>,
}

/// What changed from `a` to `b`, by site. Snapshots of one sampling run
/// compare site by site; across runs (a restart between) the live
/// figures still compare, the cumulative ones do not.
pub fn diff(a: &Snapshot, b: &Snapshot) -> Diff {
    let before: std::collections::HashMap<&str, &Site> =
        a.sites.iter().map(|s| (s.key.as_str(), s)).collect();
    let same_run = a.pid == b.pid && a.since_ms == b.since_ms;
    let mut sites: Vec<Growth> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in &b.sites {
        seen.insert(s.key.as_str());
        let old = before.get(s.key.as_str());
        let (ob, oc, oa) = old.map(|o| (o.bytes, o.count, o.allocated)).unwrap_or((0, 0, 0));
        let allocated = if same_run || old.is_some() {
            s.allocated.saturating_sub(oa)
        } else {
            s.allocated
        };
        if ob == s.bytes && oc == s.count && allocated == 0 {
            continue;
        }
        sites.push(Growth {
            key: s.key.clone(),
            function: s.function.clone(),
            file: s.file.clone(),
            line: s.line,
            path: s.path.clone(),
            scope: s.scope.clone(),
            before: ob,
            after: s.bytes,
            grew: s.bytes as i64 - ob as i64,
            count_grew: s.count as i64 - oc as i64,
            allocated,
        });
    }
    for s in &a.sites {
        if seen.contains(s.key.as_str()) || s.bytes == 0 {
            continue;
        }
        sites.push(Growth {
            key: s.key.clone(),
            function: s.function.clone(),
            file: s.file.clone(),
            line: s.line,
            path: s.path.clone(),
            scope: s.scope.clone(),
            before: s.bytes,
            after: 0,
            grew: -(s.bytes as i64),
            count_grew: -(s.count as i64),
            allocated: 0,
        });
    }
    sites.sort_by(|x, y| y.grew.cmp(&x.grew).then_with(|| x.key.cmp(&y.key)));
    Diff {
        span_ms: b.t_ms as i64 - a.t_ms as i64,
        heap_grew: b.heap as i64 - a.heap as i64,
        values_grew: b.values as i64 - a.values as i64,
        code_grew: b.code as i64 - a.code as i64,
        program_grew: b.program as i64 - a.program as i64,
        sampled_grew: b.sampled as i64 - a.sampled as i64,
        allocated: if a.pid == b.pid {
            b.allocated.saturating_sub(a.allocated)
        } else {
            b.allocated
        },
        sites,
    }
}

/// Allocations by the innermost frame that made them, since sampling
/// began: `frame id → (bytes, allocations)`, estimates — what a profile's
/// functions carry as `alloc_b` and `allocs_n`.
pub fn by_innermost_frame() -> std::collections::HashMap<u32, (u64, u64)> {
    quiet(|| {
        let keys: Vec<SiteKey> = SITES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|s| s.keys.clone())
            .unwrap_or_default();
        let mut out: std::collections::HashMap<u32, (u64, u64)> = std::collections::HashMap::new();
        for (i, key) in keys.iter().enumerate() {
            if key.depth == 0 {
                continue;
            }
            let st = &SITE_STATS[i];
            let b = st.alloc_b.load(Ordering::Relaxed);
            let n = (st.alloc_n.load(Ordering::Relaxed) as f64 / ONE).round() as u64;
            let e = out.entry(key.frames[0] & !crate::profile::PEEK_BUILTIN).or_insert((0, 0));
            e.0 += b;
            e.1 += n;
        }
        out
    })
}

// ── as text, for `olang memory` ──────────────────────────────────────

pub fn mb(b: f64) -> String {
    let a = b.abs();
    if a >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.2} GB", b / (1024.0 * 1024.0 * 1024.0))
    } else if a >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else if a >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{b:.0} B")
    }
}

fn place(file: &Option<String>, line: Option<u32>) -> String {
    match (file, line) {
        (Some(f), Some(l)) => format!("{f}:{l}"),
        (Some(f), None) => f.clone(),
        _ => String::new(),
    }
}

fn via(path: &[String], function: &str) -> String {
    let rest: Vec<&str> = path
        .iter()
        .map(|s| s.as_str())
        .filter(|s| *s != function)
        .collect();
    if rest.is_empty() {
        String::new()
    } else {
        format!(" via {}", rest.join(" < "))
    }
}

pub fn show_text(s: &Snapshot, top: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "heap {}: program {}, code {}, values {}\n",
        mb(s.heap as f64),
        mb(s.program as f64),
        mb(s.code as f64),
        mb(s.values as f64)
    ));
    out.push_str(&format!(
        "sampled since start of sampling: {} live in {} samples (one per {} allocated){}\n\n",
        mb(s.sampled as f64),
        s.samples,
        mb(s.rate as f64),
        if s.dropped > 0 { format!(", {} dropped", s.dropped) } else { String::new() }
    ));
    out.push_str(&format!("{:>10} {:>9} {:>10}  {:<7} site\n", "live", "blocks", "allocated", "scope"));
    for r in s.sites.iter().take(top) {
        out.push_str(&format!(
            "{:>10} {:>9} {:>10}  {:<7} {}{}  {}\n",
            mb(r.bytes as f64),
            r.count,
            mb(r.allocated as f64),
            r.scope,
            r.function,
            via(&r.path, &r.function),
            place(&r.file, r.line)
        ));
    }
    if !s.tasks.is_empty() {
        out.push_str("\ntasks: name, held (sampled live), net (allocated less freed by itself)\n");
        for t in &s.tasks {
            out.push_str(&format!(
                "  {:<20} {:>10} {:>10}\n",
                t.name,
                t.held.map(|h| mb(h as f64)).unwrap_or_else(|| "-".into()),
                mb(t.net as f64)
            ));
        }
    }
    out
}

pub fn diff_text(d: &Diff, top: usize) -> String {
    let mut out = String::new();
    let hours = d.span_ms as f64 / 3_600_000.0;
    let span = if hours >= 1.0 {
        format!("{hours:.1} h")
    } else {
        format!("{:.1} min", d.span_ms as f64 / 60_000.0)
    };
    let signed = |v: i64| {
        if v >= 0 {
            format!("+{}", mb(v as f64))
        } else {
            mb(v as f64)
        }
    };
    out.push_str(&format!(
        "over {span}: heap {}, values {}, code {}, program {}; {} allocated meanwhile\n",
        signed(d.heap_grew),
        signed(d.values_grew),
        signed(d.code_grew),
        signed(d.program_grew),
        mb(d.allocated as f64)
    ));
    out.push_str(&format!("sampled live: {}\n\n", signed(d.sampled_grew)));
    out.push_str(&format!("{:>10} {:>10} {:>9} {:>10}  {:<7} site\n", "grew", "now", "blocks", "allocated", "scope"));
    let grown: Vec<&Growth> = d.sites.iter().filter(|g| g.grew > 0).take(top).collect();
    for g in &grown {
        out.push_str(&format!(
            "{:>10} {:>10} {:>+9} {:>10}  {:<7} {}{}  {}\n",
            signed(g.grew),
            mb(g.after as f64),
            g.count_grew,
            mb(g.allocated as f64),
            g.scope,
            g.function,
            via(&g.path, &g.function),
            place(&g.file, g.line)
        ));
    }
    let shrunk: Vec<&Growth> = d.sites.iter().rev().filter(|g| g.grew < 0).take(5).collect();
    if !shrunk.is_empty() {
        out.push_str("\nshrank most:\n");
        for g in shrunk {
            out.push_str(&format!(
                "{:>10} {:>10}  {:<7} {}{}  {}\n",
                signed(g.grew),
                mb(g.after as f64),
                g.scope,
                g.function,
                via(&g.path, &g.function),
                place(&g.file, g.line)
            ));
        }
    }
    out
}

// ── `olang memory show` and `olang memory diff` ─────────────────────

/// A folder's snapshots of its newest process: (first, newest).
fn first_and_newest(dir: &std::path::Path) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // heap-<pid>-<t_ms>.json
    let mut found: Vec<(u64, u32, std::path::PathBuf)> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_string();
            let rest = name.strip_prefix("heap-")?.strip_suffix(".json")?;
            let (pid, t) = rest.split_once('-')?;
            Some((t.parse().ok()?, pid.parse().ok()?, p))
        })
        .collect();
    found.sort();
    let Some((_, pid, newest)) = found.last().cloned() else {
        return Err(format!("{}: no heap-<pid>-<time>.json snapshots", dir.display()));
    };
    let first = found
        .iter()
        .find(|(_, p, _)| *p == pid)
        .map(|(_, _, f)| f.clone())
        .unwrap_or_else(|| newest.clone());
    if first == newest {
        return Err(format!(
            "{}: one snapshot of process {pid} so far; a diff needs two",
            dir.display()
        ));
    }
    Ok((first, newest))
}

pub fn cli(action: &str, args: &[String], top: usize, format: &str) -> i32 {
    // `--top`/`--format` after the files land among the arguments (they
    // are a program's own for `run`): read them here
    let (mut top, mut format) = (top, format.to_string());
    let mut files: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        match flag {
            "--top" | "--format" => {
                let Some(v) = inline.or_else(|| it.next().cloned()) else {
                    eprintln!("olang memory: {flag} needs a value");
                    return 2;
                };
                if flag == "--top" {
                    match v.parse() {
                        Ok(n) => top = n,
                        Err(_) => {
                            eprintln!("olang memory: --top is a number, not `{v}`");
                            return 2;
                        }
                    }
                } else {
                    format = v;
                }
            }
            _ => files.push(a.clone()),
        }
    }
    let args = files.as_slice();
    let format = format.as_str();
    if format != "text" && format != "json" {
        eprintln!("olang memory: --format is `text` or `json`, not `{format}`");
        return 2;
    }
    let json = format == "json";
    let result: Result<String, String> = match (action, args) {
        ("show", [file]) => read_snapshot(std::path::Path::new(file)).map(|s| {
            if json {
                serde_json::to_string(&s).unwrap_or_default()
            } else {
                show_text(&s, top)
            }
        }),
        ("diff", [dir]) => first_and_newest(std::path::Path::new(dir)).and_then(|(a, b)| {
            let (sa, sb) = (read_snapshot(&a)?, read_snapshot(&b)?);
            let d = diff(&sa, &sb);
            Ok(if json {
                serde_json::to_string(&d).unwrap_or_default()
            } else {
                format!("{} → {}\n{}", a.display(), b.display(), diff_text(&d, top))
            })
        }),
        ("diff", [a, b]) => (|| -> Result<String, String> {
            let sa = read_snapshot(std::path::Path::new(a))?;
            let sb = read_snapshot(std::path::Path::new(b))?;
            let d = diff(&sa, &sb);
            Ok(if json {
                serde_json::to_string(&d).unwrap_or_default()
            } else {
                diff_text(&d, top)
            })
        })(),
        ("show", _) => Err("olang memory show takes one snapshot".into()),
        ("diff", _) => Err("olang memory diff takes two snapshots, or a folder of them".into()),
        _ => Err(format!(
            "olang memory: `{action}` is not an action: `run`, `show` or `diff`"
        )),
    };
    match result {
        Ok(text) => {
            print!("{text}");
            if !text.ends_with('\n') {
                println!();
            }
            0
        }
        Err(e) => {
            eprintln!("olang memory: {e}");
            2
        }
    }
}

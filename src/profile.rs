//! The sampling profiler behind `olang profile`.
//!
//! Every performance hunt in this repository so far has gone through
//! `sample(1)` and Rust symbol names — a tool that answers "which
//! *Rust* function is hot", when the question is "which *olang*
//! function is hot, and which tier is it running on". This answers the
//! second question.
//!
//! The mechanism is a shadow stack per thread, sampled by a background
//! thread:
//!
//!   * Each execution tier pushes a frame — a function name and the
//!     tier running it — on entry and pops it on exit. The push is two
//!     relaxed atomic stores into a fixed array; no allocation, no
//!     lock, no ordering with other threads.
//!   * A sampler thread wakes on an interval, walks the registered
//!     stacks, and records what each was doing. Samples are counts, so
//!     the profile is statistical: the interval bounds the error, and
//!     a function that never appears was never on a stack at a tick.
//!   * When profiling is off — every ordinary run — the hot path is
//!     one relaxed load of an `AtomicBool` and a predicted-false
//!     branch. Nothing else in the language pays for this file
//!     existing.
//!
//! Name interning takes a lock, so a profiled run is slower than an
//! unprofiled one (measurably, not dramatically). That trade is
//! deliberate: the alternative — an id cached in every `Function` —
//! would spend memory and complexity on every run to speed up the rare
//! one.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Frames recorded per thread. Deeper stacks still profile correctly —
/// the leaf is always recorded, so self time stays exact — but their
/// call paths are reported truncated.
const MAX_FRAMES: usize = 192;

/// Which execution tier is running a frame. The whole point of
/// profiling a tiered language: "slow" and "slow *and still
/// interpreted*" are different findings with different fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Interpreter = 0,
    Vm = 1,
    Native = 2,
}

/// Marks a frame as a *builtin* rather than a user function. Stored in
/// the tier byte's high bit so a builtin costs no extra array: it needs
/// to appear in call paths (a lambda's caller is usually `map`, and
/// saying so is the whole point) without being chosen as the name that
/// qualifies an anonymous frame, where the useful answer is the user
/// function further up.
const BUILTIN_BIT: u8 = 0x80;
/// Marks a frame as anonymous (its display name is qualified by a
/// parent). Rides the tier byte like BUILTIN_BIT so the parent walk in
/// `named_parent` reads bits instead of interned names.
const ANON_BIT: u8 = 0x40;
/// Marks an interpreter frame whose call a tier took over (tier
/// statistics): its call is counted once, on the tier that ran it.
const TAKEN_BIT: u8 = 0x20;

impl Tier {
    fn from_u8(v: u8) -> Tier {
        match v & !(BUILTIN_BIT | ANON_BIT | TAKEN_BIT) {
            1 => Tier::Vm,
            2 => Tier::Native,
            _ => Tier::Interpreter,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tier::Interpreter => "interp",
            Tier::Vm => "vm",
            Tier::Native => "native",
        }
    }
}

/// One thread's shadow stack. Written only by its own thread; read by
/// the sampler. A torn read (the sampler catching a push mid-flight)
/// costs at most one misattributed sample out of thousands, which is
/// why none of this needs ordering stronger than `Relaxed`.
struct ThreadStack {
    depth: AtomicUsize,
    frames: [AtomicU32; MAX_FRAMES],
    tiers: [AtomicU8; MAX_FRAMES],
    /// False for the first thread to run olang code, true for the
    /// parallel workers that register later. Worth distinguishing: work
    /// handed to `par_map` (or to the automatic parallelism a large
    /// `map` takes) runs with its caller on *another* thread, so its
    /// frames legitimately appear with no ancestors at all.
    worker: bool,
    /// Set while the thread is parked — in `chan.recv`, `chan.send` on a
    /// full channel, `time.sleep`, `task.join`, a server's accept loop.
    /// A parked thread does no work, and charging its wall time to the
    /// function that parked it made every actor loop the hottest row of
    /// a profile whose CPU was idle.
    blocked: AtomicBool,
    /// Tier statistics (`--ovm-stats=json`): calls per (frame id, tier)
    /// and the native attempts that declined, `STAT_SLOTS` counters a
    /// frame id. Written only by this thread (a load and a store, no
    /// read-modify-write); allocated the first time a counted frame is
    /// pushed, so an ordinary profile never pays for it.
    counts: OnceLock<Box<[AtomicU64]>>,
}

/// Frame ids counted per thread; a frame interned past this is timed by
/// the sampler but its calls are not counted.
const STAT_IDS: usize = 4096;
/// Per frame id: calls on the interpreter, the VM and native code, the
/// native attempts that declined, then the declines by reason (1–4).
const STAT_SLOTS: usize = 8;
const SLOT_DECLINES: usize = 3;

impl ThreadStack {
    fn new(worker: bool) -> Self {
        ThreadStack {
            depth: AtomicUsize::new(0),
            blocked: AtomicBool::new(false),
            counts: OnceLock::new(),
            frames: std::array::from_fn(|_| AtomicU32::new(0)),
            tiers: std::array::from_fn(|_| AtomicU8::new(0)),
            worker,
        }
    }
}

struct Registry {
    stacks: Mutex<Vec<Arc<ThreadStack>>>,
    names: Mutex<(Vec<String>, HashMap<String, u32>)>,
    enabled: AtomicBool,
    /// Who wants frames recorded: a sampler session, the instrumented
    /// mode, tier statistics. `enabled` is true while any does, so one
    /// finishing does not switch off another.
    users: AtomicUsize,
    /// Sampler sessions a profile started (not the statistics' own), so
    /// `runtime.profile_start` still refuses to nest inside `olang
    /// profile` while statistics are being kept.
    profiles: AtomicUsize,
    /// Counts of threads that have exited, folded in when the sampler
    /// prunes them (`STAT_IDS * STAT_SLOTS`).
    retired: Mutex<Vec<u64>>,
}

fn retain() {
    let r = registry();
    r.users.fetch_add(1, Ordering::SeqCst);
    r.enabled.store(true, Ordering::Relaxed);
}

fn release() {
    let r = registry();
    let before = r
        .users
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| Some(n.saturating_sub(1)))
        .unwrap_or(0);
    if before <= 1 {
        r.enabled.store(false, Ordering::Relaxed);
    }
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry {
        stacks: Mutex::new(Vec::new()),
        names: Mutex::new((Vec::new(), HashMap::new())),
        enabled: AtomicBool::new(false),
        users: AtomicUsize::new(0),
        profiles: AtomicUsize::new(0),
        retired: Mutex::new(Vec::new()),
    })
}

thread_local! {
    static LOCAL: Arc<ThreadStack> = {
        let mut guard = registry().stacks.lock();
        let worker = guard.as_ref().map(|s| !s.is_empty()).unwrap_or(false);
        let stack = Arc::new(ThreadStack::new(worker));
        if let Ok(stacks) = guard.as_mut() {
            stacks.push(stack.clone());
        }
        stack
    };
}

/// True while a profiling run is active. One relaxed load — the guard
/// every hot path checks before doing anything else.
#[inline(always)]
pub fn enabled() -> bool {
    registry().enabled.load(Ordering::Relaxed)
}

// ── the instrumented mode ─────────────────────────────────────────────
//
// The sampler needs a second thread, and a browser session has none.
// The instrumented mode answers the same question — which olang
// function is hot, and on which tier — from the same shadow stack, by
// timing every push/pop pair: exact self and total time per function,
// with a clock read per call instead of a tick. It is what
// `window.olangProfile` in the web SDK's shim reports; a repaint that
// spends its time in one view helper shows that helper at the top.

static INSTRUMENT: AtomicBool = AtomicBool::new(false);

/// (frame id, tier bits): one row of the instrumented report.
type InstrKey = (u32, u8);
/// (calls, self ms, total ms).
type InstrTotals = (u64, f64, f64);

thread_local! {
    /// The open frames: (frame id, tier bits, entered at, children's ms).
    static INSTR_STACK: std::cell::RefCell<Vec<(u32, u8, crate::clock::Instant, f64)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// (frame id, tier bits) → (calls, self ms, total ms).
    static INSTR_TOTALS: std::cell::RefCell<HashMap<InstrKey, InstrTotals>> =
        std::cell::RefCell::new(HashMap::new());
    static INSTR_STARTED: std::cell::Cell<Option<crate::clock::Instant>> =
        const { std::cell::Cell::new(None) };
}

#[inline(always)]
fn instrumenting() -> bool {
    INSTRUMENT.load(Ordering::Relaxed)
}

/// Begin timing every frame on this thread. Turns the shadow stack on;
/// a report is available at any time and `instrument_stop` turns it
/// off again.
pub fn instrument_start() {
    INSTR_TOTALS.with(|t| t.borrow_mut().clear());
    INSTR_STACK.with(|s| s.borrow_mut().clear());
    INSTR_STARTED.with(|s| s.set(Some(crate::clock::Instant::now())));
    if !INSTRUMENT.swap(true, Ordering::Relaxed) {
        retain();
    }
}

/// The report so far, as JSON: `{ "elapsed_ms", "rows": [{ "function",
/// "tier", "builtin", "calls", "self_ms", "total_ms" }] }`, rows by
/// self time descending.
pub fn instrument_report() -> String {
    let elapsed_ms = INSTR_STARTED
        .with(|s| s.get())
        .map(|t| t.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let mut rows: Vec<serde_json::Value> = INSTR_TOTALS.with(|t| {
        let mut entries: Vec<(InstrKey, InstrTotals)> =
            t.borrow().iter().map(|(k, v)| (*k, *v)).collect();
        entries.sort_by(|a, b| {
            b.1.1
                .partial_cmp(&a.1.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries
            .into_iter()
            .map(|((id, bits), (calls, self_ms, total_ms))| {
                serde_json::json!({
                    "function": name_of(id),
                    "tier": Tier::from_u8(bits).label(),
                    "builtin": bits & BUILTIN_BIT != 0,
                    "calls": calls,
                    "self_ms": (self_ms * 1000.0).round() / 1000.0,
                    "total_ms": (total_ms * 1000.0).round() / 1000.0,
                })
            })
            .collect()
    });
    rows.truncate(200);
    serde_json::json!({ "elapsed_ms": (elapsed_ms * 1000.0).round() / 1000.0, "rows": rows })
        .to_string()
}

/// Stop timing and return the final report.
pub fn instrument_stop() -> String {
    let report = instrument_report();
    if INSTRUMENT.swap(false, Ordering::Relaxed) {
        release();
    }
    INSTR_STACK.with(|s| s.borrow_mut().clear());
    report
}

#[inline]
fn instrument_push(id: u32, bits: u8) {
    if instrumenting() {
        INSTR_STACK.with(|s| {
            s.borrow_mut()
                .push((id, bits, crate::clock::Instant::now(), 0.0))
        });
    }
}

#[inline]
fn instrument_pop() {
    if !instrumenting() {
        return;
    }
    INSTR_STACK.with(|s| {
        let mut s = s.borrow_mut();
        let Some((id, bits, start, children_ms)) = s.pop() else {
            return;
        };
        let total_ms = start.elapsed().as_secs_f64() * 1000.0;
        let self_ms = (total_ms - children_ms).max(0.0);
        if let Some(parent) = s.last_mut() {
            parent.3 += total_ms;
        }
        INSTR_TOTALS.with(|t| {
            let mut t = t.borrow_mut();
            let e = t.entry((id, bits)).or_insert((0, 0.0, 0.0));
            e.0 += 1;
            e.1 += self_ms;
            e.2 += total_ms;
        });
    });
}

fn intern(name: &str) -> u32 {
    let mut guard = match registry().names.lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    if let Some(id) = guard.1.get(name) {
        return *id;
    }
    let id = guard.0.len() as u32;
    guard.0.push(name.to_string());
    guard.1.insert(name.to_string(), id);
    id
}

/// A frame's display name: its interned key without the file a
/// statistics run qualifies it with.
fn name_of(id: u32) -> String {
    key_of(id).0
}

/// A frame's name and, when tier statistics were on as it was first
/// pushed, the file its function was declared in.
fn key_of(id: u32) -> (String, Option<String>) {
    let key = registry()
        .names
        .lock()
        .ok()
        .and_then(|g| g.0.get(id as usize).cloned())
        .unwrap_or_else(|| "<unknown>".to_string());
    match key.split_once(FILE_SEP) {
        Some((name, file)) => (name.to_string(), Some(file.to_string())),
        None => (key, None),
    }
}

/// Between a frame's name and its file in an interned key.
const FILE_SEP: char = '\u{1}';

/// Mark this thread parked for the guard's lifetime: the sampler counts
/// the ticks as blocked instead of charging them to the frame on top.
pub struct Blocked(());

pub fn blocked() -> Blocked {
    LOCAL.with(|stack| stack.blocked.store(true, Ordering::Relaxed));
    Blocked(())
}

impl Drop for Blocked {
    fn drop(&mut self) {
        LOCAL.with(|stack| stack.blocked.store(false, Ordering::Relaxed));
    }
}

/// Push a frame. Returns false when profiling is off, so callers can
/// skip the matching `pop` — the pattern is:
///
/// ```ignore
/// let pushed = profile::push(name, Tier::Vm);
/// let result = run();
/// if pushed { profile::pop(); }
/// ```
///
/// Deliberately not an RAII guard: the hot paths that call this are
/// already threading results through early returns, and a guard would
/// borrow across them.
/// Names the tiers give a function value with no name of its own. The
/// interpreter says `<lambda>`, the bytecode compiler says
/// `<function value>`, and `<anonymous>` is the VM's fallback — three
/// spellings of the same thing, which would otherwise split one
/// function across three rows of the report.
fn is_anonymous(name: &str) -> bool {
    matches!(name, "<lambda>" | "<function value>" | "<anonymous>")
        || name.starts_with("<lambda in")
}

thread_local! {
    /// Per-thread memo of resolved frame names: (name ptr, name len,
    /// parent frame id) → interned id. The steady state of a profiled
    /// program is the same functions pushed millions of times; this is
    /// what keeps those pushes off the global intern lock, which
    /// otherwise serializes every thread of a parallel program into a
    /// crawl.
    static NAME_MEMO: std::cell::RefCell<HashMap<(usize, usize, usize, u32), u32>> =
        std::cell::RefCell::new(HashMap::new());
}

/// The nearest named, non-builtin ancestor's frame id on this thread's
/// stack — the qualifier for an anonymous frame (`<lambda in bench>`
/// locates a lambda the way a reader would describe it). Reads only
/// this thread's atomics: builtin and anonymous frames are recognized
/// by bits, never by interned names, so no lock is taken.
fn named_parent(stack: &ThreadStack) -> u32 {
    let depth = stack.depth.load(Ordering::Relaxed).min(MAX_FRAMES);
    for i in (0..depth).rev() {
        let bits = stack.tiers[i].load(Ordering::Relaxed);
        if bits & (BUILTIN_BIT | ANON_BIT) == 0 {
            return stack.frames[i].load(Ordering::Relaxed);
        }
    }
    u32::MAX
}

#[inline]
pub fn push(name: &str, tier: Tier) -> bool {
    push_at(name, None, tier)
}

/// `push`, saying the file the function was declared in. With tier
/// statistics on, a frame is keyed by its name *and* file, so two
/// modules' functions of one name stay two rows (the ambiguous-name
/// case is exactly the one worth seeing); otherwise the file is not
/// read.
#[inline]
pub fn push_at(name: &str, file: Option<&str>, tier: Tier) -> bool {
    if !enabled() {
        return false;
    }
    let anon = is_anonymous(name);
    let file = if stats_on() { file } else { None };
    LOCAL.with(|stack| {
        // Resolve the display name through the per-thread memo; the
        // global intern lock is touched once per new (name, parent)
        // pair per thread, not per call. Normalizing the anonymous
        // spellings before interning is what lets the sampler's
        // adjacent-duplicate collapse fold a promoted call's
        // interpreter, VM, and native frames into one function.
        let parent = if anon { named_parent(stack) } else { u32::MAX };
        let key = (
            name.as_ptr() as usize,
            name.len(),
            file.map(|f| f.as_ptr() as usize).unwrap_or(0),
            parent,
        );
        let id = match NAME_MEMO.with(|m| m.borrow().get(&key).copied()) {
            Some(hit) => hit,
            None => {
                let mut resolved = if anon {
                    if parent == u32::MAX {
                        "<lambda>".to_string()
                    } else {
                        format!("<lambda in {}>", name_of(parent))
                    }
                } else {
                    name.to_string()
                };
                if let Some(f) = file {
                    resolved.push(FILE_SEP);
                    resolved.push_str(f);
                }
                let id = intern(&resolved);
                NAME_MEMO.with(|m| {
                    let mut m = m.borrow_mut();
                    if m.len() >= 4096 {
                        m.clear();
                    }
                    m.insert(key, id);
                });
                id
            }
        };
        let bits = tier as u8 | if anon { ANON_BIT } else { 0 };
        let depth = stack.depth.load(Ordering::Relaxed);
        if depth < MAX_FRAMES {
            stack.frames[depth].store(id, Ordering::Relaxed);
            stack.tiers[depth].store(bits, Ordering::Relaxed);
        }
        // Depth counts past the array so pops stay balanced; frames
        // beyond MAX_FRAMES simply are not recorded.
        stack.depth.store(depth + 1, Ordering::Relaxed);
        if stats_on() {
            count(stack, id, tier as usize, 1);
            // the interpreter's frame for this very call, under a tier's:
            // the call is the tier's (once, though a declined native
            // attempt and the VM both push)
            if tier != Tier::Interpreter && depth > 0 && depth <= MAX_FRAMES {
                let below = stack.tiers[depth - 1].load(Ordering::Relaxed);
                if below & (BUILTIN_BIT | TAKEN_BIT) == 0
                    && Tier::from_u8(below) == Tier::Interpreter
                    && stack.frames[depth - 1].load(Ordering::Relaxed) == id
                {
                    stack.tiers[depth - 1].store(below | TAKEN_BIT, Ordering::Relaxed);
                    count(stack, id, Tier::Interpreter as usize, -1);
                }
            }
        }
        instrument_push(id, bits);
    });
    true
}

/// Add `by` to one of this thread's counters for frame `id`. The thread
/// is the only writer, so a load and a store suffice.
#[inline]
fn count(stack: &ThreadStack, id: u32, slot: usize, by: i64) {
    let id = id as usize;
    if id >= STAT_IDS {
        return;
    }
    let counts = stack.counts.get_or_init(|| {
        (0..STAT_IDS * STAT_SLOTS)
            .map(|_| AtomicU64::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice()
    });
    let c = &counts[id * STAT_SLOTS + slot];
    c.store(
        (c.load(Ordering::Relaxed) as i64 + by).max(0) as u64,
        Ordering::Relaxed,
    );
}

/// Pop a native frame whose call declined (the native code refused the
/// arguments, or deopted): it ran nowhere, so its call is taken back
/// and counted as a decline, with the reason (1–4, `DECLINE_REASONS`).
#[inline]
pub fn pop_declined(reason: u8) {
    if stats_on() {
        LOCAL.with(|stack| {
            let depth = stack.depth.load(Ordering::Relaxed);
            if depth > 0 && depth <= MAX_FRAMES {
                let id = stack.frames[depth - 1].load(Ordering::Relaxed);
                count(stack, id, Tier::Native as usize, -1);
                count(stack, id, SLOT_DECLINES, 1);
                if (1..=4).contains(&reason) {
                    count(stack, id, SLOT_DECLINES + reason as usize, 1);
                }
            }
        });
    }
    pop();
}

/// Why a native attempt declined, by the code `pop_declined` was given.
pub const DECLINE_REASONS: [&str; 4] = [
    "an argument of a kind native code does not take",
    "the native compiler refused it (type inference or code generation)",
    "a guard failed while it ran (a value the native code did not expect, an allocation cap, or an interrupt), and the bytecode ran the call again",
    "no native specialization for these argument kinds",
];

/// Push a frame for a builtin that runs user code — `map`, `fold`,
/// `sort_by`. Without it a lambda passed to `map` appears in the
/// profile with no caller at all, since builtins are Rust and leave no
/// olang frame behind.
#[inline]
pub fn push_builtin(name: &str) -> bool {
    if !enabled() {
        return false;
    }
    let id = intern(name);
    LOCAL.with(|stack| {
        let depth = stack.depth.load(Ordering::Relaxed);
        if depth < MAX_FRAMES {
            stack.frames[depth].store(id, Ordering::Relaxed);
            stack.tiers[depth].store(Tier::Interpreter as u8 | BUILTIN_BIT, Ordering::Relaxed);
        }
        stack.depth.store(depth + 1, Ordering::Relaxed);
        instrument_push(id, Tier::Interpreter as u8 | BUILTIN_BIT);
    });
    true
}

#[inline]
pub fn pop() {
    LOCAL.with(|stack| {
        let depth = stack.depth.load(Ordering::Relaxed);
        stack
            .depth
            .store(depth.saturating_sub(1), Ordering::Relaxed);
    });
    instrument_pop();
}

/// A running profile: the sampler thread plus what it has collected.
pub struct Session {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Collected>>,
    interval_us: u64,
    started: std::time::Instant,
}

#[derive(Default)]
struct Collected {
    /// Full call paths (root-first, as ids) to the number of samples
    /// that observed them.
    stacks: HashMap<Vec<u32>, u64>,
    /// Leaf id and tier to sample count — self time, and the tier the
    /// work actually ran on.
    leaves: HashMap<(u32, u8), u64>,
    total: u64,
    /// Ticks that found every thread idle (between programs, or inside
    /// a builtin that never pushes a frame).
    idle: u64,
    /// Samples taken on parallel worker threads.
    worker_samples: u64,
    /// Each sample charged to the user function it was in: the leaf, or
    /// for a builtin leaf (`map`, `sort_by`) the nearest user frame under
    /// it, keyed with the leaf's tier bits (BUILTIN_BIT for a builtin).
    attributed: HashMap<(u32, u8), u64>,
    /// Ticks on which a thread was parked (a channel receive, a sleep, a
    /// join) — waiting, not working; reported apart from the functions.
    blocked: u64,
}

/// Start sampling. The returned session must be stopped to collect.
pub fn start(interval_us: u64) -> Session {
    let session = start_sampler(interval_us);
    registry().profiles.fetch_add(1, Ordering::SeqCst);
    session
}

fn start_sampler(interval_us: u64) -> Session {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    retain();
    let handle = std::thread::Builder::new()
        .name("olang-profiler".to_string())
        .spawn(move || {
            let mut out = Collected::default();
            let interval = std::time::Duration::from_micros(interval_us.max(50));
            while !stop_for_thread.load(Ordering::Relaxed) {
                std::thread::sleep(interval);
                sample_once(&mut out);
            }
            out
        })
        .expect("spawn profiler thread");
    Session {
        stop,
        handle: Some(handle),
        interval_us,
        started: std::time::Instant::now(),
    }
}

fn sample_once(out: &mut Collected) {
    let Ok(mut stacks) = registry().stacks.lock() else {
        return;
    };
    // Prune threads that have exited: their thread-local clone dropped,
    // leaving the registry's Arc as the only owner. Without this a
    // spawn-heavy program (an http server, a task fan-out) grows the
    // list without bound, every tick scans all of it under the same
    // lock new threads need to register — and thread creation and the
    // sampler livelock each other.
    if stats_on() {
        for s in stacks.iter().filter(|s| std::sync::Arc::strong_count(s) <= 1) {
            retire(s);
        }
    }
    stacks.retain(|s| std::sync::Arc::strong_count(s) > 1);
    out.total += 1;
    let mut saw_any = false;
    for stack in stacks.iter() {
        let depth = stack.depth.load(Ordering::Relaxed).min(MAX_FRAMES);
        // Parked is parked whatever is on the stack: a top-level
        // `time.sleep` and an http worker waiting for a connection have
        // no frame at all, and were not counted anywhere.
        if stack.blocked.load(Ordering::Relaxed) {
            out.blocked += 1;
            saw_any = true;
            continue;
        }
        if depth == 0 {
            continue;
        }
        saw_any = true;
        // Adjacent identical names collapse: a promoted call pushes
        // once per tier it crosses (interpreter entry, VM frame, native
        // entry) and recursion pushes once per level. Neither is a
        // distinct *place in the program*, which is what a call path
        // is meant to name.
        let mut path: Vec<u32> = Vec::with_capacity(depth);
        for i in 0..depth {
            let id = stack.frames[i].load(Ordering::Relaxed);
            if path.last() != Some(&id) {
                path.push(id);
            }
        }
        // The builtin bit rides along: a builtin's time is Rust, and
        // counting it as interpreter time would misattribute exactly
        // the thing this profiler exists to report honestly.
        let leaf_tier = stack.tiers[depth - 1].load(Ordering::Relaxed) & !(ANON_BIT | TAKEN_BIT);
        let leaf = stack.frames[depth - 1].load(Ordering::Relaxed);
        if stack.worker {
            out.worker_samples += 1;
        }
        *out.leaves.entry((leaf, leaf_tier)).or_insert(0) += 1;
        let mut owner = (leaf, leaf_tier);
        if leaf_tier & BUILTIN_BIT != 0 {
            for i in (0..depth - 1).rev() {
                let bits = stack.tiers[i].load(Ordering::Relaxed);
                if bits & BUILTIN_BIT == 0 {
                    owner = (stack.frames[i].load(Ordering::Relaxed), BUILTIN_BIT);
                    break;
                }
            }
        }
        *out.attributed.entry(owner).or_insert(0) += 1;
        *out.stacks.entry(path).or_insert(0) += 1;
    }
    if !saw_any {
        out.idle += 1;
    }
}

impl Session {
    /// Stop sampling and render the report.
    pub fn finish(mut self, elapsed: std::time::Duration, top: usize, label: &str) -> String {
        registry().profiles.fetch_sub(1, Ordering::SeqCst);
        let collected = self.collect();
        render(&collected, elapsed, top, self.interval_us, label)
    }
}

/// What a program reads back from a profile it ran on itself
/// (`runtime.profile_stop`): the counts behind the report, not its text.
pub struct Summary {
    pub interval_us: u64,
    /// Ticks taken, and of those the ones that found every thread idle
    /// and the thread-ticks spent parked (a receive, a sleep, a join, a
    /// server waiting for a connection).
    pub ticks: u64,
    pub idle: u64,
    pub blocked: u64,
    /// Samples that landed in code: the sum of `rows`.
    pub samples: u64,
    /// (function, tier — "interp", "vm", "native" or "builtin" —, self
    /// samples), most first.
    pub rows: Vec<(String, &'static str, u64)>,
}

impl Session {
    /// Stop sampling and answer the counts.
    pub fn finish_summary(mut self) -> Summary {
        registry().profiles.fetch_sub(1, Ordering::SeqCst);
        let collected = self.collect();
        let mut rows: Vec<(String, &'static str, u64)> = collected
            .leaves
            .iter()
            .map(|((id, tier), count)| {
                let label = if tier & BUILTIN_BIT != 0 {
                    "builtin"
                } else {
                    Tier::from_u8(*tier).label()
                };
                (name_of(*id), label, *count)
            })
            .collect();
        rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        Summary {
            interval_us: self.interval_us,
            ticks: collected.total,
            idle: collected.idle,
            blocked: collected.blocked,
            samples: rows.iter().map(|r| r.2).sum(),
            rows,
        }
    }
}

/// The profile a program started on itself, if one is running.
static IN_PROCESS: Mutex<Option<Session>> = Mutex::new(None);

/// `runtime.profile_start`: begin sampling this process. Err when a
/// profile is already running — `olang profile`'s, or an earlier call's.
pub fn start_in_process(interval_us: u64) -> Result<(), String> {
    let mut slot = IN_PROCESS.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() || registry().profiles.load(Ordering::SeqCst) > 0 || instrumenting() {
        return Err("a profile is already running".to_string());
    }
    *slot = Some(start(interval_us));
    Ok(())
}

/// `runtime.profile_stop`: end it and answer what it saw.
pub fn stop_in_process() -> Result<Summary, String> {
    let session = IN_PROCESS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or_else(|| "no profile is running (runtime.profile_start starts one)".to_string())?;
    Ok(session.finish_summary())
}

/// A proportional bar. Filled cells are the share; the track makes
/// small shares visible as "small" rather than as nothing.
fn bar(share: f64, width: usize) -> String {
    let filled = ((share / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn render(
    data: &Collected,
    elapsed: std::time::Duration,
    top: usize,
    interval_us: u64,
    label: &str,
) -> String {
    let mut out = String::new();
    let active: u64 = data.leaves.values().sum();

    out.push_str(&format!("\n  profile · {}\n", label));
    out.push_str(&format!(
        "  {} samples over {:.2}s · {}µs interval\n",
        thousands(active),
        elapsed.as_secs_f64(),
        interval_us
    ));

    if active == 0 {
        out.push_str(
            "\n  No samples landed in olang code. Either the program is too short\n  \
             to profile (try a larger workload), or its time is going to builtins\n  \
             and I/O rather than to functions — which is itself the finding.\n",
        );
        return out;
    }

    // ── time by tier ────────────────────────────────────────────────
    // The headline a tiered language owes its user: not just what was
    // slow, but which engine was running when it was.
    let mut per_tier: HashMap<u8, u64> = HashMap::new();
    let mut builtin_samples = 0u64;
    for ((_, tier), count) in &data.leaves {
        if tier & BUILTIN_BIT != 0 {
            builtin_samples += count;
        } else {
            *per_tier.entry(*tier).or_insert(0) += count;
        }
    }
    out.push_str("\n  TIME BY TIER\n");
    for tier in [Tier::Native, Tier::Vm, Tier::Interpreter] {
        let n = per_tier.get(&(tier as u8)).copied().unwrap_or(0);
        let share = pct(n, active);
        let note = if tier == Tier::Interpreter && share >= 20.0 {
            "  ← never promoted"
        } else {
            ""
        };
        out.push_str(&format!(
            "    {:<7} {}  {:>5.1}%{}\n",
            tier.label(),
            bar(share, 32),
            share,
            note
        ));
    }
    if builtin_samples > 0 {
        // Builtins are Rust: neither a tier nor something a user can
        // promote, but time the reader still has to account for.
        out.push_str(&format!(
            "    {:<7} {}  {:>5.1}%\n",
            "builtin",
            bar(pct(builtin_samples, active), 32),
            pct(builtin_samples, active)
        ));
    }

    // ── per-function self and total ─────────────────────────────────
    let mut total_counts: HashMap<u32, u64> = HashMap::new();
    for (path, count) in &data.stacks {
        let mut seen = std::collections::HashSet::new();
        for id in path {
            if seen.insert(*id) {
                *total_counts.entry(*id).or_insert(0) += count;
            }
        }
    }
    let mut by_fn: HashMap<u32, (u64, HashMap<u8, u64>)> = HashMap::new();
    for ((id, tier), count) in &data.leaves {
        let entry = by_fn.entry(*id).or_insert_with(|| (0, HashMap::new()));
        entry.0 += count;
        *entry.1.entry(*tier).or_insert(0) += count;
    }
    let mut rows: Vec<(u32, u64, HashMap<u8, u64>)> = by_fn
        .into_iter()
        .map(|(id, (self_n, tiers))| (id, self_n, tiers))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let name_width = rows
        .iter()
        .take(top)
        .map(|(id, _, _)| name_of(*id).chars().count())
        .max()
        .unwrap_or(8)
        .clamp(8, 40);
    out.push_str("\n  FUNCTIONS\n");
    out.push_str(&format!(
        "    {:<width$}  {:>18}  {:>6}  {:>7}  {}\n",
        "function",
        "self",
        "total",
        "samples",
        "tier",
        width = name_width
    ));
    for (id, self_n, tiers) in rows.iter().take(top) {
        let total_n = total_counts.get(id).copied().unwrap_or(*self_n);
        let mut tier_list: Vec<(&u8, &u64)> = tiers.iter().collect();
        tier_list.sort_by(|a, b| b.1.cmp(a.1));
        let tier_label = match tier_list.as_slice() {
            [] => "-".to_string(),
            [(t, _)] if **t & BUILTIN_BIT != 0 => "builtin".to_string(),
            [(t, _)] => Tier::from_u8(**t).label().to_string(),
            [(t, n), rest @ ..] => {
                if pct(**n, *self_n) >= 90.0 {
                    Tier::from_u8(**t).label().to_string()
                } else {
                    format!(
                        "{}+{}",
                        Tier::from_u8(**t).label(),
                        Tier::from_u8(*rest[0].0).label()
                    )
                }
            }
        };
        let mut name = name_of(*id);
        if name.chars().count() > name_width {
            name = format!("{}…", name.chars().take(name_width - 1).collect::<String>());
        }
        out.push_str(&format!(
            "    {:<width$}  {} {:>5.1}%  {:>5.1}%  {:>7}  {}\n",
            name,
            bar(pct(*self_n, active), 11),
            pct(*self_n, active),
            pct(total_n, active),
            thousands(*self_n),
            tier_label,
            width = name_width
        ));
    }
    if rows.len() > top {
        out.push_str(&format!(
            "    … {} more (--top {} to see them)\n",
            rows.len() - top,
            rows.len()
        ));
    }

    // ── hottest complete paths ──────────────────────────────────────
    let mut paths: Vec<(&Vec<u32>, &u64)> = data.stacks.iter().collect();
    paths.sort_by(|a, b| b.1.cmp(a.1));
    out.push_str("\n  HOTTEST CALL PATHS\n");
    for (path, count) in paths.iter().take(5) {
        let rendered: Vec<String> = path.iter().map(|id| name_of(*id)).collect();
        out.push_str(&format!(
            "    {:>5.1}%  {}\n",
            pct(**count, active),
            rendered.join(" → ")
        ));
    }

    // ── what to do about it ─────────────────────────────────────────
    // Findings, not just numbers: the report says what a reader should
    // take away, and says nothing when there is nothing to take.
    let mut notes: Vec<String> = Vec::new();
    let interp_share = pct(
        per_tier
            .get(&(Tier::Interpreter as u8))
            .copied()
            .unwrap_or(0),
        active,
    );
    if interp_share >= 20.0 {
        let worst: Vec<String> = rows
            .iter()
            .filter(|(_, _, tiers)| {
                // Builtins are Rust — never the answer to "why is this
                // still interpreted".
                if tiers.keys().any(|t| t & BUILTIN_BIT != 0) {
                    return false;
                }
                let interp = tiers.get(&(Tier::Interpreter as u8)).copied().unwrap_or(0);
                let total: u64 = tiers.values().sum();
                total > 0 && pct(interp, total) >= 50.0
            })
            .take(3)
            .map(|(id, _, _)| name_of(*id))
            .collect();
        if !worst.is_empty() {
            notes.push(format!(
                "{:.0}% of the time is on the tree-walking interpreter, mostly in {}.\n      \
                 A hot function that never promotes is usually a shape the bytecode\n      \
                 compiler declined, not a slow algorithm — worth checking before\n      \
                 optimizing the algorithm.",
                interp_share,
                worst.join(", ")
            ));
        }
    }
    if data.worker_samples > 0 {
        notes.push(format!(
            "{:.0}% of samples were on parallel worker threads. Work handed to a\n      \
             worker runs with its caller on another thread, so its frames appear\n      \
             with no call path above them — that is the parallelism working, not\n      \
             a gap in the profile.",
            pct(data.worker_samples, active)
        ));
    }
    if data.blocked > 0 {
        notes.push(format!(
            "{} thread-tick(s) were parked in a channel receive, a sleep, a join, or an
                   accept loop — waiting, not working — and are not charged to any function.
                   (Before 0.84 an actor loop parked in chan.recv read as the hottest row.)",
            data.blocked
        ));
    }
    if data.idle > 0 && pct(data.idle, data.total) >= 10.0 {
        notes.push(format!(
            "{:.0}% of ticks found no olang frame at all — that time is in builtins,\n      \
             I/O, or startup, none of which this profiler attributes to a function.",
            pct(data.idle, data.total)
        ));
    }
    if active < 200 {
        notes.push(
            "Few samples: percentages here are coarse. Profile a longer run, or\n      \
             lower --interval, before drawing conclusions from small differences."
                .to_string(),
        );
    }
    if !notes.is_empty() {
        out.push_str("\n  NOTES\n");
        for note in notes {
            out.push_str(&format!("    · {}\n", note));
        }
    }

    out.push_str(
        "\n  self = samples where the function was running · total = samples where it\n  \
         was anywhere on the stack · recursion and inlined callees fold into one frame\n",
    );
    out
}

impl Session {
    /// Stop the sampler thread and take what it collected.
    fn collect(&mut self) -> Collected {
        self.stop.store(true, Ordering::Relaxed);
        release();
        match self.handle.take() {
            Some(h) => h.join().unwrap_or_default(),
            None => Collected::default(),
        }
    }
}

// ── tier statistics (`--ovm-stats=json`, `olang repl --serve`'s `stats`) ──
//
// The question a profile answers per sample, kept per function for a
// whole run: how many calls each tier served, how much time each tier
// spent in it, and how often native code declined. Calls are counted
// exactly, on the push every tier already makes (a load and a store on
// the thread's own counters); time is sampled, as `olang profile`
// samples it, so the clock is never read per call — counting time per
// call would cost more than the native calls it measures.

static STATS: AtomicBool = AtomicBool::new(false);

/// True while tier statistics are kept.
#[inline(always)]
pub fn stats_on() -> bool {
    STATS.load(Ordering::Relaxed)
}

/// Keep tier statistics from now on (for the rest of the process).
/// Frames pushed from here are keyed by name and file, and counted.
pub fn stats_enable() {
    if !STATS.swap(true, Ordering::SeqCst) {
        retain();
    }
}

/// What the statistics' sampler has seen so far, across its sessions.
#[derive(Default)]
struct StatsAcc {
    attributed: HashMap<(u32, u8), u64>,
    inclusive: HashMap<u32, u64>,
    /// Wall time sampled and the ticks it took: ms per tick is their
    /// ratio, which stays honest when the OS oversleeps the interval.
    sampled_ms: f64,
    ticks: u64,
}

static STATS_SESSION: Mutex<Option<Session>> = Mutex::new(None);
static STATS_ACC: OnceLock<Mutex<StatsAcc>> = OnceLock::new();

fn stats_acc() -> std::sync::MutexGuard<'static, StatsAcc> {
    STATS_ACC
        .get_or_init(|| Mutex::new(StatsAcc::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Start the statistics' sampler (a run's whole length, or one REPL
/// evaluation). A sampler already running is left alone.
pub fn stats_sample_begin(interval_us: u64) {
    stats_enable();
    let mut slot = STATS_SESSION.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(start_sampler(interval_us));
    }
}

/// Stop the statistics' sampler and fold what it saw in.
pub fn stats_sample_end() {
    let session = STATS_SESSION
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    let Some(mut session) = session else {
        return;
    };
    let wall = session.started.elapsed().as_secs_f64() * 1000.0;
    let collected = session.collect();
    let mut acc = stats_acc();
    for (k, n) in collected.attributed {
        *acc.attributed.entry(k).or_insert(0) += n;
    }
    for (path, n) in &collected.stacks {
        let mut seen = std::collections::HashSet::new();
        for id in path {
            if seen.insert(*id) {
                *acc.inclusive.entry(*id).or_insert(0) += n;
            }
        }
    }
    acc.ticks += collected.total;
    acc.sampled_ms += wall;
}

/// Fold an exited thread's counters into the retired totals.
fn retire(stack: &ThreadStack) {
    let Some(counts) = stack.counts.get() else {
        return;
    };
    let mut retired = registry().retired.lock().unwrap_or_else(|e| e.into_inner());
    if retired.is_empty() {
        retired.resize(STAT_IDS * STAT_SLOTS, 0);
    }
    for (i, c) in counts.iter().enumerate() {
        retired[i] += c.load(Ordering::Relaxed);
    }
}

/// One function's statistics.
#[derive(Debug, Clone, Default)]
pub struct StatRow {
    pub name: String,
    /// The file it was declared in, when a tier said.
    pub file: Option<String>,
    /// Calls served by the interpreter, the VM, and native code.
    pub calls: [u64; 3],
    /// Native attempts that declined, and how many for each reason
    /// (`DECLINE_REASONS`).
    pub declines: u64,
    pub decline_reasons: [u64; 4],
    /// Self time (ms) on the interpreter, the VM, native code, and in
    /// builtins it called (`map`, `sort_by`, …).
    pub self_ms: [f64; 4],
    /// Time (ms) with the function anywhere on the stack.
    pub total_ms: f64,
}

/// The statistics so far. `reset` starts them again from zero (the
/// REPL asks for each evaluation's own).
pub struct StatsSnapshot {
    pub rows: Vec<StatRow>,
    /// Wall time sampled, and ms charged per sample.
    pub sampled_ms: f64,
    pub ms_per_sample: f64,
}

pub fn stats_snapshot(reset: bool) -> StatsSnapshot {
    let mut totals = vec![0u64; STAT_IDS * STAT_SLOTS];
    {
        let mut retired = registry().retired.lock().unwrap_or_else(|e| e.into_inner());
        for (i, n) in retired.iter_mut().enumerate() {
            totals[i] += *n;
            if reset {
                *n = 0;
            }
        }
    }
    if let Ok(stacks) = registry().stacks.lock() {
        for stack in stacks.iter() {
            if let Some(counts) = stack.counts.get() {
                for (i, c) in counts.iter().enumerate() {
                    totals[i] += if reset {
                        c.swap(0, Ordering::Relaxed)
                    } else {
                        c.load(Ordering::Relaxed)
                    };
                }
            }
        }
    }
    let mut acc = stats_acc();
    let ms_per_sample = if acc.ticks > 0 {
        acc.sampled_ms / acc.ticks as f64
    } else {
        0.0
    };
    let mut rows: HashMap<u32, StatRow> = HashMap::new();
    fn row(rows: &mut HashMap<u32, StatRow>, id: u32) -> &mut StatRow {
        rows.entry(id).or_insert_with(|| {
            let (name, file) = key_of(id);
            StatRow {
                name,
                file,
                ..StatRow::default()
            }
        })
    }
    for id in 0..STAT_IDS {
        let base = id * STAT_SLOTS;
        if totals[base..base + STAT_SLOTS].iter().all(|n| *n == 0) {
            continue;
        }
        let r = row(&mut rows, id as u32);
        r.calls = [totals[base], totals[base + 1], totals[base + 2]];
        r.declines = totals[base + SLOT_DECLINES];
        for k in 0..4 {
            r.decline_reasons[k] = totals[base + SLOT_DECLINES + 1 + k];
        }
    }
    for ((id, bits), n) in acc.attributed.iter() {
        let slot = if bits & BUILTIN_BIT != 0 {
            3
        } else {
            Tier::from_u8(*bits) as usize
        };
        row(&mut rows, *id).self_ms[slot] += *n as f64 * ms_per_sample;
    }
    for (id, n) in acc.inclusive.iter() {
        row(&mut rows, *id).total_ms += *n as f64 * ms_per_sample;
    }
    let sampled_ms = acc.sampled_ms;
    if reset {
        *acc = StatsAcc::default();
    }
    drop(acc);
    let mut rows: Vec<StatRow> = rows.into_values().filter(|r| !r.name.is_empty()).collect();
    rows.sort_by(|a, b| a.file.cmp(&b.file).then(a.name.cmp(&b.name)));
    StatsSnapshot {
        rows,
        sampled_ms,
        ms_per_sample,
    }
}

fn pct(n: u64, of: u64) -> f64 {
    if of == 0 {
        0.0
    } else {
        (n as f64) * 100.0 / (of as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, not two: the enabled flag is process-global, and
    /// separate tests would race under the parallel harness.
    #[test]
    fn frames_balance_and_report_and_cost_nothing_when_off() {
        // Off: pushes are refused and nothing lands on the stack.
        assert!(!enabled());
        assert!(!push("ignored", Tier::Interpreter));
        LOCAL.with(|s| assert_eq!(s.depth.load(Ordering::Relaxed), 0));
        inner_enabled_case();
    }

    fn inner_enabled_case() {
        let session = start(200);
        assert!(enabled(), "the run is active");
        // Push a small stack and keep it live across several ticks.
        assert!(push("outer", Tier::Interpreter));
        assert!(push("inner", Tier::Vm));
        std::thread::sleep(std::time::Duration::from_millis(20));
        pop();
        pop();
        let report = session.finish(std::time::Duration::from_millis(20), 10, "test.ol");
        assert!(!enabled(), "stopping clears the flag");
        assert!(report.contains("inner"), "leaf must appear: {report}");
        assert!(report.contains("vm"), "tier must appear: {report}");
        LOCAL.with(|s| assert_eq!(s.depth.load(Ordering::Relaxed), 0, "frames balanced"));
    }
}

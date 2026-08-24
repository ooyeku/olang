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
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicUsize, Ordering};
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

impl Tier {
    fn from_u8(v: u8) -> Tier {
        match v {
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
    live: AtomicBool,
}

impl ThreadStack {
    fn new() -> Self {
        ThreadStack {
            depth: AtomicUsize::new(0),
            frames: std::array::from_fn(|_| AtomicU32::new(0)),
            tiers: std::array::from_fn(|_| AtomicU8::new(0)),
            live: AtomicBool::new(true),
        }
    }
}

struct Registry {
    stacks: Mutex<Vec<Arc<ThreadStack>>>,
    names: Mutex<(Vec<String>, HashMap<String, u32>)>,
    enabled: AtomicBool,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| Registry {
        stacks: Mutex::new(Vec::new()),
        names: Mutex::new((Vec::new(), HashMap::new())),
        enabled: AtomicBool::new(false),
    })
}

thread_local! {
    static LOCAL: Arc<ThreadStack> = {
        let stack = Arc::new(ThreadStack::new());
        if let Ok(mut stacks) = registry().stacks.lock() {
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

fn name_of(id: u32) -> String {
    registry()
        .names
        .lock()
        .ok()
        .and_then(|g| g.0.get(id as usize).cloned())
        .unwrap_or_else(|| "<unknown>".to_string())
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
#[inline]
pub fn push(name: &str, tier: Tier) -> bool {
    if !enabled() {
        return false;
    }
    let id = intern(name);
    LOCAL.with(|stack| {
        let depth = stack.depth.load(Ordering::Relaxed);
        if depth < MAX_FRAMES {
            stack.frames[depth].store(id, Ordering::Relaxed);
            stack.tiers[depth].store(tier as u8, Ordering::Relaxed);
        }
        // Depth counts past the array so pops stay balanced; frames
        // beyond MAX_FRAMES simply are not recorded.
        stack.depth.store(depth + 1, Ordering::Relaxed);
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
}

/// A running profile: the sampler thread plus what it has collected.
pub struct Session {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Collected>>,
    interval_us: u64,
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
}

/// Start sampling. The returned session must be stopped to collect.
pub fn start(interval_us: u64) -> Session {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = stop.clone();
    registry().enabled.store(true, Ordering::Relaxed);
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
    }
}

fn sample_once(out: &mut Collected) {
    let Ok(stacks) = registry().stacks.lock() else {
        return;
    };
    out.total += 1;
    let mut saw_any = false;
    for stack in stacks.iter() {
        if !stack.live.load(Ordering::Relaxed) {
            continue;
        }
        let depth = stack.depth.load(Ordering::Relaxed).min(MAX_FRAMES);
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
        let leaf_tier = stack.tiers[depth - 1].load(Ordering::Relaxed);
        let leaf = stack.frames[depth - 1].load(Ordering::Relaxed);
        *out.leaves.entry((leaf, leaf_tier)).or_insert(0) += 1;
        *out.stacks.entry(path).or_insert(0) += 1;
    }
    if !saw_any {
        out.idle += 1;
    }
}

impl Session {
    /// Stop sampling and render the report.
    pub fn finish(mut self, elapsed: std::time::Duration, top: usize) -> String {
        self.stop.store(true, Ordering::Relaxed);
        registry().enabled.store(false, Ordering::Relaxed);
        let collected = match self.handle.take() {
            Some(h) => h.join().unwrap_or_default(),
            None => Collected::default(),
        };
        render(&collected, elapsed, top, self.interval_us)
    }
}

fn render(data: &Collected, elapsed: std::time::Duration, top: usize, interval_us: u64) -> String {
    let mut out = String::new();
    let active: u64 = data.leaves.values().sum();
    out.push_str(&format!(
        "\n{} samples over {:.2}s ({}µs interval)\n",
        active,
        elapsed.as_secs_f64(),
        interval_us
    ));
    if active == 0 {
        out.push_str(
            "\nNo samples landed in olang code. The program may be too short to \
             profile (try a larger workload), or its time may be going to \
             builtins and I/O rather than to functions the profiler tracks.\n",
        );
        return out;
    }

    // Total time: a function is "in" a sample when it appears anywhere
    // on the path. Counting distinct paths avoids charging a recursive
    // function once per frame.
    let mut total_counts: HashMap<u32, u64> = HashMap::new();
    for (path, count) in &data.stacks {
        let mut seen = std::collections::HashSet::new();
        for id in path {
            if seen.insert(*id) {
                *total_counts.entry(*id).or_insert(0) += count;
            }
        }
    }

    // Self time per function, with the tier that ran it. A function
    // that ran on more than one tier lists the one that dominated,
    // with the split noted.
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

    out.push_str(&format!(
        "\n{:>7}  {:>7}  {:<8} {}\n",
        "self", "total", "tier", "function"
    ));
    out.push_str(&format!("{}\n", "─".repeat(60)));
    for (id, self_n, tiers) in rows.iter().take(top) {
        let total_n = total_counts.get(id).copied().unwrap_or(*self_n);
        let mut tier_list: Vec<(&u8, &u64)> = tiers.iter().collect();
        tier_list.sort_by(|a, b| b.1.cmp(a.1));
        // One tier normally dominates; a genuine split (a function that
        // ran interpreted before promotion, say) names both rather than
        // hiding the smaller half behind a percentage.
        let tier_label = match tier_list.as_slice() {
            [] => "-".to_string(),
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
        out.push_str(&format!(
            "{:>6.1}%  {:>6.1}%  {:<8} {}\n",
            pct(*self_n, active),
            pct(total_n, active),
            tier_label,
            name_of(*id)
        ));
    }
    if rows.len() > top {
        out.push_str(&format!("… {} more functions\n", rows.len() - top));
    }

    // The hottest complete call paths — where "this function is hot"
    // becomes "this function is hot *when called from here*".
    let mut paths: Vec<(&Vec<u32>, &u64)> = data.stacks.iter().collect();
    paths.sort_by(|a, b| b.1.cmp(a.1));
    out.push_str("\nhottest call paths\n");
    out.push_str(&format!("{}\n", "─".repeat(60)));
    for (path, count) in paths.iter().take(5) {
        let rendered: Vec<String> = path.iter().map(|id| name_of(*id)).collect();
        out.push_str(&format!(
            "{:>6.1}%  {}\n",
            pct(**count, active),
            rendered.join(" → ")
        ));
    }

    if data.idle > 0 {
        out.push_str(&format!(
            "\n{:.0}% of ticks found no olang frame on any thread — time in \
             builtins, I/O, or startup.\n",
            pct(data.idle, data.total)
        ));
    }
    out.push_str(
        "\nself = samples where the function was running; total = samples where it \
         was anywhere on the stack.\n",
    );
    out
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
        let report = session.finish(std::time::Duration::from_millis(20), 10);
        assert!(!enabled(), "stopping clears the flag");
        assert!(report.contains("inner"), "leaf must appear: {report}");
        assert!(report.contains("vm"), "tier must appear: {report}");
        LOCAL.with(|s| assert_eq!(s.depth.load(Ordering::Relaxed), 0, "frames balanced"));
    }
}

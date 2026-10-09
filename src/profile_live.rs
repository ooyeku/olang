//! A live profile: the sampling profiler's aggregate and the process's
//! vital signs, written to a file while the program runs, for an editor
//! to draw as it goes (olang Studio's profiler page).
//!
//! Two ways in:
//!
//!   * `olang profile FILE --format json [--live DIR]`: profiled from the
//!     start; the final document goes to `--out` (or stdout), and with
//!     `--live` a snapshot of it to `DIR/<pid>.json` every `--live-every`
//!     milliseconds (500 by default) while the program runs.
//!   * `olang --profile-live DIR …` (or `OLANG_PROFILE_LIVE=DIR`, which
//!     reaches the children a run starts): any run, *armed*. The vital
//!     signs are written from the start; the sampler starts only while
//!     the file `DIR/attach` exists — an editor attaches to a program it
//!     started by creating it and detaches by deleting it. Until then
//!     the run pays nothing for profiling (the shadow stack is off).
//!
//! A snapshot is cumulative — the stacks so far, every point of the
//! series so far — and replaces the last one atomically (written beside
//! and renamed), so a reader never sees half a document and needs no
//! offset into a stream. The series keeps the last `KEEP_POINTS` points.
//!
//! What a point says, each cheap to read: the samples each tier took
//! since the last point, resident memory, the thread count and cpu time
//! (the kernel's), the heap and the program's share of it (the counting
//! allocator, `runtime.memory()`), the tasks counted apart, the olang
//! threads and how many are parked, and — when tier statistics are kept
//! (`--ovm-stats`) — the calls each tier served and the deopts so far.

use serde_json::{Value as J, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::profile::{LineCache, Session};

/// Points kept in the series (at 500 ms, an hour).
pub const KEEP_POINTS: usize = 7200;
/// Default milliseconds between snapshots.
pub const DEFAULT_EVERY_MS: u64 = 500;
/// The sampler's tick for an armed run once attached (µs).
pub const ATTACH_INTERVAL_US: u64 = 1000;

/// How a live profile runs.
pub struct LiveSpec {
    /// Where snapshots go (`<pid>.json`); none: kept for the final
    /// document only.
    pub dir: Option<PathBuf>,
    pub every_ms: u64,
    /// Sample from the start (`olang profile`), else only while
    /// `dir/attach` exists.
    pub from_start: bool,
    pub interval_us: u64,
    /// What ran, for the document.
    pub label: String,
}

struct State {
    session: Option<Session>,
    /// The last attached session's document, kept after a detach.
    kept: Option<J>,
    series: Vec<J>,
    lines: LineCache,
    prev_tiers: [u64; 4],
    prev_calls: Option<(u64, u64)>,
    attaches: u64,
}

struct Live {
    spec: LiveSpec,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    state: Arc<Mutex<State>>,
    started: std::time::Instant,
}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

/// Start a live profile (once a process; a second call is refused).
pub fn start(spec: LiveSpec) -> bool {
    let mut slot = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return false;
    }
    if let Some(dir) = &spec.dir {
        let _ = std::fs::create_dir_all(dir);
    }
    crate::profile::detail_enable();
    // Attached from the first call when `DIR/attach` is there already (an
    // editor's "profile this file" writes it before the child starts):
    // waiting for the first period would miss a run shorter than it.
    let attached_now = spec.from_start || attach_wanted(spec.dir.as_deref());
    let started = std::time::Instant::now();
    let mut first = State {
        session: if attached_now {
            Some(crate::profile::start(spec.interval_us))
        } else {
            None
        },
        kept: None,
        series: Vec::new(),
        lines: LineCache::default(),
        prev_tiers: [0; 4],
        prev_calls: None,
        attaches: if attached_now { 1 } else { 0 },
    };
    // the series' first point at the start, so even a short run spans its time
    point(&mut first, started);
    let state = Arc::new(Mutex::new(first));
    let stop = Arc::new(AtomicBool::new(false));
    let (st, sp) = (state.clone(), stop.clone());
    let dir = spec.dir.clone();
    let every = std::time::Duration::from_millis(spec.every_ms.max(50));
    let from_start = spec.from_start;
    let interval_us = spec.interval_us;
    let label = spec.label.clone();
    let thread = std::thread::Builder::new()
        .name("olang-profile-live".to_string())
        .spawn(move || {
            while !sp.load(Ordering::Relaxed) {
                // sleep in short steps, so finishing does not wait a period;
                // an attach (or a detach) is looked for every ATTACH_POLL_MS,
                // not only at a snapshot, so sampling starts when asked
                let until = std::time::Instant::now() + every;
                let mut polled = std::time::Instant::now();
                while std::time::Instant::now() < until && !sp.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(10).min(every));
                    if !from_start && polled.elapsed().as_millis() as u64 >= ATTACH_POLL_MS {
                        polled = std::time::Instant::now();
                        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
                        follow_attach(&mut s, dir.as_deref(), interval_us);
                    }
                }
                if sp.load(Ordering::Relaxed) {
                    break;
                }
                let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
                if !from_start {
                    follow_attach(&mut s, dir.as_deref(), interval_us);
                }
                point(&mut s, started);
                if let Some(d) = &dir {
                    let doc = document(&mut s, started, &label, false, None);
                    write_atomic(&d.join(format!("{}.json", std::process::id())), &doc);
                }
            }
        })
        .ok();
    *slot = Some(Live {
        spec,
        stop,
        thread,
        state,
        started,
    });
    true
}

/// How often an armed run looks for `DIR/attach` between snapshots (ms).
pub const ATTACH_POLL_MS: u64 = 50;

/// Whether `dir/attach` exists: an editor wants the stacks.
fn attach_wanted(dir: Option<&Path>) -> bool {
    dir.is_some_and(|d| d.join("attach").exists())
}

/// The sampler started when `dir/attach` appeared, stopped (the profile so
/// far kept) when it went.
fn follow_attach(s: &mut State, dir: Option<&Path>, interval_us: u64) {
    let want = attach_wanted(dir);
    if want && s.session.is_none() {
        s.session = Some(crate::profile::start(interval_us));
        s.prev_tiers = [0; 4];
        s.attaches += 1;
    } else if !want && let Some(session) = s.session.take() {
        let mut lines = std::mem::take(&mut s.lines);
        s.kept = Some(session.finish_json(&mut lines));
        s.lines = lines;
    }
}

/// Whether a live profile runs in this process.
pub fn running() -> bool {
    LIVE.lock().map(|l| l.is_some()).unwrap_or(false)
}

/// End the live profile: the final document (`done`), written as the
/// last snapshot too. None when none ran.
pub fn finish(exit_code: Option<i32>) -> Option<J> {
    let live = LIVE.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    live.stop.store(true, Ordering::Relaxed);
    if let Some(t) = live.thread {
        let _ = t.join();
    }
    let mut s = live.state.lock().unwrap_or_else(|e| e.into_inner());
    point(&mut s, live.started);
    if let Some(session) = s.session.take() {
        let mut lines = std::mem::take(&mut s.lines);
        s.kept = Some(session.finish_json(&mut lines));
        s.lines = lines;
    }
    let doc = document(&mut s, live.started, &live.spec.label, true, exit_code);
    if let Some(d) = &live.spec.dir {
        write_atomic(&d.join(format!("{}.json", std::process::id())), &doc);
    }
    Some(doc)
}

fn document(
    s: &mut State,
    started: std::time::Instant,
    label: &str,
    done: bool,
    exit_code: Option<i32>,
) -> J {
    let mut doc = match &s.session {
        Some(session) => {
            let mut lines = std::mem::take(&mut s.lines);
            let d = session.peek_json(&mut lines, false);
            s.lines = lines;
            d
        }
        None => s.kept.clone().unwrap_or_else(|| {
            json!({ "format": 1, "kind": "profile", "olang": crate::version::VERSION, "samples": 0, "ticks": 0, "frames": [], "stacks": [], "functions": [],
                    "tiers": { "interpreter": 0, "bytecode": 0, "native": 0, "builtin": 0 } })
        }),
    };
    doc["live"] = J::Bool(!done);
    doc["done"] = J::Bool(done);
    doc["attached"] = J::Bool(s.session.is_some());
    doc["attaches"] = J::from(s.attaches);
    doc["pid"] = J::from(std::process::id());
    doc["file"] = J::String(label.to_string());
    doc["wall_ms"] = J::from(round1(started.elapsed().as_secs_f64() * 1000.0));
    doc["series"] = J::Array(s.series.clone());
    if let Some(code) = exit_code {
        doc["exit"] = J::from(code);
    }
    doc
}

/// Add a point of the series: what each tier sampled since the last one
/// and the process's vital signs now.
fn point(s: &mut State, started: std::time::Instant) {
    let (tiers, ticks) = match &s.session {
        Some(session) => session.tier_samples(),
        None => ([0; 4], 0),
    };
    let d: Vec<u64> = (0..4)
        .map(|i| tiers[i].saturating_sub(s.prev_tiers[i]))
        .collect();
    s.prev_tiers = tiers;
    let heap = crate::memory::heap_bytes();
    let program = crate::memory::program_bytes().min(heap);
    let (rss_kb, threads) = vitals();
    let mut p = json!({
        "t_ms": round1(started.elapsed().as_secs_f64() * 1000.0),
        "samples": { "interpreter": d[0], "bytecode": d[1], "native": d[2], "builtin": d[3] },
        "ticks": ticks,
        "rss_kb": rss_kb,
        "threads": threads,
        "cpu_ms": cpu_ms(),
        "heap": heap,
        "program": program,
        "values": heap - program,
        "tasks": crate::memory::tasks().len(),
        "olang_threads": crate::profile::thread_counts().0,
        "parked": crate::profile::thread_counts().1,
    });
    if crate::profile::stats_on() {
        let (calls, deopts) = crate::profile::stats_call_totals();
        let (pc, pd) = s.prev_calls.unwrap_or((0, 0));
        p["calls"] = J::from(calls.saturating_sub(pc));
        p["deopts"] = J::from(deopts.saturating_sub(pd));
        s.prev_calls = Some((calls, deopts));
    }
    s.series.push(p);
    if s.series.len() > KEEP_POINTS {
        let extra = s.series.len() - KEEP_POINTS;
        s.series.drain(0..extra);
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn write_atomic(path: &Path, doc: &J) {
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, doc.to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Resident memory (KB) and the threads of this process, as the kernel
/// counts them.
#[cfg(target_os = "macos")]
fn vitals() -> (Option<u64>, Option<u64>) {
    // SAFETY: proc_pidinfo fills a plain C struct of the size given.
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        let n = libc::proc_pidinfo(
            std::process::id() as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if n != size {
            return (None, None);
        }
        (
            Some(info.pti_resident_size / 1024),
            Some(info.pti_threadnum.max(0) as u64),
        )
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn vitals() -> (Option<u64>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return (None, None);
    };
    let field = |k: &str| {
        text.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    (field("VmRSS:"), field("Threads:"))
}

#[cfg(not(unix))]
fn vitals() -> (Option<u64>, Option<u64>) {
    (None, None)
}

/// The cpu time this process has used (user and system), in ms.
#[cfg(unix)]
fn cpu_ms() -> Option<f64> {
    // SAFETY: getrusage fills a plain C struct.
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut u) != 0 {
            return None;
        }
        let tv = |t: libc::timeval| t.tv_sec as f64 * 1000.0 + t.tv_usec as f64 / 1000.0;
        Some(round1(tv(u.ru_utime) + tv(u.ru_stime)))
    }
}

#[cfg(not(unix))]
fn cpu_ms() -> Option<f64> {
    None
}

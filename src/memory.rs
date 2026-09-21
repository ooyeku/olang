//! Where the process's memory is: the accounting behind
//! `runtime.memory()`.
//!
//! A counting wrapper over the system allocator keeps the bytes each
//! thread has allocated and not yet freed. A spawned task and an http
//! worker claim a named slot for their life (`enter_task`); every other
//! thread takes an anonymous one on its first allocation. No two running
//! threads share a counter unless there are more threads than slots:
//! one shared by the workers of a `par_map` made every allocation a
//! contended write to the same cache line, and the interpreter ran six
//! times slower in parallel than alone. The cost is a thread-local read
//! and an uncontended atomic add per allocation.
//!
//! Attribution is by the thread that made the call: a value a task
//! allocates and the main thread later frees counts up on the task and
//! down on main. The total is exact; a task's share is "what this
//! thread allocated and did not itself free", which is the question a
//! `/profile` page asks. A named slot's balance folds into its thread's
//! anonymous slot when the task ends, so nothing is lost from the total.
//!
//! `program` is measured, not estimated: the heap's growth across each
//! outermost load (parsing the entry file, a `use` and everything it
//! pulls in). `values` is the rest of the heap.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

const SLOTS: usize = 192;
/// Slots `[0, ANONYMOUS)` are handed out round-robin to threads that
/// claimed nothing; the rest are the named ones.
const ANONYMOUS: usize = 128;
const UNSET: usize = usize::MAX;

#[repr(align(64))]
struct Slot {
    live: AtomicI64,
}

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY: Slot = Slot {
    live: AtomicI64::new(0),
};
static LIVE: [Slot; SLOTS] = [EMPTY; SLOTS];
/// Names of the claimed slots (the named range only).
static NAMES: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());
static NEXT_ANONYMOUS: AtomicU64 = AtomicU64::new(0);
static PROGRAM: AtomicI64 = AtomicI64::new(0);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REQUESTED: AtomicU64 = AtomicU64::new(0);

thread_local! {
    // Const-initialized and without a destructor, so the allocator may
    // read it at any point in a thread's life, teardown included.
    static SLOT: Cell<usize> = const { Cell::new(UNSET) };
    static LOAD_DEPTH: Cell<u32> = const { Cell::new(0) };
}

pub struct Counting;

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: Counting = Counting;

/// This thread's slot, assigned on first use. No allocation happens
/// here: the allocator calls it.
#[inline]
fn my_slot() -> usize {
    SLOT.try_with(|s| {
        let slot = s.get();
        if slot != UNSET {
            return slot;
        }
        let fresh = (NEXT_ANONYMOUS.fetch_add(1, Ordering::Relaxed) as usize) % ANONYMOUS;
        s.set(fresh);
        fresh
    })
    .unwrap_or(0)
}

#[inline]
fn count(delta: i64) {
    LIVE[my_slot()].live.fetch_add(delta, Ordering::Relaxed);
}

/// `OLANG_ALLOC_TRACE=<bytes>` in an `alloc-count` build: every
/// allocation at least that large prints its size and backtrace to
/// stderr — how a transient spike is found, which a heap snapshot cannot
/// show because the memory is already free when anyone looks.
#[cfg(feature = "alloc-count")]
fn trace_large(size: usize) {
    use std::sync::atomic::AtomicUsize;
    static THRESHOLD: AtomicUsize = AtomicUsize::new(usize::MAX - 1);
    thread_local! {
        static TRACING: Cell<bool> = const { Cell::new(false) };
    }
    // Reading the environment and printing a backtrace both allocate.
    if TRACING.try_with(|t| t.replace(true)).unwrap_or(true) {
        return;
    }
    let mut threshold = THRESHOLD.load(Ordering::Relaxed);
    if threshold == usize::MAX - 1 {
        threshold = std::env::var("OLANG_ALLOC_TRACE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(usize::MAX);
        THRESHOLD.store(threshold, Ordering::Relaxed);
    }
    if size >= threshold {
        eprintln!(
            "alloc-trace: {} bytes\n{}",
            size,
            std::backtrace::Backtrace::force_capture()
        );
    }
    let _ = TRACING.try_with(|t| t.set(false));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size() as i64);
        #[cfg(feature = "alloc-count")]
        trace_large(layout.size());
        if cfg!(feature = "alloc-count") {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            REQUESTED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size() as i64);
        if cfg!(feature = "alloc-count") {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            REQUESTED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count(-(layout.size() as i64));
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size as i64 - layout.size() as i64);
        #[cfg(feature = "alloc-count")]
        trace_large(new_size);
        if cfg!(feature = "alloc-count") {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            REQUESTED.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Bytes allocated and not yet freed, across every thread.
pub fn heap_bytes() -> u64 {
    let total: i64 = LIVE.iter().map(|s| s.live.load(Ordering::Relaxed)).sum();
    total.max(0) as u64
}

/// Bytes the loaded program holds (see the module header).
pub fn program_bytes() -> u64 {
    PROGRAM.load(Ordering::Relaxed).max(0) as u64
}

/// Allocations made and bytes requested so far — counted only under the
/// `alloc-count` feature.
pub fn allocation_stats() -> (u64, u64) {
    (
        ALLOCATIONS.load(Ordering::Relaxed),
        REQUESTED.load(Ordering::Relaxed),
    )
}

/// Counts the heap's growth across an outermost load toward `program`.
/// Nested loads (a module's own `use`s) are inside the outer measure.
pub struct LoadScope {
    before: Option<u64>,
}

pub fn load_scope() -> LoadScope {
    let depth = LOAD_DEPTH.with(|d| {
        d.set(d.get() + 1);
        d.get()
    });
    LoadScope {
        before: (depth == 1).then(heap_bytes),
    }
}

impl Drop for LoadScope {
    fn drop(&mut self) {
        LOAD_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        if let Some(before) = self.before {
            let grown = heap_bytes() as i64 - before as i64;
            if grown > 0 {
                PROGRAM.fetch_add(grown, Ordering::Relaxed);
            }
        }
    }
}

/// A thread counted apart, under `name`, until the guard drops. When
/// every named slot is taken the thread keeps its anonymous one.
pub struct TaskGuard {
    slot: usize,
    before: usize,
}

pub fn enter_task(name: &str) -> TaskGuard {
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if names.is_empty() {
        names.resize(SLOTS, None);
    }
    let before = my_slot();
    let slot = match (ANONYMOUS..SLOTS).find(|&i| names[i].is_none()) {
        Some(i) => {
            names[i] = Some(name.to_string());
            i
        }
        None => before,
    };
    drop(names);
    SLOT.with(|s| s.set(slot));
    TaskGuard { slot, before }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if self.slot == self.before {
            return;
        }
        SLOT.with(|s| s.set(self.before));
        // What the task allocated and left behind — a result it handed
        // to a joiner, a value in a channel — belongs to the rest now.
        // Folded before the slot is offered again, so the next holder
        // starts from zero.
        let left = LIVE[self.slot].live.swap(0, Ordering::Relaxed);
        LIVE[self.before].live.fetch_add(left, Ordering::Relaxed);
        let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
        names[self.slot] = None;
    }
}

/// The threads counted apart, with the bytes each holds, largest first.
pub fn tasks() -> Vec<(String, u64)> {
    let names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<(String, u64)> = names
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let name = name.as_ref()?;
            Some((
                name.clone(),
                LIVE[i].live.load(Ordering::Relaxed).max(0) as u64,
            ))
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Print the allocation counts to stderr when `OLANG_ALLOC_STATS` is set,
/// in a build with the `alloc-count` feature.
pub fn report_if_asked() {
    if cfg!(feature = "alloc-count") && std::env::var_os("OLANG_ALLOC_STATS").is_some() {
        let (n, bytes) = allocation_stats();
        eprintln!(
            "allocations: {} ({} MB requested)",
            n,
            bytes / (1024 * 1024)
        );
    }
}

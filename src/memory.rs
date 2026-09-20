//! Where the process's memory is: the accounting behind
//! `runtime.memory()`.
//!
//! A counting wrapper over the system allocator keeps one number per
//! thread that asked to be counted apart — the bytes it has allocated and
//! not yet freed — and one for everything else. A spawned task and an
//! http worker claim a slot for their life (`enter_task`); the main
//! thread and any thread that claimed nothing share slot 0. The cost is
//! a thread-local read and an uncontended atomic add per allocation.
//!
//! Attribution is by the thread that made the call: a value a task
//! allocates and the main thread later frees counts up on the task and
//! down on main. The total is exact; a task's share is "what this
//! thread allocated and did not itself free", which is the question a
//! `/profile` page asks. A slot's balance folds into slot 0 when its
//! task ends, so nothing is lost from the total.
//!
//! `program` is measured, not estimated: the heap's growth across each
//! outermost load (parsing the entry file, a `use` and everything it
//! pulls in). `values` is the rest of the heap.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

const SLOTS: usize = 128;

#[repr(align(64))]
struct Slot {
    live: AtomicI64,
}

#[allow(clippy::declare_interior_mutable_const)]
const EMPTY: Slot = Slot {
    live: AtomicI64::new(0),
};
static LIVE: [Slot; SLOTS] = [EMPTY; SLOTS];
/// Names of the claimed slots; index 0 is never claimed.
static NAMES: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());
static PROGRAM: AtomicI64 = AtomicI64::new(0);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REQUESTED: AtomicU64 = AtomicU64::new(0);

thread_local! {
    // Const-initialized and without a destructor, so the allocator may
    // read it at any point in a thread's life, teardown included.
    static SLOT: Cell<usize> = const { Cell::new(0) };
    static LOAD_DEPTH: Cell<u32> = const { Cell::new(0) };
}

pub struct Counting;

#[cfg(not(target_arch = "wasm32"))]
#[global_allocator]
static GLOBAL: Counting = Counting;

#[inline]
fn count(delta: i64) {
    let slot = SLOT.try_with(Cell::get).unwrap_or(0);
    LIVE[slot].live.fetch_add(delta, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size() as i64);
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
/// every slot is taken the thread counts with the rest, unnamed.
pub struct TaskGuard {
    slot: usize,
}

pub fn enter_task(name: &str) -> TaskGuard {
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if names.is_empty() {
        names.resize(SLOTS, None);
    }
    let free = (1..SLOTS).find(|&i| names[i].is_none());
    let slot = match free {
        Some(i) => {
            names[i] = Some(name.to_string());
            i
        }
        None => 0,
    };
    drop(names);
    SLOT.with(|s| s.set(slot));
    TaskGuard { slot }
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        if self.slot == 0 {
            return;
        }
        SLOT.with(|s| s.set(0));
        // What the task allocated and left behind — a result it handed
        // to a joiner, a value in a channel — belongs to the rest now.
        // Folded before the slot is offered again, so the next holder
        // starts from zero.
        let left = LIVE[self.slot].live.swap(0, Ordering::Relaxed);
        LIVE[0].live.fetch_add(left, Ordering::Relaxed);
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

//! A counting global allocator, behind the `alloc-count` feature: how
//! many allocations a run makes, printed at exit when
//! `OLANG_ALLOC_STATS` is set. The measurement behind the strings row
//! of the roadmap — a representation change is judged by this number
//! before it is written.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Allocations (mallocs and reallocs) and bytes requested so far.
pub fn stats() -> (u64, u64) {
    (
        ALLOCATIONS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    )
}

/// Print the counts to stderr when `OLANG_ALLOC_STATS` is set.
pub fn report_if_asked() {
    if std::env::var_os("OLANG_ALLOC_STATS").is_some() {
        let (n, bytes) = stats();
        eprintln!(
            "allocations: {} ({} MB requested)",
            n,
            bytes / (1024 * 1024)
        );
    }
}

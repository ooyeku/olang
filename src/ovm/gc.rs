//! GC coordination and allocation accounting.
//!
//! OVM values are reference-counted (`Arc` payloads in `ovm::value`), so
//! memory is reclaimed deterministically when the last reference drops —
//! there is no tracing collector. This module keeps the public surface the
//! rest of the crate depends on:
//!
//! - [`SafepointManager`]: thread coordination flags used by the execution
//!   engine and interpreter safepoint polls
//! - [`GarbageCollector`]: allocation/deallocation accounting and a no-op
//!   `force_collection` that reports stats
//!
//! The previous implementation carried a concurrent mark/sweep engine that
//! operated on an object model the allocator never actually created (it cast
//! payload pointers to `ValueHeader`s), so running it would have corrupted
//! memory. If a tracing GC is ever needed (e.g. for closure cycles), it must
//! be built on a unified header+payload object model from the start.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Garbage collection errors
#[derive(Debug, thiserror::Error)]
pub enum GcError {
    #[error("GC not running")]
    NotRunning,

    #[error("GC already running")]
    AlreadyRunning,

    #[error("Collection failed: {0}")]
    CollectionFailed(String),

    #[error("Thread error: {0}")]
    ThreadError(String),

    #[error("Safepoint timeout")]
    SafepointTimeout,

    #[error("Memory corruption detected at {address:x}")]
    MemoryCorruption { address: usize },

    #[error("Invalid object reference")]
    InvalidReference,
}

/// Garbage collection statistics
#[derive(Debug, Clone, Default)]
pub struct GcStats {
    pub collections: u64,
    pub minor_collections: u64,
    pub major_collections: u64,
    pub total_time: Duration,
    pub marking_time: Duration,
    pub sweeping_time: Duration,
    pub compaction_time: Duration,
    pub bytes_collected: u64,
    pub bytes_allocated: u64,
    pub average_pause: Duration,
    pub max_pause: Duration,
    pub min_pause: Duration,
    pub throughput: f64, // MB/s
    pub fragmentation: f64,
    pub heap_utilization: f64,
}

/// Allocation accounting for the reference-counted value model.
#[derive(Debug, Default)]
pub struct GarbageCollector {
    running: AtomicBool,
    objects_allocated: AtomicU64,
    bytes_allocated: AtomicU64,
    bytes_deallocated: AtomicU64,
    collections_requested: AtomicU64,
}

impl GarbageCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self) -> Result<(), GcError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(GcError::AlreadyRunning);
        }
        Ok(())
    }

    pub fn stop(&self) -> Result<(), GcError> {
        if !self.running.swap(false, Ordering::SeqCst) {
            return Err(GcError::NotRunning);
        }
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Reference counting reclaims memory continuously; a "collection" just
    /// snapshots the accounting so callers (REPL `:gc`, stats displays) keep
    /// working.
    pub fn force_collection(&self) -> Result<GcStats, GcError> {
        self.collections_requested.fetch_add(1, Ordering::Relaxed);
        Ok(self.stats())
    }

    pub fn stats(&self) -> GcStats {
        GcStats {
            collections: self.collections_requested.load(Ordering::Relaxed),
            bytes_allocated: self.bytes_allocated.load(Ordering::Relaxed),
            bytes_collected: self.bytes_deallocated.load(Ordering::Relaxed),
            ..GcStats::default()
        }
    }

    pub fn record_allocation(&self, size: usize) {
        self.objects_allocated.fetch_add(1, Ordering::Relaxed);
        self.bytes_allocated
            .fetch_add(size as u64, Ordering::Relaxed);
    }

    pub fn record_deallocation(&self, size: usize) {
        self.bytes_deallocated
            .fetch_add(size as u64, Ordering::Relaxed);
    }

    /// (objects allocated, bytes allocated, collections requested)
    pub fn get_allocation_stats(&self) -> (usize, usize, usize) {
        (
            self.objects_allocated.load(Ordering::Relaxed) as usize,
            self.bytes_allocated.load(Ordering::Relaxed) as usize,
            self.collections_requested.load(Ordering::Relaxed) as usize,
        )
    }
}

/// Debug information about safepoint state
#[derive(Debug, Clone)]
pub struct SafepointDebugInfo {
    pub safepoint_requested: bool,
    pub threads_at_safepoint: usize,
    pub total_threads: usize,
}

/// Thread coordination for safepoint polling.
#[derive(Debug)]
pub struct SafepointManager {
    safepoint_requested: Arc<AtomicBool>,
    active_threads: Arc<AtomicUsize>,
}

impl Default for SafepointManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SafepointManager {
    pub fn new() -> Self {
        Self {
            safepoint_requested: Arc::new(AtomicBool::new(false)),
            active_threads: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Simplified safepoint request - just set flag, no complex coordination
    pub fn request_safepoint(&self) -> Result<(), GcError> {
        self.safepoint_requested.store(true, Ordering::Relaxed);

        // Brief pause to allow running threads to notice the flag
        std::thread::sleep(Duration::from_millis(1));

        Ok(())
    }

    /// Release safepoint flag
    pub fn release_safepoint(&self) {
        self.safepoint_requested.store(false, Ordering::Relaxed);
    }

    /// Simplified safepoint poll - just check flag
    pub fn safepoint_poll(&self) -> Result<(), GcError> {
        if self.safepoint_requested.load(Ordering::Relaxed) {
            // Brief pause to allow coordination to proceed
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    /// Register thread (simplified tracking)
    pub fn register_thread(&self) {
        self.active_threads.fetch_add(1, Ordering::Relaxed);
    }

    /// Unregister thread (simplified tracking)
    pub fn unregister_thread(&self) {
        self.active_threads.fetch_sub(1, Ordering::Relaxed);
    }

    /// Check safepoint before allocation
    pub fn check_safepoint(&self) {
        let _ = self.safepoint_poll();
    }

    /// Record allocation (values are refcounted; nothing to track here)
    pub fn record_allocation(&self, _size: usize) {}

    /// Whether a collection pass has been explicitly requested
    pub fn should_collect(&self) -> bool {
        self.safepoint_requested.load(Ordering::Relaxed)
    }

    /// Request GC collection (simplified)
    pub fn request_collection(&self) {
        let _ = self.request_safepoint();
    }

    /// Get simplified debug info
    pub fn get_debug_info(&self) -> SafepointDebugInfo {
        SafepointDebugInfo {
            safepoint_requested: self.safepoint_requested.load(Ordering::Relaxed),
            threads_at_safepoint: 0, // Simplified - not tracked
            total_threads: self.active_threads.load(Ordering::Relaxed),
        }
    }

    /// Check if safepoint is currently requested
    pub fn is_safepoint_requested(&self) -> bool {
        self.safepoint_requested.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_stop_lifecycle() {
        let gc = GarbageCollector::new();
        assert!(gc.start().is_ok());
        assert!(matches!(gc.start(), Err(GcError::AlreadyRunning)));
        assert!(gc.stop().is_ok());
        assert!(matches!(gc.stop(), Err(GcError::NotRunning)));
    }

    #[test]
    fn allocation_accounting() {
        let gc = GarbageCollector::new();
        gc.record_allocation(100);
        gc.record_allocation(50);
        gc.record_deallocation(30);
        let (objects, bytes, _) = gc.get_allocation_stats();
        assert_eq!(objects, 2);
        assert_eq!(bytes, 150);
        let stats = gc.force_collection().unwrap();
        assert_eq!(stats.bytes_collected, 30);
        assert_eq!(stats.collections, 1);
    }

    #[test]
    fn safepoint_flags() {
        let sp = SafepointManager::new();
        assert!(!sp.is_safepoint_requested());
        sp.request_safepoint().unwrap();
        assert!(sp.is_safepoint_requested());
        assert!(sp.should_collect());
        sp.release_safepoint();
        assert!(!sp.is_safepoint_requested());
    }
}

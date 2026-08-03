//! OVM memory accounting.
//!
//! Values are reference-counted (`Arc` payloads in `ovm::value`), so the
//! process allocator and refcounts do the real memory management. This
//! module keeps the `MemoryManager` API the VM and integration layer use —
//! lifecycle, stats, and pressure heuristics — as plain accounting.
//!
//! The previous implementation was a region/TLAB bump allocator whose sweep
//! path pushed interior bump pointers onto a global free list (double-use of
//! memory if ever exercised) and whose `allocate_object` had no callers. A
//! real custom heap should only come back together with a unified object
//! model for a tracing GC.

use crate::ovm::config::OvmConfig;
use crate::ovm::gc::{GarbageCollector, GcStats};

/// Main memory manager for the OVM
#[derive(Debug)]
pub struct MemoryManager {
    heap_size_limit: u64,
    gc: GarbageCollector,
}

/// Memory statistics
#[derive(Debug, Clone, Default)]
pub struct MemoryStats {
    pub heap_size: u64,
    pub heap_used: u64,
    pub heap_free: u64,
    pub allocation_rate: f64,
    pub gc_pressure: f64,
    pub fragmentation: f64,
    pub regions_allocated: usize,
    pub objects_allocated: u64,
}

/// Memory management errors
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("Out of memory")]
    OutOfMemory,

    #[error("Invalid allocation size: {size}")]
    InvalidSize { size: usize },

    #[error("GC error: {0}")]
    GcError(#[from] crate::ovm::gc::GcError),

    #[error("Heap corruption detected")]
    HeapCorruption,

    #[error("Region allocation failed")]
    RegionAllocationFailed,
}

impl MemoryManager {
    pub fn new(config: &OvmConfig) -> Result<Self, MemoryError> {
        Ok(Self {
            heap_size_limit: config.memory.heap_size.unwrap_or(0) as u64,
            gc: GarbageCollector::new(),
        })
    }

    pub fn start_gc(&mut self) -> Result<(), MemoryError> {
        self.gc.start().map_err(MemoryError::from)
    }

    pub fn stop_gc(&mut self) -> Result<(), MemoryError> {
        self.gc.stop().map_err(MemoryError::from)
    }

    pub fn force_collection(&mut self) -> Result<GcStats, MemoryError> {
        self.gc.force_collection().map_err(MemoryError::from)
    }

    /// Record an allocation for accounting purposes.
    pub fn record_allocation(&self, size: usize) {
        self.gc.record_allocation(size);
    }

    /// Record a deallocation for accounting purposes.
    pub fn record_deallocation(&self, size: usize) {
        self.gc.record_deallocation(size);
    }

    pub fn get_stats(&self) -> Result<MemoryStats, MemoryError> {
        let (objects, bytes, _collections) = self.gc.get_allocation_stats();
        let gc_stats = self.gc.stats();
        let live = bytes as u64 - gc_stats.bytes_collected.min(bytes as u64);
        Ok(MemoryStats {
            heap_size: self.heap_size_limit,
            heap_used: live,
            heap_free: self.heap_size_limit.saturating_sub(live),
            allocation_rate: 0.0,
            gc_pressure: if self.heap_size_limit > 0 {
                live as f64 / self.heap_size_limit as f64
            } else {
                0.0
            },
            fragmentation: 0.0,
            regions_allocated: 0,
            objects_allocated: objects as u64,
        })
    }

    pub fn is_under_pressure(&self) -> bool {
        self.get_stats()
            .map(|s| s.gc_pressure > 0.9)
            .unwrap_or(false)
    }

    pub fn emergency_cleanup(&mut self) -> Result<(), MemoryError> {
        // Refcounting frees continuously; nothing extra to release here.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_and_stats() {
        let config = OvmConfig::default();
        let mut mm = MemoryManager::new(&config).unwrap();
        assert!(mm.start_gc().is_ok());
        assert!(mm.start_gc().is_err()); // double start reports AlreadyRunning
        mm.record_allocation(1024);
        let stats = mm.get_stats().unwrap();
        assert_eq!(stats.objects_allocated, 1);
        assert!(stats.heap_used >= 1024);
        assert!(mm.force_collection().is_ok());
        assert!(mm.stop_gc().is_ok());
    }
}

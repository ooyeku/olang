//! OVM Memory Management System
//!
//! Provides unified memory management with garbage collection, allocation strategies,
//! and lazy evaluation integration.

use std::collections::HashMap;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::ptr::NonNull;

use crate::ovm::config::{MemoryConfig, OvmConfig};
use crate::ovm::gc::{GarbageCollector, GcStats};
use crate::ovm::value::{GcPtr, ValueHeader};

/// Main memory manager for the OVM
pub struct MemoryManager {
    config: MemoryConfig,
    heap: UnifiedHeap,
    allocator: TieredAllocator,
    gc: GarbageCollector,
    stats: Arc<Mutex<MemoryStats>>,
}

/// Unified heap structure with actual memory regions
pub struct UnifiedHeap {
    nursery: NurserySpace,
    young_gen: YoungGeneration,
    old_gen: OldGeneration,
    large_objects: LargeObjectSpace,
    code_space: CodeSpace,
    lazy_space: LazySpace,
    regions: RwLock<Vec<Arc<HeapRegion>>>,
}

/// Heap region with bump pointer allocation
pub struct HeapRegion {
    start: *mut u8,
    end: *mut u8,
    current: AtomicPtr<u8>,
    objects: RwLock<Vec<GcPtr<ValueHeader>>>,
    generation: Generation,
    region_id: usize,
}

/// Generation for generational GC
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generation {
    Nursery,
    Young,
    Old,
    Large,
}

/// Tiered allocation strategy with actual implementation
pub struct TieredAllocator {
    tlab_manager: TlabManager,
    lockfree_allocator: LockFreeAllocator,
    global_allocator: GlobalAllocator,
    region_allocator: RegionAllocator,
}

/// Region-based allocator
pub struct RegionAllocator {
    current_region: Arc<Mutex<Option<Arc<HeapRegion>>>>,
    region_size: usize,
    available_regions: Mutex<Vec<Arc<HeapRegion>>>,
}

/// Memory statistics
#[derive(Debug, Clone)]
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

    #[error("Invalid heap region")]
    InvalidRegion,
}

// Heap space implementations with actual memory management
pub struct NurserySpace {
    size: usize,
    used: AtomicUsize,
    region: Option<Arc<HeapRegion>>,
}

pub struct YoungGeneration {
    size: usize,
    used: AtomicUsize,
    regions: RwLock<Vec<Arc<HeapRegion>>>,
}

pub struct OldGeneration {
    size: usize,
    used: AtomicUsize,
    regions: RwLock<Vec<Arc<HeapRegion>>>,
}

pub struct LargeObjectSpace {
    objects: RwLock<Vec<(*mut u8, usize)>>, // (ptr, size) pairs
    total_size: AtomicUsize,
}

pub struct CodeSpace {
    size: usize,
    used: AtomicUsize,
    region: Option<Arc<HeapRegion>>,
}

pub struct LazySpace {
    size: usize,
    used: AtomicUsize,
    region: Option<Arc<HeapRegion>>,
}

pub struct TlabManager {
    tlabs: HashMap<std::thread::ThreadId, ThreadLocalBuffer>,
    tlab_size: usize,
}

pub struct LockFreeAllocator {
    bump_pointer: AtomicPtr<u8>,
    limit: AtomicPtr<u8>,
}

pub struct GlobalAllocator {
    heap_lock: Mutex<()>,
    free_list: Mutex<Vec<(*mut u8, usize)>>,
}

pub struct ThreadLocalBuffer {
    buffer: *mut u8,
    size: usize,
    position: AtomicUsize,
}

impl Drop for MemoryManager {
    fn drop(&mut self) {
        // Ensure GC is properly shut down
        let _ = self.stop_gc();
    }
}

impl MemoryManager {
    pub fn new(config: &OvmConfig) -> Result<Self, MemoryError> {
        let heap = UnifiedHeap::new(&config.memory)?;
        let allocator = TieredAllocator::new(&config.memory)?;
        let mut gc = GarbageCollector::new(&config.memory)?;
        let stats = Arc::new(Mutex::new(MemoryStats::new()));

        let memory_manager = Self {
            config: config.memory.clone(),
            heap,
            allocator,
            gc,
            stats,
        };

        // Note: Don't start GC automatically in constructor
        // It will be started explicitly by the caller when ready

        Ok(memory_manager)
    }

    /// Fast path allocation with bump pointer
    pub fn allocate(&mut self, size: usize) -> Result<*mut u8, MemoryError> {
        if size == 0 {
            return Err(MemoryError::InvalidSize { size });
        }

        // Safety check for extremely large allocations
        if size > 1024 * 1024 * 1024 {  // 1GB limit
            return Err(MemoryError::InvalidSize { size });
        }

        // Record allocation for GC triggering
        self.gc.record_allocation(size);

        // Try fast allocation paths with safety checks
        if let Some(ptr) = self.allocator.try_tlab_allocate(size) {
            if !ptr.is_null() {
                self.update_stats_allocated(size);
                return Ok(ptr);
            }
        }

        if let Some(ptr) = self.allocator.try_region_allocate(size) {
            if !ptr.is_null() {
                self.update_stats_allocated(size);
                return Ok(ptr);
            }
        }

        // Slow path: potential GC trigger
        self.slow_allocate(size)
    }

    /// Allocate a GC-managed object with proper header
    pub fn allocate_object<T>(&mut self, data: T, type_tag: crate::ovm::value::TypeTag) -> Result<GcPtr<ValueHeader>, MemoryError> {
        let total_size = std::mem::size_of::<ValueHeader>() + std::mem::size_of::<T>();
        let ptr = self.allocate(total_size)?;

        unsafe {
            // Initialize header
            let header_ptr = ptr as *mut ValueHeader;
            let header = ValueHeader::new(
                type_tag,
                crate::ovm::value::ExecutionTier::Interpreter,
                crate::ovm::value::LazyState::Eager,
            );
            std::ptr::write(header_ptr, header);
            (*header_ptr).size = total_size as u32;

            // Initialize data after header
            let data_ptr = header_ptr.add(1) as *mut T;
            std::ptr::write(data_ptr, data);

            let gc_ptr = GcPtr::new(header_ptr);
            
            // Register with appropriate heap region
            self.register_object_with_region(gc_ptr.clone())?;
            
            Ok(gc_ptr)
        }
    }

    /// Deallocate memory (called by GC sweeper)
    pub unsafe fn deallocate(&mut self, ptr: *mut u8, size: usize) {
        self.allocator.deallocate(ptr, size);
        self.update_stats_freed(size);
    }

    /// Get all allocated objects for GC root scanning
    pub fn get_all_objects(&self) -> Vec<GcPtr<ValueHeader>> {
        let mut objects = Vec::new();
        
        if let Ok(regions) = self.heap.regions.read() {
            for region in regions.iter() {
                if let Ok(region_objects) = region.objects.read() {
                    objects.extend(region_objects.iter().cloned());
                }
            }
        }
        
        objects
    }

    fn slow_allocate(&mut self, size: usize) -> Result<*mut u8, MemoryError> {
        // Check if GC should be triggered
        if self.gc.should_collect() {
            let _ = self.gc.force_collection();
        }

        // Try allocation again after potential GC
        if let Some(ptr) = self.allocator.try_region_allocate(size) {
            self.update_stats_allocated(size);
            return Ok(ptr);
        }

        // Try global allocator as last resort
        self.allocator.global_allocate(size, &mut self.gc)
    }

    fn register_object_with_region(&mut self, gc_ptr: GcPtr<ValueHeader>) -> Result<(), MemoryError> {
        let ptr = gc_ptr.as_ptr() as *mut u8;
        
        if let Ok(regions) = self.heap.regions.read() {
            for region in regions.iter() {
                if ptr >= region.start && ptr < region.end {
                    if let Ok(mut objects) = region.objects.write() {
                        objects.push(gc_ptr);
                        return Ok(());
                    }
                }
            }
        }
        
        Err(MemoryError::InvalidRegion)
    }

    fn update_stats_allocated(&self, size: usize) {
        if let Ok(mut stats) = self.stats.lock() {
            stats.heap_used += size as u64;
            stats.objects_allocated += 1;
        }
    }

    fn update_stats_freed(&self, size: usize) {
        if let Ok(mut stats) = self.stats.lock() {
            stats.heap_used = stats.heap_used.saturating_sub(size as u64);
            stats.heap_free += size as u64;
        }
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

    pub fn get_stats(&self) -> Result<MemoryStats, MemoryError> {
        self.stats.lock().map(|stats| stats.clone()).map_err(|_| {
            MemoryError::GcError(crate::ovm::gc::GcError::CollectionFailed(
                "Failed to lock stats".to_string(),
            ))
        })
    }
}

impl HeapRegion {
    pub fn new(size: usize, generation: Generation, region_id: usize) -> Result<Arc<Self>, MemoryError> {
        let layout = std::alloc::Layout::from_size_align(size, 8)
            .map_err(|_| MemoryError::RegionAllocationFailed)?;
        
        unsafe {
            let ptr = std::alloc::alloc(layout);
            if ptr.is_null() {
                return Err(MemoryError::OutOfMemory);
            }

            let region = Arc::new(HeapRegion {
                start: ptr,
                end: ptr.add(size),
                current: AtomicPtr::new(ptr),
                objects: RwLock::new(Vec::new()),
                generation,
                region_id,
            });

            Ok(region)
        }
    }

    /// Bump pointer allocation within region
    pub fn allocate(&self, size: usize) -> Option<*mut u8> {
        if size == 0 {
            return None;
        }
        
        // Safety: limit allocation size to prevent overflow issues
        if size > 64 * 1024 * 1024 {  // 64MB max allocation per region
            return None;
        }
        
        let aligned_size = (size + 7) & !7; // 8-byte alignment
        
        // Verify aligned_size didn't overflow
        if aligned_size < size {
            return None;
        }
        
        loop {
            let current = self.current.load(Ordering::Relaxed);
            
            // Validate current pointer is within bounds
            if current < self.start || current >= self.end {
                return None;
            }
            
            // Safety check: ensure we don't overflow pointer arithmetic
            let remaining = unsafe { self.end.offset_from(current) } as usize;
            if aligned_size > remaining {
                return None; // Region full
            }
            
            let new_ptr = unsafe { current.add(aligned_size) };
            
            // Double-check bounds
            if new_ptr > self.end {
                return None;
            }
            
            match self.current.compare_exchange_weak(
                current, 
                new_ptr, 
                Ordering::Relaxed, 
                Ordering::Relaxed
            ) {
                Ok(_) => return Some(current),
                Err(_) => continue, // Retry on contention
            }
        }
    }

    pub fn reset(&self) {
        self.current.store(self.start, Ordering::Relaxed);
        if let Ok(mut objects) = self.objects.write() {
            objects.clear();
        }
    }

    pub fn usage(&self) -> f64 {
        let current = self.current.load(Ordering::Relaxed);
        let used = unsafe { current.offset_from(self.start) } as usize;
        let total = unsafe { self.end.offset_from(self.start) } as usize;
        
        if total == 0 {
            0.0
        } else {
            used as f64 / total as f64
        }
    }
}

impl Drop for HeapRegion {
    fn drop(&mut self) {
        unsafe {
            let size = self.end.offset_from(self.start) as usize;
            let layout = std::alloc::Layout::from_size_align_unchecked(size, 8);
            std::alloc::dealloc(self.start, layout);
        }
    }
}

impl UnifiedHeap {
    fn new(config: &MemoryConfig) -> Result<Self, MemoryError> {
        let mut regions = Vec::new();
        
        // Create initial regions
        let nursery_region = HeapRegion::new(config.nursery_size, Generation::Nursery, 0)?;
        let young_region = HeapRegion::new(config.young_gen_size, Generation::Young, 1)?;
        let old_region = HeapRegion::new(config.young_gen_size * 4, Generation::Old, 2)?;
        
        regions.push(nursery_region.clone());
        regions.push(young_region.clone());
        regions.push(old_region.clone());

        Ok(Self {
            nursery: NurserySpace::new(config.nursery_size, Some(nursery_region)),
            young_gen: YoungGeneration::new(config.young_gen_size),
            old_gen: OldGeneration::new(),
            large_objects: LargeObjectSpace::new(),
            code_space: CodeSpace::new(),
            lazy_space: LazySpace::new(),
            regions: RwLock::new(regions),
        })
    }
}

impl TieredAllocator {
    fn new(config: &MemoryConfig) -> Result<Self, MemoryError> {
        Ok(Self {
            tlab_manager: TlabManager::new(config)?,
            lockfree_allocator: LockFreeAllocator::new(config)?,
            global_allocator: GlobalAllocator::new(config)?,
            region_allocator: RegionAllocator::new(1024 * 1024), // 1MB regions
        })
    }

    fn try_tlab_allocate(&mut self, size: usize) -> Option<*mut u8> {
        self.tlab_manager.allocate(size)
    }

    fn try_region_allocate(&mut self, size: usize) -> Option<*mut u8> {
        self.region_allocator.allocate(size)
    }

    fn global_allocate(
        &mut self,
        size: usize,
        gc: &mut GarbageCollector,
    ) -> Result<*mut u8, MemoryError> {
        // Try global allocation first
        if let Some(ptr) = self.global_allocator.allocate(size) {
            return Ok(ptr);
        }

        // Trigger GC and try again
        gc.force_collection()?;

        if let Some(ptr) = self.global_allocator.allocate(size) {
            Ok(ptr)
        } else {
            Err(MemoryError::OutOfMemory)
        }
    }

    unsafe fn deallocate(&mut self, ptr: *mut u8, size: usize) {
        self.global_allocator.deallocate(ptr, size);
    }
}

impl RegionAllocator {
    fn new(region_size: usize) -> Self {
        Self {
            current_region: Arc::new(Mutex::new(None)),
            region_size,
            available_regions: Mutex::new(Vec::new()),
        }
    }

    fn allocate(&self, size: usize) -> Option<*mut u8> {
        // Try current region first
        if let Ok(region_guard) = self.current_region.lock() {
            if let Some(region) = region_guard.as_ref() {
                if let Some(ptr) = region.allocate(size) {
                    return Some(ptr);
                }
            }
        }

        // Try to get a new region
        self.get_new_region_and_allocate(size)
    }

    fn get_new_region_and_allocate(&self, size: usize) -> Option<*mut u8> {
        // Try to get an available region first
        if let Ok(mut available) = self.available_regions.lock() {
            if let Some(region) = available.pop() {
                // Store the Arc safely
                if let Ok(mut current) = self.current_region.lock() {
                    *current = Some(region.clone());
                    return region.allocate(size);
                }
            }
        }

        // Create new region if needed
        if let Ok(new_region) = HeapRegion::new(self.region_size, Generation::Young, 0) {
            // Store the Arc safely
            if let Ok(mut current) = self.current_region.lock() {
                *current = Some(new_region.clone());
                return new_region.allocate(size);
            }
        }

        None
    }
}

impl MemoryStats {
    fn new() -> Self {
        Self {
            heap_size: 0,
            heap_used: 0,
            heap_free: 0,
            allocation_rate: 0.0,
            gc_pressure: 0.0,
            fragmentation: 0.0,
            regions_allocated: 0,
            objects_allocated: 0,
        }
    }

    pub fn update_allocation_rate(&mut self, bytes_per_second: f64) {
        self.allocation_rate = bytes_per_second;
    }

    pub fn update_fragmentation(&mut self, fragmentation: f64) {
        self.fragmentation = fragmentation;
    }

    pub fn update_gc_pressure(&mut self, pressure: f64) {
        self.gc_pressure = pressure;
    }
}

// Placeholder implementations for heap spaces
impl NurserySpace {
    fn new(size: usize, region: Option<Arc<HeapRegion>>) -> Self {
        Self {
            size,
            used: AtomicUsize::new(0),
            region,
        }
    }

    pub fn allocate(&self, size: usize) -> Option<*mut u8> {
        if let Some(region) = &self.region {
            region.allocate(size)
        } else {
            None
        }
    }

    pub fn collect(&self) -> usize {
        let collected = self.used.load(Ordering::Relaxed);
        self.used.store(0, Ordering::Relaxed);
        
        if let Some(region) = &self.region {
            region.reset();
        }
        
        collected
    }

    pub fn usage(&self) -> f64 {
        if let Some(region) = &self.region {
            region.usage()
        } else if self.size == 0 {
            0.0
        } else {
            self.used.load(Ordering::Relaxed) as f64 / self.size as f64
        }
    }
}

impl YoungGeneration {
    fn new(size: usize) -> Self {
        Self {
            size,
            used: AtomicUsize::new(0),
            regions: RwLock::new(Vec::new()),
        }
    }

    pub fn allocate(&self, size: usize) -> Option<*mut u8> {
        if let Ok(regions) = self.regions.read() {
            for region in regions.iter() {
                if let Some(ptr) = region.allocate(size) {
                    return Some(ptr);
                }
            }
        }
        None
    }

    pub fn collect(&self) -> usize {
        let collected = self.used.load(Ordering::Relaxed);
        self.used.store(0, Ordering::Relaxed);
        
        if let Ok(regions) = self.regions.read() {
            for region in regions.iter() {
                region.reset();
            }
        }
        
        collected
    }

    pub fn usage(&self) -> f64 {
        if let Ok(regions) = self.regions.read() {
            if regions.is_empty() {
                return 0.0;
            }
            
            let total_usage: f64 = regions.iter().map(|r| r.usage()).sum();
            total_usage / regions.len() as f64
        } else {
            0.0
        }
    }
}

impl OldGeneration {
    fn new() -> Self {
        Self {
            size: 64 * 1024 * 1024, // 64MB default
            used: AtomicUsize::new(0),
            regions: RwLock::new(Vec::new()),
        }
    }

    pub fn allocate(&self, size: usize) -> Option<*mut u8> {
        if let Ok(regions) = self.regions.read() {
            for region in regions.iter() {
                if let Some(ptr) = region.allocate(size) {
                    return Some(ptr);
                }
            }
        }
        None
    }

    pub fn collect(&self) -> usize {
        let collected = self.used.load(Ordering::Relaxed);
        self.used.store(0, Ordering::Relaxed);
        
        if let Ok(regions) = self.regions.read() {
            for region in regions.iter() {
                region.reset();
            }
        }
        
        collected
    }

    pub fn usage(&self) -> f64 {
        if let Ok(regions) = self.regions.read() {
            if regions.is_empty() {
                return 0.0;
            }
            
            let total_usage: f64 = regions.iter().map(|r| r.usage()).sum();
            total_usage / regions.len() as f64
        } else {
            0.0
        }
    }
}

impl LargeObjectSpace {
    fn new() -> Self {
        Self {
            objects: RwLock::new(Vec::new()),
            total_size: AtomicUsize::new(0),
        }
    }

    pub fn allocate(&self, size: usize) -> Option<*mut u8> {
        if size > 32 * 1024 {
            // Objects larger than 32KB go here
            let layout = std::alloc::Layout::from_size_align(size, 8).ok()?;
            unsafe {
                let ptr = std::alloc::alloc(layout);
                if !ptr.is_null() {
                    if let Ok(mut objects) = self.objects.write() {
                        objects.push((ptr, size));
                        self.total_size.fetch_add(size, Ordering::Relaxed);
                        Some(ptr)
                    } else {
                        std::alloc::dealloc(ptr, layout);
                        None
                    }
                } else {
                    None
                }
            }
        } else {
            None
        }
    }

    pub fn deallocate(&self, ptr: *mut u8) -> bool {
        if let Ok(mut objects) = self.objects.write() {
            if let Some(pos) = objects.iter().position(|&(p, _)| p == ptr) {
                let (_, size) = objects.remove(pos);
                self.total_size.fetch_sub(size, Ordering::Relaxed);
                
                unsafe {
                    let layout = std::alloc::Layout::from_size_align_unchecked(size, 8);
                    std::alloc::dealloc(ptr, layout);
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn object_count(&self) -> usize {
        self.objects.read().map(|objects| objects.len()).unwrap_or(0)
    }

    pub fn total_size(&self) -> usize {
        self.total_size.load(Ordering::Relaxed)
    }
}

impl CodeSpace {
    fn new() -> Self {
        Self {
            size: 16 * 1024 * 1024, // 16MB for JIT code
            used: AtomicUsize::new(0),
            region: None,
        }
    }

    pub fn allocate(&self, size: usize) -> Option<*mut u8> {
        if let Some(region) = &self.region {
            region.allocate(size)
        } else {
            None
        }
    }

    pub fn usage(&self) -> f64 {
        if let Some(region) = &self.region {
            region.usage()
        } else if self.size == 0 {
            0.0
        } else {
            self.used.load(Ordering::Relaxed) as f64 / self.size as f64
        }
    }
}

impl LazySpace {
    fn new() -> Self {
        Self {
            size: 32 * 1024 * 1024, // 32MB for lazy values
            used: AtomicUsize::new(0),
            region: None,
        }
    }

    pub fn allocate(&self, size: usize) -> Option<*mut u8> {
        if let Some(region) = &self.region {
            region.allocate(size)
        } else {
            None
        }
    }

    pub fn usage(&self) -> f64 {
        if let Some(region) = &self.region {
            region.usage()
        } else if self.size == 0 {
            0.0
        } else {
            self.used.load(Ordering::Relaxed) as f64 / self.size as f64
        }
    }
}

impl TlabManager {
    fn new(config: &MemoryConfig) -> Result<Self, MemoryError> {
        Ok(Self {
            tlabs: HashMap::new(),
            tlab_size: config.tlab_size,
        })
    }

    fn allocate(&mut self, size: usize) -> Option<*mut u8> {
        let thread_id = std::thread::current().id();
        
        if let Some(tlab) = self.tlabs.get(&thread_id) {
            tlab.allocate(size)
        } else {
            // Create new TLAB for this thread
            if let Some(new_tlab) = ThreadLocalBuffer::new(self.tlab_size) {
                let ptr = new_tlab.allocate(size);
                self.tlabs.insert(thread_id, new_tlab);
                ptr
            } else {
                None
            }
        }
    }
}

impl ThreadLocalBuffer {
    fn new(size: usize) -> Option<Self> {
        let layout = std::alloc::Layout::from_size_align(size, 8).ok()?;
        unsafe {
            let buffer = std::alloc::alloc(layout);
            if !buffer.is_null() {
                Some(Self {
                    buffer,
                    size,
                    position: AtomicUsize::new(0),
                })
            } else {
                None
            }
        }
    }

    fn allocate(&self, size: usize) -> Option<*mut u8> {
        let aligned_size = (size + 7) & !7; // 8-byte alignment
        let current = self.position.fetch_add(aligned_size, Ordering::Relaxed);
        
        if current + aligned_size <= self.size {
            unsafe { Some(self.buffer.add(current)) }
        } else {
            None
        }
    }
}

impl Drop for ThreadLocalBuffer {
    fn drop(&mut self) {
        unsafe {
            let layout = std::alloc::Layout::from_size_align_unchecked(self.size, 8);
            std::alloc::dealloc(self.buffer, layout);
        }
    }
}

impl LockFreeAllocator {
    fn new(_config: &MemoryConfig) -> Result<Self, MemoryError> {
        Ok(Self {
            bump_pointer: AtomicPtr::new(std::ptr::null_mut()),
            limit: AtomicPtr::new(std::ptr::null_mut()),
        })
    }

    fn allocate(&self, size: usize) -> Option<*mut u8> {
        let aligned_size = (size + 7) & !7; // 8-byte alignment
        
        loop {
            let current = self.bump_pointer.load(Ordering::Relaxed);
            let limit = self.limit.load(Ordering::Relaxed);
            
            if current.is_null() || limit.is_null() {
                return None;
            }
            
            unsafe {
                let new_ptr = current.add(aligned_size);
                if new_ptr <= limit {
                    match self.bump_pointer.compare_exchange_weak(
                        current,
                        new_ptr,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => return Some(current),
                        Err(_) => continue,
                    }
                } else {
                    return None;
                }
            }
        }
    }
}

impl GlobalAllocator {
    fn new(_config: &MemoryConfig) -> Result<Self, MemoryError> {
        Ok(Self {
            heap_lock: Mutex::new(()),
            free_list: Mutex::new(Vec::new()),
        })
    }

    fn allocate(&self, size: usize) -> Option<*mut u8> {
        let _lock = self.heap_lock.lock().ok()?;
        
        if let Ok(mut free_list) = self.free_list.lock() {
            // Try to find a suitable free block
            for (i, &(ptr, block_size)) in free_list.iter().enumerate() {
                if block_size >= size {
                    free_list.remove(i);
                    return Some(ptr);
                }
            }
        }
        
        // Allocate new memory if no free block found
        let layout = std::alloc::Layout::from_size_align(size, 8).ok()?;
        unsafe {
            let ptr = std::alloc::alloc(layout);
            if !ptr.is_null() {
                Some(ptr)
            } else {
                None
            }
        }
    }

    fn deallocate(&mut self, ptr: *mut u8, size: usize) {
        if let Ok(mut free_list) = self.free_list.lock() {
            free_list.push((ptr, size));
        }
    }
}

// Enhanced MemoryManager implementation
impl MemoryManager {
    pub fn allocate_in_nursery(&mut self, size: usize) -> Result<*mut u8, MemoryError> {
        self.heap
            .nursery
            .allocate(size)
            .ok_or(MemoryError::OutOfMemory)
    }

    pub fn allocate_in_young(&mut self, size: usize) -> Result<*mut u8, MemoryError> {
        self.heap
            .young_gen
            .allocate(size)
            .ok_or(MemoryError::OutOfMemory)
    }

    pub fn allocate_in_old(&mut self, size: usize) -> Result<*mut u8, MemoryError> {
        self.heap
            .old_gen
            .allocate(size)
            .ok_or(MemoryError::OutOfMemory)
    }

    pub fn allocate_large_object(&mut self, size: usize) -> Result<*mut u8, MemoryError> {
        self.heap
            .large_objects
            .allocate(size)
            .ok_or(MemoryError::OutOfMemory)
    }

    pub fn allocate_code(&mut self, size: usize) -> Result<*mut u8, MemoryError> {
        self.heap
            .code_space
            .allocate(size)
            .ok_or(MemoryError::OutOfMemory)
    }

    pub fn allocate_lazy(&mut self, size: usize) -> Result<*mut u8, MemoryError> {
        self.heap
            .lazy_space
            .allocate(size)
            .ok_or(MemoryError::OutOfMemory)
    }

    pub fn promote_to_old(&mut self, object: *mut u8, size: usize) -> Result<*mut u8, MemoryError> {
        // Allocate space in old generation
        let new_ptr = self.heap
            .old_gen
            .allocate(size)
            .ok_or(MemoryError::OutOfMemory)?;
        
        // Copy the object data
        unsafe {
            std::ptr::copy_nonoverlapping(object, new_ptr, size);
        }
        
        Ok(new_ptr)
    }

    pub fn collect_young_generation(&mut self) -> Result<usize, MemoryError> {
        let collected = self.heap.young_gen.collect();
        self.heap.nursery.collect();
        Ok(collected)
    }

    pub fn get_heap_usage(&self) -> HeapUsage {
        HeapUsage {
            nursery_usage: self.heap.nursery.usage(),
            young_usage: self.heap.young_gen.usage(),
            old_usage: self.heap.old_gen.usage(),
            code_usage: self.heap.code_space.usage(),
            lazy_usage: self.heap.lazy_space.usage(),
            large_object_count: self.heap.large_objects.object_count(),
        }
    }

    pub fn update_stats(&mut self) -> Result<(), MemoryError> {
        let usage = self.get_heap_usage();

        if let Ok(mut stats) = self.stats.lock() {
            stats.heap_used = (usage.nursery_usage + usage.young_usage + usage.old_usage) as u64;
            stats.heap_size = self.config.heap_size.unwrap_or(128 * 1024 * 1024) as u64;
            stats.heap_free = stats.heap_size - stats.heap_used;

            // Calculate fragmentation
            let total_usage = usage.nursery_usage + usage.young_usage + usage.old_usage;
            stats.fragmentation = if total_usage > 0.8 {
                (total_usage - 0.8) / 0.2 // Simple fragmentation metric
            } else {
                0.0
            };
        }

        Ok(())
    }
}

/// Heap usage information
#[derive(Debug, Clone)]
pub struct HeapUsage {
    pub nursery_usage: f64,
    pub young_usage: f64,
    pub old_usage: f64,
    pub code_usage: f64,
    pub lazy_usage: f64,
    pub large_object_count: usize,
}

impl HeapUsage {
    pub fn total_usage(&self) -> f64 {
        (self.nursery_usage + self.young_usage + self.old_usage + self.code_usage + self.lazy_usage)
            / 5.0
    }

    pub fn needs_collection(&self) -> bool {
        self.nursery_usage > 0.8 || self.young_usage > 0.9
    }

    pub fn needs_compaction(&self) -> bool {
        self.old_usage > 0.7 && self.large_object_count > 100
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ovm::config::OvmConfig;

    #[test]
    fn test_memory_manager_creation() {
        let config = OvmConfig::default();
        let memory_manager = MemoryManager::new(&config);
        assert!(memory_manager.is_ok());
    }

    #[test]
    fn test_allocation() {
        let config = OvmConfig::default();
        let mut memory_manager = MemoryManager::new(&config).unwrap();

        let ptr = memory_manager.allocate(1024);
        assert!(ptr.is_ok());

        // Test invalid size
        let invalid_ptr = memory_manager.allocate(0);
        assert!(invalid_ptr.is_err());
    }
}

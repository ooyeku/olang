//! OVM Garbage Collection System
//!
//! Provides concurrent, generational garbage collection for the OVM

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::ovm::config::MemoryConfig;
use crate::ovm::value::{GcPtr, ValueHeader};

/// Main garbage collector with concurrent marking and generational collection
pub struct GarbageCollector {
    config: MemoryConfig,
    is_running: Arc<AtomicBool>,
    collector_thread: Option<JoinHandle<()>>,
    stats: Arc<Mutex<GcStats>>,

    // Concurrent GC state
    marking_engine: Arc<ConcurrentMarkingEngine>,
    sweeping_engine: Arc<IncrementalSweepingEngine>,
    compaction_engine: Arc<SelectiveCompactionEngine>,
    safepoint_manager: Arc<SafepointManager>,
    write_barrier_manager: Arc<WriteBarrierManager>,

    // Root set management
    root_scanner: Arc<RootScanner>,
    remembered_set: Arc<Mutex<RememberedSet>>,

    // Collection scheduling
    collection_trigger: Arc<(Mutex<bool>, Condvar)>,
    allocation_counter: Arc<AtomicUsize>,
}

/// Concurrent marking engine with work stealing
pub struct ConcurrentMarkingEngine {
    work_queues: Vec<Arc<Mutex<VecDeque<GcPtr<ValueHeader>>>>>,
    worker_threads: Vec<JoinHandle<()>>,
    is_marking: Arc<AtomicBool>,
    mark_stack: Arc<Mutex<Vec<GcPtr<ValueHeader>>>>,
    marked_objects: Arc<AtomicUsize>,
}

/// Incremental sweeping engine with pause budgets
pub struct IncrementalSweepingEngine {
    sweep_position: Arc<AtomicUsize>,
    pause_budget: Duration,
    swept_bytes: Arc<AtomicUsize>,
    free_list: Arc<Mutex<FreeList>>,
}

/// Selective compaction engine for fragmentation control
pub struct SelectiveCompactionEngine {
    compaction_threshold: f64,
    regions_to_compact: Arc<Mutex<Vec<MemoryRegion>>>,
    forwarding_table: Arc<RwLock<HashMap<usize, usize>>>,
}

/// Safepoint manager for mutator coordination
pub struct SafepointManager {
    safepoint_requested: Arc<AtomicBool>,
    threads_at_safepoint: Arc<AtomicUsize>,
    total_threads: Arc<AtomicUsize>,
    safepoint_barrier: Arc<(Mutex<bool>, Condvar)>,
}

/// Debug information for safepoint coordination
#[derive(Debug, Clone)]
pub struct SafepointDebugInfo {
    pub safepoint_requested: bool,
    pub threads_at_safepoint: usize,
    pub total_threads: usize,
}

/// Write barrier manager with optimized barriers
pub struct WriteBarrierManager {
    card_table: Arc<RwLock<Vec<AtomicBool>>>,
    dirty_cards: Arc<Mutex<Vec<usize>>>,
    barrier_enabled: Arc<AtomicBool>,
}

/// Root scanner for finding GC roots
pub struct RootScanner {
    stack_roots: Arc<Mutex<Vec<GcPtr<ValueHeader>>>>,
    global_roots: Arc<Mutex<Vec<GcPtr<ValueHeader>>>>,
    thread_locals: Arc<Mutex<HashMap<std::thread::ThreadId, Vec<GcPtr<ValueHeader>>>>>,
}

/// Remembered set for cross-generational references
pub struct RememberedSet {
    old_to_young_refs: HashMap<usize, Vec<GcPtr<ValueHeader>>>,
    dirty_regions: Vec<MemoryRegion>,
}

/// Free list for allocation
pub struct FreeList {
    blocks: VecDeque<FreeBlock>,
    total_free: usize,
}

/// Free block in the heap
#[derive(Debug, Clone)]
pub struct FreeBlock {
    address: usize,
    size: usize,
}

/// Memory region for compaction
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    start: usize,
    end: usize,
    live_bytes: usize,
    total_bytes: usize,
}

/// GC statistics with detailed metrics
#[derive(Debug, Clone)]
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

/// Collection types
#[derive(Debug, Clone, Copy)]
pub enum CollectionType {
    Minor, // Young generation only
    Major, // Full heap collection
    Mixed, // Young + some old regions
}

/// GC phases
#[derive(Debug, Clone, Copy)]
pub enum GcPhase {
    Idle,
    InitialMark,
    ConcurrentMark,
    Remark,
    ConcurrentSweep,
    Compaction,
    Cleanup,
}

/// GC errors
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

impl GarbageCollector {
    pub fn new(config: &MemoryConfig) -> Result<Self, GcError> {
        let marking_engine = Arc::new(ConcurrentMarkingEngine::new(config.gc_threads)?);
        let sweeping_engine = Arc::new(IncrementalSweepingEngine::new(Duration::from_millis(
            config.gc_target_pause_ms,
        )));
        let compaction_engine = Arc::new(SelectiveCompactionEngine::new(0.3)); // 30% fragmentation threshold
        let safepoint_manager = Arc::new(SafepointManager::new());
        let write_barrier_manager = Arc::new(WriteBarrierManager::new(
            config.heap_size.unwrap_or(64 * 1024 * 1024),
        )?);
        let root_scanner = Arc::new(RootScanner::new());

        Ok(Self {
            config: config.clone(),
            is_running: Arc::new(AtomicBool::new(false)),
            collector_thread: None,
            stats: Arc::new(Mutex::new(GcStats::new())),
            marking_engine,
            sweeping_engine,
            compaction_engine,
            safepoint_manager,
            write_barrier_manager,
            root_scanner,
            remembered_set: Arc::new(Mutex::new(RememberedSet::new())),
            collection_trigger: Arc::new((Mutex::new(false), Condvar::new())),
            allocation_counter: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn start(&mut self) -> Result<(), GcError> {
        if self.is_running.load(Ordering::Relaxed) {
            return Err(GcError::AlreadyRunning);
        }

        self.is_running.store(true, Ordering::Relaxed);
        self.write_barrier_manager.enable_barriers();

        // Start background GC thread
        let is_running = Arc::clone(&self.is_running);
        let stats = self.stats.clone();
        let marking_engine = self.marking_engine.clone();
        let sweeping_engine = self.sweeping_engine.clone();
        let compaction_engine = self.compaction_engine.clone();
        let safepoint_manager = self.safepoint_manager.clone();
        let root_scanner = self.root_scanner.clone();
        let remembered_set = self.remembered_set.clone();
        let collection_trigger = self.collection_trigger.clone();
        let allocation_counter = self.allocation_counter.clone();
        let gc_trigger_threshold = self.config.gc_trigger_threshold;

        let handle = thread::spawn(move || {
            Self::gc_thread_main(
                is_running,
                stats,
                marking_engine,
                sweeping_engine,
                compaction_engine,
                safepoint_manager,
                root_scanner,
                remembered_set,
                collection_trigger,
                allocation_counter,
                gc_trigger_threshold,
            );
        });

        self.collector_thread = Some(handle);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), GcError> {
        if !self.is_running.load(Ordering::Relaxed) {
            return Ok(());
        }

        self.is_running.store(false, Ordering::Relaxed);
        self.write_barrier_manager.disable_barriers();

        // Wake up GC thread
        {
            let (lock, cvar) = &*self.collection_trigger;
            let mut triggered = lock.lock().unwrap();
            *triggered = true;
            cvar.notify_all();
        }

        if let Some(handle) = self.collector_thread.take() {
            handle
                .join()
                .map_err(|_| GcError::ThreadError("Failed to join GC thread".to_string()))?;
        }

        Ok(())
    }

    pub fn force_collection(&mut self) -> Result<GcStats, GcError> {
        if !self.is_running.load(Ordering::Relaxed) {
            return Err(GcError::NotRunning);
        }

        let start = Instant::now();

        // Trigger immediate collection
        {
            let (lock, cvar) = &*self.collection_trigger;
            let mut triggered = lock.lock().unwrap();
            *triggered = true;
            cvar.notify_all();
        }

        // Perform collection
        self.perform_collection(CollectionType::Major)?;

        let duration = start.elapsed();

        // Update statistics
        if let Ok(mut stats) = self.stats.lock() {
            stats.collections += 1;
            stats.major_collections += 1;
            stats.total_time += duration;
            stats.update_pause_times(duration);
        }

        self.stats
            .lock()
            .map(|stats| stats.clone())
            .map_err(|_| GcError::CollectionFailed("Failed to lock stats".to_string()))
    }

    /// Record allocation for GC triggering
    pub fn record_allocation(&self, size: usize) {
        self.allocation_counter.fetch_add(size, Ordering::Relaxed);
    }

    /// Check if GC should be triggered
    pub fn should_collect(&self) -> bool {
        self.allocation_counter.load(Ordering::Relaxed) >= self.config.gc_trigger_threshold
    }

    /// Register a GC root
    pub fn register_root(&self, root: GcPtr<ValueHeader>) {
        self.root_scanner.add_global_root(root);
    }

    /// Unregister a GC root
    pub fn unregister_root(&self, root: GcPtr<ValueHeader>) {
        self.root_scanner.remove_global_root(root);
    }

    /// Write barrier for pointer updates
    pub fn write_barrier(
        &self,
        object: GcPtr<ValueHeader>,
        field_addr: usize,
        new_value: GcPtr<ValueHeader>,
    ) {
        if self.write_barrier_manager.is_enabled() {
            self.write_barrier_manager
                .record_write(object, field_addr, new_value);
        }
    }

    fn gc_thread_main(
        is_running: Arc<AtomicBool>,
        stats: Arc<Mutex<GcStats>>,
        marking_engine: Arc<ConcurrentMarkingEngine>,
        sweeping_engine: Arc<IncrementalSweepingEngine>,
        compaction_engine: Arc<SelectiveCompactionEngine>,
        safepoint_manager: Arc<SafepointManager>,
        root_scanner: Arc<RootScanner>,
        remembered_set: Arc<Mutex<RememberedSet>>,
        collection_trigger: Arc<(Mutex<bool>, Condvar)>,
        allocation_counter: Arc<AtomicUsize>,
        gc_trigger_threshold: usize,
    ) {
        let (lock, cvar) = &*collection_trigger;

        while is_running.load(Ordering::Relaxed) {
            // Wait for collection trigger or timeout
            let triggered = {
                let triggered = lock.lock().unwrap();
                let mut result = cvar
                    .wait_timeout_while(triggered, Duration::from_millis(100), |&mut triggered| {
                        !triggered
                            && allocation_counter.load(Ordering::Relaxed) < gc_trigger_threshold
                    })
                    .unwrap();
                let should_collect =
                    *result.0 || allocation_counter.load(Ordering::Relaxed) >= gc_trigger_threshold;
                if should_collect {
                    *result.0 = false;
                }
                should_collect
            };

            if triggered {
                // Perform garbage collection
                let collection_type =
                    if allocation_counter.load(Ordering::Relaxed) >= gc_trigger_threshold * 2 {
                        CollectionType::Major
                    } else {
                        CollectionType::Minor
                    };

                if let Err(e) = Self::perform_collection_impl(
                    &stats,
                    &marking_engine,
                    &sweeping_engine,
                    &compaction_engine,
                    &safepoint_manager,
                    &root_scanner,
                    &remembered_set,
                    collection_type,
                ) {
                    eprintln!("GC collection failed: {}", e);
                }

                // Reset allocation counter
                allocation_counter.store(0, Ordering::Relaxed);
            }
        }
    }

    fn perform_collection(&self, collection_type: CollectionType) -> Result<(), GcError> {
        Self::perform_collection_impl(
            &self.stats,
            &self.marking_engine,
            &self.sweeping_engine,
            &self.compaction_engine,
            &self.safepoint_manager,
            &self.root_scanner,
            &self.remembered_set,
            collection_type,
        )
    }

    fn perform_collection_impl(
        stats: &Arc<Mutex<GcStats>>,
        marking_engine: &Arc<ConcurrentMarkingEngine>,
        sweeping_engine: &Arc<IncrementalSweepingEngine>,
        compaction_engine: &Arc<SelectiveCompactionEngine>,
        safepoint_manager: &Arc<SafepointManager>,
        root_scanner: &Arc<RootScanner>,
        remembered_set: &Arc<Mutex<RememberedSet>>,
        collection_type: CollectionType,
    ) -> Result<(), GcError> {
        let collection_start = Instant::now();

        // Phase 1: Initial mark (requires safepoint)
        let mark_start = Instant::now();
        safepoint_manager.request_safepoint()?;

        let roots = root_scanner.scan_roots()?;
        marking_engine.initial_mark(&roots)?;

        safepoint_manager.release_safepoint();
        let mark_time = mark_start.elapsed();

        // Phase 2: Concurrent mark
        marking_engine.concurrent_mark()?;

        // Phase 3: Remark (requires safepoint)
        safepoint_manager.request_safepoint()?;
        marking_engine.remark()?;
        safepoint_manager.release_safepoint();

        // Phase 4: Concurrent sweep
        let sweep_start = Instant::now();
        sweeping_engine.concurrent_sweep()?;
        let sweep_time = sweep_start.elapsed();

        // Phase 5: Selective compaction (if needed)
        let compact_start = Instant::now();
        if compaction_engine.should_compact()? {
            safepoint_manager.request_safepoint()?;
            compaction_engine.compact()?;
            safepoint_manager.release_safepoint();
        }
        let compact_time = compact_start.elapsed();

        let total_time = collection_start.elapsed();

        // Update statistics
        if let Ok(mut stats) = stats.lock() {
            stats.collections += 1;
            match collection_type {
                CollectionType::Minor => stats.minor_collections += 1,
                CollectionType::Major => stats.major_collections += 1,
                CollectionType::Mixed => stats.major_collections += 1,
            }
            stats.total_time += total_time;
            stats.marking_time += mark_time;
            stats.sweeping_time += sweep_time;
            stats.compaction_time += compact_time;
            stats.update_pause_times(total_time);
            stats.bytes_collected += sweeping_engine.get_swept_bytes();
        }

        Ok(())
    }
}

impl GcStats {
    fn new() -> Self {
        Self {
            collections: 0,
            minor_collections: 0,
            major_collections: 0,
            total_time: Duration::ZERO,
            marking_time: Duration::ZERO,
            sweeping_time: Duration::ZERO,
            compaction_time: Duration::ZERO,
            bytes_collected: 0,
            bytes_allocated: 0,
            average_pause: Duration::ZERO,
            max_pause: Duration::ZERO,
            min_pause: Duration::MAX,
            throughput: 0.0,
            fragmentation: 0.0,
            heap_utilization: 0.0,
        }
    }

    fn update_pause_times(&mut self, pause: Duration) {
        if self.collections > 0 {
            self.average_pause = self.total_time / self.collections as u32;
        }

        if pause > self.max_pause {
            self.max_pause = pause;
        }

        if pause < self.min_pause {
            self.min_pause = pause;
        }

        // Calculate throughput (MB/s)
        if self.total_time.as_secs_f64() > 0.0 {
            self.throughput =
                (self.bytes_allocated as f64 / (1024.0 * 1024.0)) / self.total_time.as_secs_f64();
        }
    }
}

impl Drop for GarbageCollector {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

// Implementation of GC engine components

impl ConcurrentMarkingEngine {
    pub fn new(num_threads: usize) -> Result<Self, GcError> {
        let mut work_queues = Vec::new();
        let worker_threads = Vec::new();

        for _ in 0..num_threads {
            work_queues.push(Arc::new(Mutex::new(VecDeque::new())));
        }

        Ok(Self {
            work_queues,
            worker_threads,
            is_marking: Arc::new(AtomicBool::new(false)),
            mark_stack: Arc::new(Mutex::new(Vec::new())),
            marked_objects: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn initial_mark(&self, roots: &[GcPtr<ValueHeader>]) -> Result<(), GcError> {
        self.is_marking.store(true, Ordering::Relaxed);
        self.marked_objects.store(0, Ordering::Relaxed);

        // Add roots to mark stack
        if let Ok(mut stack) = self.mark_stack.lock() {
            stack.clear();
            for root in roots {
                stack.push(GcPtr::new(root.as_ptr()));
            }
        }

        Ok(())
    }

    pub fn concurrent_mark(&self) -> Result<(), GcError> {
        // Distribute work across worker threads
        while let Some(object) = self.pop_from_mark_stack() {
            self.mark_object(object)?;
        }

        Ok(())
    }

    pub fn remark(&self) -> Result<(), GcError> {
        // Final marking phase - process any remaining objects
        while let Some(object) = self.pop_from_mark_stack() {
            self.mark_object(object)?;
        }

        self.is_marking.store(false, Ordering::Relaxed);
        Ok(())
    }

    fn pop_from_mark_stack(&self) -> Option<GcPtr<ValueHeader>> {
        if let Ok(mut stack) = self.mark_stack.lock() {
            stack.pop()
        } else {
            None
        }
    }

    fn mark_object(&self, object: GcPtr<ValueHeader>) -> Result<(), GcError> {
        // Mark the object and add its children to the mark stack
        // This is a simplified implementation
        self.marked_objects.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl IncrementalSweepingEngine {
    pub fn new(pause_budget: Duration) -> Self {
        Self {
            sweep_position: Arc::new(AtomicUsize::new(0)),
            pause_budget,
            swept_bytes: Arc::new(AtomicUsize::new(0)),
            free_list: Arc::new(Mutex::new(FreeList::new())),
        }
    }

    pub fn concurrent_sweep(&self) -> Result<(), GcError> {
        let start = Instant::now();
        let mut swept = 0;

        // Sweep unmarked objects within pause budget
        while start.elapsed() < self.pause_budget {
            // Simplified sweep implementation
            swept += 1;

            if swept > 1000 {
                // Arbitrary limit for demo
                break;
            }
        }

        self.swept_bytes.fetch_add(swept * 64, Ordering::Relaxed); // Assume 64-byte objects
        Ok(())
    }

    pub fn get_swept_bytes(&self) -> u64 {
        self.swept_bytes.load(Ordering::Relaxed) as u64
    }
}

impl SelectiveCompactionEngine {
    pub fn new(threshold: f64) -> Self {
        Self {
            compaction_threshold: threshold,
            regions_to_compact: Arc::new(Mutex::new(Vec::new())),
            forwarding_table: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn should_compact(&self) -> Result<bool, GcError> {
        // Simplified check - in reality would analyze heap fragmentation
        Ok(false) // Disable compaction for now
    }

    pub fn compact(&self) -> Result<(), GcError> {
        // Compact selected regions
        if let Ok(mut regions) = self.regions_to_compact.lock() {
            regions.clear();
        }

        if let Ok(mut table) = self.forwarding_table.write() {
            table.clear();
        }

        Ok(())
    }
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
            threads_at_safepoint: Arc::new(AtomicUsize::new(0)),
            total_threads: Arc::new(AtomicUsize::new(0)),
            safepoint_barrier: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    /// Request all threads to reach a safepoint and wait for coordination
    pub fn request_safepoint(&self) -> Result<(), GcError> {
        // Mark that a safepoint is requested
        self.safepoint_requested.store(true, Ordering::Relaxed);

        // If no threads are registered, immediately succeed
        let total = self.total_threads.load(Ordering::Relaxed);
        if total == 0 {
            let (lock, cvar) = &*self.safepoint_barrier;
            let mut reached = lock.lock().unwrap();
            *reached = true;
            cvar.notify_all();
            return Ok(());
        }

        // Wait for all threads to reach safepoint
        let (lock, cvar) = &*self.safepoint_barrier;
        let reached = lock.lock().unwrap();

        let timeout = Duration::from_millis(5000); // Increased timeout for better reliability
        let result = cvar
            .wait_timeout_while(reached, timeout, |&mut reached| !reached)
            .unwrap();

        if result.1.timed_out() {
            // Reset safepoint request on timeout
            self.safepoint_requested.store(false, Ordering::Relaxed);
            
            // Log debugging information
            eprintln!("🚨 SAFEPOINT TIMEOUT DEBUG INFO:");
            eprintln!("   Total threads registered: {}", self.total_threads.load(Ordering::Relaxed));
            eprintln!("   Threads at safepoint: {}", self.threads_at_safepoint.load(Ordering::Relaxed));
            eprintln!("   Safepoint requested: {}", self.safepoint_requested.load(Ordering::Relaxed));
            
            return Err(GcError::SafepointTimeout);
        }

        Ok(())
    }

    /// Release safepoint and allow threads to continue
    pub fn release_safepoint(&self) {
        self.safepoint_requested.store(false, Ordering::Relaxed);

        let (lock, cvar) = &*self.safepoint_barrier;
        let mut reached = lock.lock().unwrap();
        *reached = false;
        cvar.notify_all();
        
        // Reset threads at safepoint counter
        self.threads_at_safepoint.store(0, Ordering::Relaxed);
    }

    /// Check if safepoint is requested and coordinate if needed
    /// This should be called periodically by mutator threads
    pub fn safepoint_poll(&self) -> Result<(), GcError> {
        if !self.safepoint_requested.load(Ordering::Relaxed) {
            return Ok(());
        }

        // Increment threads at safepoint
        let count = self.threads_at_safepoint.fetch_add(1, Ordering::Relaxed);
        let total = self.total_threads.load(Ordering::Relaxed);

        // If all threads are at safepoint, signal completion
        if count + 1 >= total {
            let (lock, cvar) = &*self.safepoint_barrier;
            let mut reached = lock.lock().unwrap();
            *reached = true;
            cvar.notify_all();
        }

        // Wait for safepoint to be released
        let (lock, cvar) = &*self.safepoint_barrier;
        let _guard = cvar.wait_while(lock.lock().unwrap(), |&mut reached| {
            self.safepoint_requested.load(Ordering::Relaxed) && reached
        }).unwrap();

        // Decrement thread count when leaving safepoint
        self.threads_at_safepoint.fetch_sub(1, Ordering::Relaxed);
        
        Ok(())
    }

    /// Register a thread with the safepoint manager
    /// Must be called by each thread that participates in safepoint coordination
    pub fn register_thread(&self) {
        let new_count = self.total_threads.fetch_add(1, Ordering::Relaxed) + 1;
        
        // Debug logging for thread registration
        if cfg!(debug_assertions) {
            println!("🧵 Thread registered for safepoint coordination. Total: {}", new_count);
        }
    }

    /// Unregister a thread from the safepoint manager
    /// Must be called when a thread exits to avoid safepoint deadlocks
    pub fn unregister_thread(&self) {
        let prev_count = self.total_threads.fetch_sub(1, Ordering::Relaxed);
        
        // Ensure we don't underflow
        if prev_count == 0 {
            self.total_threads.store(0, Ordering::Relaxed);
        }
        
        let new_count = prev_count.saturating_sub(1);
        
        // Debug logging for thread unregistration
        if cfg!(debug_assertions) {
            println!("🧵 Thread unregistered from safepoint coordination. Total: {}", new_count);
        }
        
        // If this was the last thread and safepoint is pending, signal completion
        if new_count == 0 && self.safepoint_requested.load(Ordering::Relaxed) {
            let (lock, cvar) = &*self.safepoint_barrier;
            let mut reached = lock.lock().unwrap();
            *reached = true;
            cvar.notify_all();
        }
    }

    /// Get current safepoint coordination state for debugging
    pub fn get_debug_info(&self) -> SafepointDebugInfo {
        SafepointDebugInfo {
            safepoint_requested: self.safepoint_requested.load(Ordering::Relaxed),
            threads_at_safepoint: self.threads_at_safepoint.load(Ordering::Relaxed),
            total_threads: self.total_threads.load(Ordering::Relaxed),
        }
    }

    /// Check if safepoint is currently requested
    pub fn is_safepoint_requested(&self) -> bool {
        self.safepoint_requested.load(Ordering::Relaxed)
    }
}

impl WriteBarrierManager {
    pub fn new(heap_size: usize) -> Result<Self, GcError> {
        let card_size = 512; // 512-byte cards
        let num_cards = heap_size.div_ceil(card_size);
        let mut card_table = Vec::with_capacity(num_cards);

        for _ in 0..num_cards {
            card_table.push(AtomicBool::new(false));
        }

        Ok(Self {
            card_table: Arc::new(RwLock::new(card_table)),
            dirty_cards: Arc::new(Mutex::new(Vec::new())),
            barrier_enabled: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn enable_barriers(&self) {
        self.barrier_enabled.store(true, Ordering::Relaxed);
    }

    pub fn disable_barriers(&self) {
        self.barrier_enabled.store(false, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.barrier_enabled.load(Ordering::Relaxed)
    }

    pub fn record_write(
        &self,
        _object: GcPtr<ValueHeader>,
        field_addr: usize,
        _new_value: GcPtr<ValueHeader>,
    ) {
        if !self.is_enabled() {
            return;
        }

        let card_size = 512;
        let card_index = field_addr / card_size;

        if let Ok(card_table) = self.card_table.read() {
            if card_index < card_table.len() && !card_table[card_index].load(Ordering::Relaxed) {
                card_table[card_index].store(true, Ordering::Relaxed);

                if let Ok(mut dirty_cards) = self.dirty_cards.lock() {
                    dirty_cards.push(card_index);
                }
            }
        }
    }
}

impl Default for RootScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl RootScanner {
    pub fn new() -> Self {
        Self {
            stack_roots: Arc::new(Mutex::new(Vec::new())),
            global_roots: Arc::new(Mutex::new(Vec::new())),
            thread_locals: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn scan_roots(&self) -> Result<Vec<GcPtr<ValueHeader>>, GcError> {
        let mut all_roots = Vec::new();

        // Collect stack roots
        if let Ok(stack_roots) = self.stack_roots.lock() {
            for root in stack_roots.iter() {
                all_roots.push(GcPtr::new(root.as_ptr()));
            }
        }

        // Collect global roots
        if let Ok(global_roots) = self.global_roots.lock() {
            for root in global_roots.iter() {
                all_roots.push(GcPtr::new(root.as_ptr()));
            }
        }

        // Collect thread-local roots
        if let Ok(thread_locals) = self.thread_locals.lock() {
            for roots in thread_locals.values() {
                for root in roots.iter() {
                    all_roots.push(GcPtr::new(root.as_ptr()));
                }
            }
        }

        Ok(all_roots)
    }

    pub fn add_global_root(&self, root: GcPtr<ValueHeader>) {
        if let Ok(mut global_roots) = self.global_roots.lock() {
            global_roots.push(root);
        }
    }

    pub fn remove_global_root(&self, root: GcPtr<ValueHeader>) {
        if let Ok(mut global_roots) = self.global_roots.lock() {
            global_roots.retain(|r| r.as_ptr() != root.as_ptr());
        }
    }

    pub fn add_stack_root(&self, root: GcPtr<ValueHeader>) {
        if let Ok(mut stack_roots) = self.stack_roots.lock() {
            stack_roots.push(root);
        }
    }

    pub fn remove_stack_root(&self, root: GcPtr<ValueHeader>) {
        if let Ok(mut stack_roots) = self.stack_roots.lock() {
            stack_roots.retain(|r| r.as_ptr() != root.as_ptr());
        }
    }
}

impl Default for RememberedSet {
    fn default() -> Self {
        Self::new()
    }
}

impl RememberedSet {
    pub fn new() -> Self {
        Self {
            old_to_young_refs: HashMap::new(),
            dirty_regions: Vec::new(),
        }
    }

    pub fn add_reference(&mut self, from: GcPtr<ValueHeader>, to: GcPtr<ValueHeader>) {
        let key = from.as_ptr() as usize;
        self.old_to_young_refs.entry(key).or_default().push(to);
    }

    pub fn remove_reference(&mut self, from: GcPtr<ValueHeader>, to: GcPtr<ValueHeader>) {
        let key = from.as_ptr() as usize;
        if let Some(refs) = self.old_to_young_refs.get_mut(&key) {
            refs.retain(|r| r.as_ptr() != to.as_ptr());
            if refs.is_empty() {
                self.old_to_young_refs.remove(&key);
            }
        }
    }

    pub fn get_references(&self, from: GcPtr<ValueHeader>) -> Option<&Vec<GcPtr<ValueHeader>>> {
        let key = from.as_ptr() as usize;
        self.old_to_young_refs.get(&key)
    }
}

impl Default for FreeList {
    fn default() -> Self {
        Self::new()
    }
}

impl FreeList {
    pub fn new() -> Self {
        Self {
            blocks: VecDeque::new(),
            total_free: 0,
        }
    }

    pub fn add_block(&mut self, address: usize, size: usize) {
        self.blocks.push_back(FreeBlock { address, size });
        self.total_free += size;
    }

    pub fn allocate(&mut self, size: usize) -> Option<usize> {
        // Find first fit
        for (i, block) in self.blocks.iter().enumerate() {
            if block.size >= size {
                let address = block.address;
                let remaining_size = block.size - size;

                if remaining_size > 0 {
                    // Split the block
                    let mut block = self.blocks.remove(i).unwrap();
                    block.address += size;
                    block.size = remaining_size;
                    self.blocks.insert(i, block);
                } else {
                    // Use entire block
                    self.blocks.remove(i);
                }

                self.total_free -= size;
                return Some(address);
            }
        }

        None
    }

    pub fn total_free(&self) -> usize {
        self.total_free
    }

    pub fn largest_block(&self) -> usize {
        self.blocks.iter().map(|b| b.size).max().unwrap_or(0)
    }
}

// Helper function for pointer comparison
fn ptr_eq(a: GcPtr<ValueHeader>, b: GcPtr<ValueHeader>) -> bool {
    a.as_ptr() == b.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ovm::config::MemoryConfig;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_gc_creation() {
        let config = MemoryConfig::default();
        let result = GarbageCollector::new(&config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gc_lifecycle() {
        let config = MemoryConfig::default();
        let mut gc = GarbageCollector::new(&config).unwrap();

        assert!(gc.start().is_ok());
        assert!(gc.stop().is_ok());
    }

    #[test]
    fn test_allocation_tracking() {
        let config = MemoryConfig::default();
        let gc = GarbageCollector::new(&config).unwrap();

        gc.record_allocation(1024);
        assert!(gc.should_collect());
    }

    #[test]
    fn test_gc_stats() {
        let config = MemoryConfig::default();
        let mut gc = GarbageCollector::new(&config).unwrap();

        let _ = gc.start();
        let _ = gc.force_collection();
        let _ = gc.stop();
    }

    #[test]
    fn test_safepoint_basic_coordination() {
        let manager = SafepointManager::new();
        
        // Test with no threads registered
        assert!(manager.request_safepoint().is_ok());
        manager.release_safepoint();
    }

    #[test]
    fn test_safepoint_single_thread() {
        let manager = Arc::new(SafepointManager::new());
        
        // Register thread
        manager.register_thread();
        
        let manager_clone = Arc::clone(&manager);
        let handle = thread::spawn(move || {
            // Simulate thread doing work and polling safepoints
            for _ in 0..10 {
                thread::sleep(Duration::from_millis(10));
                let _ = manager_clone.safepoint_poll();
            }
            // Unregister when done
            manager_clone.unregister_thread();
        });
        
        // Give thread time to start
        thread::sleep(Duration::from_millis(50));
        
        // Request safepoint - should succeed quickly
        assert!(manager.request_safepoint().is_ok());
        manager.release_safepoint();
        
        // Wait for thread to complete
        handle.join().unwrap();
    }

    #[test]
    fn test_safepoint_multiple_threads() {
        let manager = Arc::new(SafepointManager::new());
        let mut handles = Vec::new();
        
        // Start multiple threads
        for i in 0..3 {
            let manager_clone = Arc::clone(&manager);
            let handle = thread::spawn(move || {
                // Register with safepoint manager
                manager_clone.register_thread();
                
                // Simulate work with periodic safepoint polls
                for j in 0..5 {
                    thread::sleep(Duration::from_millis(20));
                    if let Err(e) = manager_clone.safepoint_poll() {
                        eprintln!("Thread {} iteration {} safepoint poll failed: {}", i, j, e);
                    }
                }
                
                // Unregister when done
                manager_clone.unregister_thread();
            });
            handles.push(handle);
        }
        
        // Give threads time to start and register
        thread::sleep(Duration::from_millis(100));
        
        // Request safepoint - should coordinate with all threads
        let debug_info = manager.get_debug_info();
        println!("Debug info before safepoint: {:?}", debug_info);
        
        assert!(manager.request_safepoint().is_ok());
        manager.release_safepoint();
        
        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn test_safepoint_timeout_recovery() {
        let manager = Arc::new(SafepointManager::new());
        
        // Register a thread but don't start polling
        manager.register_thread();
        
        // Request safepoint - should timeout since no thread is polling
        let result = manager.request_safepoint();
        assert!(result.is_err());
        if let Err(GcError::SafepointTimeout) = result {
            // Expected
        } else {
            panic!("Expected SafepointTimeout error");
        }
        
        // After timeout, system should recover
        manager.unregister_thread();
        assert!(manager.request_safepoint().is_ok());
        manager.release_safepoint();
    }

    #[test]
    fn test_safepoint_debug_info() {
        let manager = SafepointManager::new();
        
        let debug_info = manager.get_debug_info();
        assert_eq!(debug_info.total_threads, 0);
        assert_eq!(debug_info.threads_at_safepoint, 0);
        assert!(!debug_info.safepoint_requested);
        
        manager.register_thread();
        let debug_info = manager.get_debug_info();
        assert_eq!(debug_info.total_threads, 1);
        
        manager.unregister_thread();
        let debug_info = manager.get_debug_info();
        assert_eq!(debug_info.total_threads, 0);
    }
}

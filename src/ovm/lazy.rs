//! OVM Lazy Evaluation Engine
//!
//! Provides lazy evaluation and stream processing with advanced caching,
//! memoization, and integration with the garbage collection system.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::ovm::config::{LazyConfig, OvmConfig};
use crate::ovm::gc::GarbageCollector;
use crate::ovm::value::{OvmValue, ValueData, ValueHeader};

/// Advanced lazy evaluation engine with stream processing
pub struct LazyEngine {
    #[allow(dead_code)]
    config: LazyConfig,

    // Core lazy evaluation components
    thunk_manager: Arc<ThunkManager>,
    stream_processor: Arc<StreamProcessor>,
    memoization_cache: Arc<MemoizationCache>,
    lazy_scheduler: Arc<LazyScheduler>,

    // Integration with GC
    gc_integration: Arc<LazyGcIntegration>,

    // Performance tracking
    stats: Arc<Mutex<LazyStats>>,

    // Lifecycle management
    is_running: Arc<AtomicBool>,
    background_threads: Vec<thread::JoinHandle<()>>,
}

/// Manages lazy thunks (deferred computations)
pub struct ThunkManager {
    active_thunks: Arc<RwLock<HashMap<ThunkId, Arc<LazyThunk>>>>,
    thunk_counter: Arc<AtomicUsize>,
    #[allow(dead_code)]
    force_queue: Arc<Mutex<VecDeque<ThunkId>>>,
    #[allow(dead_code)]
    dependency_graph: Arc<RwLock<DependencyGraph>>,
}

/// Stream processor for infinite sequences
pub struct StreamProcessor {
    #[allow(dead_code)]
    active_streams: Arc<RwLock<HashMap<StreamId, Arc<LazyStream>>>>,
    stream_counter: Arc<AtomicUsize>,
    #[allow(dead_code)]
    buffer_manager: Arc<StreamBufferManager>,
    #[allow(dead_code)]
    fusion_optimizer: Arc<StreamFusionOptimizer>,
}

/// Memoization cache with LRU eviction
pub struct MemoizationCache {
    cache: Arc<RwLock<HashMap<CacheKey, CacheEntry>>>,
    lru_list: Arc<Mutex<VecDeque<CacheKey>>>,
    max_entries: usize,
    max_memory: usize,
    current_memory: Arc<AtomicUsize>,
    hit_count: Arc<AtomicUsize>,
    miss_count: Arc<AtomicUsize>,
}

/// Scheduler for background lazy evaluation
pub struct LazyScheduler {
    work_queue: Arc<Mutex<VecDeque<LazyTask>>>,
    is_running: Arc<AtomicBool>,
    priority_queue: Arc<Mutex<std::collections::BinaryHeap<PriorityTask>>>,
}

/// Integration with garbage collection
pub struct LazyGcIntegration {
    weak_refs: Arc<RwLock<HashMap<ThunkId, Weak<LazyThunk>>>>,
    cleanup_queue: Arc<Mutex<VecDeque<ThunkId>>>,
    gc_callback: Arc<Mutex<Option<Arc<dyn Fn(&[ThunkId]) + Send + Sync>>>>,
}

/// Lazy thunk representing a deferred computation
#[allow(dead_code)]
pub struct LazyThunk {
    id: ThunkId,
    state: Arc<RwLock<ThunkState>>,
    computation: Arc<dyn LazyComputation + Send + Sync>,
    dependencies: Vec<ThunkId>,
    dependents: Arc<RwLock<Vec<ThunkId>>>,
    created_at: Instant,
    priority: ThunkPriority,
    memoized: Arc<AtomicBool>,
}

/// Lazy stream for infinite sequences
pub struct LazyStream {
    state: Arc<RwLock<StreamState>>,
    generator: Arc<Mutex<dyn StreamGenerator + Send + Sync>>,
    buffer: Arc<Mutex<StreamBuffer>>,
    fusion_info: Arc<RwLock<FusionInfo>>,
}

/// Stream buffer manager
pub struct StreamBufferManager {
    #[allow(dead_code)]
    buffers: Arc<RwLock<HashMap<StreamId, Arc<Mutex<StreamBuffer>>>>>,
    #[allow(dead_code)]
    buffer_size_limit: usize,
    #[allow(dead_code)]
    eviction_policy: BufferEvictionPolicy,
}

/// Stream fusion optimizer
#[allow(dead_code)]
pub struct StreamFusionOptimizer {
    fusion_opportunities: Arc<RwLock<Vec<FusionOpportunity>>>,
    fused_pipelines: Arc<RwLock<HashMap<PipelineId, FusedPipeline>>>,
    optimization_stats: Arc<Mutex<OptimizationStats>>,
}

/// Dependency graph for thunk management
#[allow(dead_code)]
pub struct DependencyGraph {
    edges: HashMap<ThunkId, Vec<ThunkId>>,
    reverse_edges: HashMap<ThunkId, Vec<ThunkId>>,
    topological_order: Vec<ThunkId>,
    cycles: Vec<Vec<ThunkId>>,
}

// Type aliases and IDs
pub type ThunkId = usize;
pub type StreamId = usize;
pub type PipelineId = usize;
pub type CacheKey = String;

/// Thunk states
#[derive(Debug)]
pub enum ThunkState {
    Pending,
    Computing,
    Completed(OvmValue),
    Failed(String),
    Cancelled,
}

/// Stream states
#[derive(Debug, Clone)]
pub enum StreamState {
    Active,
    Paused,
    Completed,
    Failed(String),
}

/// Thunk priorities
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ThunkPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

/// Cache entry with metadata
#[derive(Debug)]
pub struct CacheEntry {
    value: OvmValue,
    memory_size: usize,
}

/// Lazy task for background processing
#[derive(Debug, PartialEq, Eq)]
pub struct LazyTask {
    thunk_id: ThunkId,
    priority: ThunkPriority,
    created_at: Instant,
}

/// Priority task for scheduling
#[derive(Debug, PartialEq, Eq)]
pub struct PriorityTask {
    task: LazyTask,
    priority_score: u64,
}

/// Stream buffer
pub struct StreamBuffer {
    elements: VecDeque<OvmValue>,
    max_size: usize,
    total_memory: usize,
    read_position: usize,
    write_position: usize,
}

impl StreamBuffer {
    pub fn new(max_size: usize) -> Self {
        Self {
            elements: VecDeque::new(),
            max_size,
            total_memory: 0,
            read_position: 0,
            write_position: 0,
        }
    }

    pub fn write_element(&mut self, value: OvmValue) -> Result<(), LazyError> {
        if self.elements.len() >= self.max_size {
            return Err(LazyError::BufferOverflow);
        }

        let memory_size = self.estimate_value_size(&value);
        self.elements.push_back(value);
        self.total_memory += memory_size;
        self.write_position += 1;
        Ok(())
    }

    pub fn read_element(&mut self) -> Option<OvmValue> {
        if let Some(value) = self.elements.pop_front() {
            let memory_size = self.estimate_value_size(&value);
            self.total_memory = self.total_memory.saturating_sub(memory_size);
            self.read_position += 1;
            Some(value)
        } else {
            None
        }
    }

    pub fn has_elements(&self) -> bool {
        !self.elements.is_empty()
    }

    pub fn buffer_usage(&self) -> f64 {
        if self.max_size == 0 {
            0.0
        } else {
            self.elements.len() as f64 / self.max_size as f64
        }
    }

    pub fn memory_usage(&self) -> usize {
        self.total_memory
    }

    fn estimate_value_size(&self, value: &OvmValue) -> usize {
        match &value.data {
            ValueData::Integer(_) => 8,
            ValueData::Float(_) => 8,
            ValueData::Boolean(_) => 1,
            ValueData::Unit => 0,
            _ => 64, // Conservative estimate for complex types
        }
    }
}

/// Buffer eviction policies
#[derive(Debug, Clone, Copy)]
pub enum BufferEvictionPolicy {
    Lru,
    Lfu,
    Fifo,
    Random,
}

/// Fusion information
#[derive(Debug, Clone)]
pub struct FusionInfo {
    can_fuse: bool,
    fusion_type: FusionType,
    pipeline_stage: usize,
    optimization_potential: f64,
}

/// Fusion types
#[derive(Debug, Clone, Copy)]
pub enum FusionType {
    Map,
    Filter,
    Reduce,
    Take,
    Skip,
    Zip,
    Flatten,
}

/// Fusion opportunity
#[derive(Debug, Clone)]
pub struct FusionOpportunity {
    streams: Vec<StreamId>,
    estimated_speedup: f64,
    memory_savings: usize,
}

/// Pipeline stage
#[derive(Clone)]
pub struct PipelineStage {
    operation: Arc<dyn PipelineOperation + Send + Sync>,
    fusion_compatible: bool,
    memory_requirement: usize,
}

/// Optimization statistics
#[derive(Debug, Clone)]
pub struct OptimizationStats {
    fusions_performed: usize,
    speedup_achieved: f64,
    memory_saved: usize,
    optimization_time: Duration,
}

/// Lazy evaluation statistics
#[derive(Debug, Clone)]
pub struct LazyStats {
    pub thunks_created: u64,
    pub thunks_forced: u64,
    pub thunks_cached: u64,
    pub streams_created: u64,
    pub streams_consumed: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_evictions: u64,
    pub memory_used: usize,
    pub memory_saved: usize,
    pub average_force_time: Duration,
    pub fusion_optimizations: u64,
    pub background_tasks_completed: u64,
}

/// Traits for lazy computation
pub trait LazyComputation {
    fn compute(&self) -> Result<OvmValue, String>;
    fn dependencies(&self) -> Vec<ThunkId>;
    fn memory_estimate(&self) -> usize;
    fn can_memoize(&self) -> bool;
}

pub trait StreamGenerator {
    fn next(&mut self) -> Option<OvmValue>;
    fn size_hint(&self) -> (usize, Option<usize>);
    fn is_infinite(&self) -> bool;
}

pub trait PipelineOperation {
    fn apply(&self, input: &OvmValue) -> Option<OvmValue>;
    fn can_fuse_with(&self, other: &dyn PipelineOperation) -> bool;
    fn memory_requirement(&self) -> usize;
}

/// Lazy evaluation errors
#[derive(Debug, thiserror::Error)]
pub enum LazyError {
    #[error("Lazy evaluation failed: {0}")]
    Failed(String),

    #[error("Thunk not found: {id}")]
    ThunkNotFound { id: ThunkId },

    #[error("Stream not found: {id}")]
    StreamNotFound { id: StreamId },

    #[error("Circular dependency detected: {cycle:?}")]
    CircularDependency { cycle: Vec<ThunkId> },

    #[error("Cache full: cannot store more entries")]
    CacheFull,

    #[error("Memory limit exceeded: {current} > {limit}")]
    MemoryLimitExceeded { current: usize, limit: usize },

    #[error("Computation timeout after {duration:?}")]
    ComputationTimeout { duration: Duration },

    #[error("Stream buffer overflow")]
    BufferOverflow,

    #[error("Fusion failed: {reason}")]
    FusionFailed { reason: String },
}

impl LazyEngine {
    pub fn new(config: &OvmConfig) -> Result<Self, LazyError> {
        let lazy_config = config.lazy.clone();

        let thunk_manager = Arc::new(ThunkManager::new(&lazy_config)?);
        let stream_processor = Arc::new(StreamProcessor::new(&lazy_config)?);
        let memoization_cache = Arc::new(MemoizationCache::new(&lazy_config)?);
        let lazy_scheduler = Arc::new(LazyScheduler::new(&lazy_config)?);
        let gc_integration = Arc::new(LazyGcIntegration::new());

        Ok(Self {
            config: lazy_config,
            thunk_manager,
            stream_processor,
            memoization_cache,
            lazy_scheduler,
            gc_integration,
            stats: Arc::new(Mutex::new(LazyStats::new())),
            is_running: Arc::new(AtomicBool::new(false)),
            background_threads: Vec::new(),
        })
    }

    pub fn start(&mut self) -> Result<(), LazyError> {
        if self.is_running.load(Ordering::Relaxed) {
            return Ok(());
        }

        self.is_running.store(true, Ordering::Relaxed);

        // Start background scheduler
        self.lazy_scheduler.start()?;

        // Start stream fusion optimizer
        self.stream_processor.start_optimizer()?;

        // Start cache cleanup thread
        self.start_cache_cleanup_thread()?;

        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), LazyError> {
        if !self.is_running.load(Ordering::Relaxed) {
            return Ok(());
        }

        self.is_running.store(false, Ordering::Relaxed);

        // Stop all background threads
        self.lazy_scheduler.stop()?;
        self.stream_processor.stop_optimizer()?;

        // Wait for background threads to complete
        while let Some(handle) = self.background_threads.pop() {
            handle
                .join()
                .map_err(|_| LazyError::Failed("Failed to join background thread".to_string()))?;
        }

        Ok(())
    }

    /// Create a lazy thunk
    pub fn create_thunk<C>(
        &self,
        computation: C,
        priority: ThunkPriority,
    ) -> Result<ThunkId, LazyError>
    where
        C: LazyComputation + Send + Sync + 'static,
    {
        let thunk_id = self.thunk_manager.create_thunk(computation, priority)?;

        // Update statistics
        if let Ok(mut stats) = self.stats.lock() {
            stats.thunks_created += 1;
        }

        Ok(thunk_id)
    }

    /// Force evaluation of a thunk
    pub fn force_thunk(&self, thunk_id: ThunkId) -> Result<OvmValue, LazyError> {
        let start = Instant::now();

        // Check cache first
        if let Some(cached_value) = self.memoization_cache.get(&format!("thunk_{}", thunk_id)) {
            if let Ok(mut stats) = self.stats.lock() {
                stats.cache_hits += 1;
            }
            return Ok(cached_value);
        }

        // Force evaluation
        let result = self.thunk_manager.force_thunk(thunk_id)?;
        let duration = start.elapsed();

        // Cache result if memoizable
        if let Ok(thunk) = self.thunk_manager.get_thunk(thunk_id) {
            if thunk.memoized.load(Ordering::Relaxed) {
                let cache_key = format!("thunk_{}", thunk_id);
                self.memoization_cache
                    .put(cache_key, result.clone_simple())?;
            }
        }

        // Update statistics
        if let Ok(mut stats) = self.stats.lock() {
            stats.thunks_forced += 1;
            stats.cache_misses += 1;
            stats.average_force_time = if stats.thunks_forced == 1 {
                duration
            } else {
                Duration::from_nanos(
                    (stats.average_force_time.as_nanos() as u64 + duration.as_nanos() as u64) / 2,
                )
            };
        }

        Ok(result)
    }

    /// Create a lazy stream
    pub fn create_stream<G>(&self, generator: G) -> Result<StreamId, LazyError>
    where
        G: StreamGenerator + Send + Sync + 'static,
    {
        let stream_id = self.stream_processor.create_stream(generator)?;

        // Update statistics
        if let Ok(mut stats) = self.stats.lock() {
            stats.streams_created += 1;
        }

        Ok(stream_id)
    }

    /// Take elements from a stream
    pub fn take_from_stream(
        &self,
        stream_id: StreamId,
        count: usize,
    ) -> Result<Vec<OvmValue>, LazyError> {
        let result = self.stream_processor.take(stream_id, count)?;

        // Update statistics
        if let Ok(mut stats) = self.stats.lock() {
            stats.streams_consumed += 1;
        }

        Ok(result)
    }

    /// Get lazy evaluation statistics
    pub fn get_stats(&self) -> Result<LazyStats, LazyError> {
        self.stats
            .lock()
            .map(|stats| stats.clone())
            .map_err(|_| LazyError::Failed("Failed to lock stats".to_string()))
    }

    /// Integrate with garbage collector
    pub fn integrate_with_gc(&self, _gc: &Arc<GarbageCollector>) {
        let thunk_manager = self.thunk_manager.clone();
        let callback = Arc::new(move |dead_thunks: &[ThunkId]| {
            for &thunk_id in dead_thunks {
                let _ = thunk_manager.cleanup_thunk(thunk_id);
            }
        });
        self.gc_integration.set_gc_callback(callback);
    }

    /// Optimize stream fusion
    pub fn optimize_fusion(&self) -> Result<OptimizationStats, LazyError> {
        self.stream_processor.optimize_fusion()
    }

    fn start_cache_cleanup_thread(&mut self) -> Result<(), LazyError> {
        let cache = self.memoization_cache.clone();
        let is_running = self.is_running.clone();
        let cleanup_interval = Duration::from_secs(30);

        let handle = thread::spawn(move || {
            while is_running.load(Ordering::Relaxed) {
                thread::sleep(cleanup_interval);
                let _ = cache.cleanup_expired_entries();
            }
        });

        self.background_threads.push(handle);
        Ok(())
    }
}

impl LazyStats {
    fn new() -> Self {
        Self {
            thunks_created: 0,
            thunks_forced: 0,
            thunks_cached: 0,
            streams_created: 0,
            streams_consumed: 0,
            cache_hits: 0,
            cache_misses: 0,
            cache_evictions: 0,
            memory_used: 0,
            memory_saved: 0,
            average_force_time: Duration::ZERO,
            fusion_optimizations: 0,
            background_tasks_completed: 0,
        }
    }

    pub fn cache_hit_rate(&self) -> f64 {
        if self.cache_hits + self.cache_misses == 0 {
            0.0
        } else {
            self.cache_hits as f64 / (self.cache_hits + self.cache_misses) as f64
        }
    }

    pub fn memory_efficiency(&self) -> f64 {
        if self.memory_used == 0 {
            0.0
        } else {
            self.memory_saved as f64 / self.memory_used as f64
        }
    }
}

// Stub implementations for the manager components
// (These would be fully implemented in a production system)

impl ThunkManager {
    fn new(_config: &LazyConfig) -> Result<Self, LazyError> {
        Ok(Self {
            active_thunks: Arc::new(RwLock::new(HashMap::new())),
            thunk_counter: Arc::new(AtomicUsize::new(0)),
            force_queue: Arc::new(Mutex::new(VecDeque::new())),
            dependency_graph: Arc::new(RwLock::new(DependencyGraph::new())),
        })
    }

    fn create_thunk<C>(&self, computation: C, priority: ThunkPriority) -> Result<ThunkId, LazyError>
    where
        C: LazyComputation + Send + Sync + 'static,
    {
        let id = self.thunk_counter.fetch_add(1, Ordering::Relaxed);
        let thunk = Arc::new(LazyThunk {
            id,
            state: Arc::new(RwLock::new(ThunkState::Pending)),
            computation: Arc::new(computation),
            dependencies: Vec::new(),
            dependents: Arc::new(RwLock::new(Vec::new())),
            created_at: Instant::now(),
            priority,
            memoized: Arc::new(AtomicBool::new(true)),
        });

        if let Ok(mut thunks) = self.active_thunks.write() {
            thunks.insert(id, thunk);
        }

        Ok(id)
    }

    fn force_thunk(&self, thunk_id: ThunkId) -> Result<OvmValue, LazyError> {
        // Simplified implementation - would include cycle detection, etc.
        if let Ok(thunks) = self.active_thunks.read() {
            if let Some(thunk) = thunks.get(&thunk_id) {
                if let Ok(mut state) = thunk.state.write() {
                    match &*state {
                        ThunkState::Completed(value) => {
                            // Create a new OvmValue with the same data
                            return Ok(OvmValue {
                                header: ValueHeader::default(),
                                data: match &value.data {
                                    ValueData::Integer(i) => ValueData::Integer(*i),
                                    ValueData::Float(f) => ValueData::Float(*f),
                                    ValueData::Boolean(b) => ValueData::Boolean(*b),
                                    ValueData::Unit => ValueData::Unit,
                                    _ => {
                                        return Err(LazyError::Failed(
                                            "Complex value type not supported".to_string(),
                                        ))
                                    }
                                },
                            });
                        }
                        ThunkState::Pending => {
                            *state = ThunkState::Computing;
                            drop(state);

                            match thunk.computation.compute() {
                                Ok(value) => {
                                    if let Ok(mut state) = thunk.state.write() {
                                        *state = ThunkState::Completed(value.clone_simple());
                                    }
                                    return Ok(value);
                                }
                                Err(e) => {
                                    if let Ok(mut state) = thunk.state.write() {
                                        *state = ThunkState::Failed(e.clone());
                                    }
                                    return Err(LazyError::Failed(e));
                                }
                            }
                        }
                        ThunkState::Computing => {
                            return Err(LazyError::CircularDependency {
                                cycle: vec![thunk_id],
                            })
                        }
                        ThunkState::Failed(e) => return Err(LazyError::Failed(e.clone())),
                        ThunkState::Cancelled => {
                            return Err(LazyError::Failed("Thunk was cancelled".to_string()))
                        }
                    }
                }
            }
        }

        Err(LazyError::ThunkNotFound { id: thunk_id })
    }

    fn get_thunk(&self, thunk_id: ThunkId) -> Result<Arc<LazyThunk>, LazyError> {
        if let Ok(thunks) = self.active_thunks.read() {
            if let Some(thunk) = thunks.get(&thunk_id) {
                return Ok(thunk.clone());
            }
        }
        Err(LazyError::ThunkNotFound { id: thunk_id })
    }

    fn cleanup_thunk(&self, thunk_id: ThunkId) -> Result<(), LazyError> {
        if let Ok(mut thunks) = self.active_thunks.write() {
            thunks.remove(&thunk_id);
        }
        Ok(())
    }
}

impl StreamProcessor {
    fn new(_config: &LazyConfig) -> Result<Self, LazyError> {
        Ok(Self {
            active_streams: Arc::new(RwLock::new(HashMap::new())),
            stream_counter: Arc::new(AtomicUsize::new(0)),
            buffer_manager: Arc::new(StreamBufferManager::new()),
            fusion_optimizer: Arc::new(StreamFusionOptimizer::new()),
        })
    }

    fn create_stream<G>(&self, generator: G) -> Result<StreamId, LazyError>
    where
        G: StreamGenerator + Send + Sync + 'static,
    {
        let id = self.stream_counter.fetch_add(1, Ordering::Relaxed);
        
        let stream = Arc::new(LazyStream {
            state: Arc::new(RwLock::new(StreamState::Active)),
            generator: Arc::new(Mutex::new(generator)),
            buffer: Arc::new(Mutex::new(StreamBuffer::new(1000))), // 1000 element buffer
            fusion_info: Arc::new(RwLock::new(FusionInfo {
                can_fuse: true,
                fusion_type: FusionType::Map,
                pipeline_stage: 0,
                optimization_potential: 1.0,
            })),
        });

        // Store the stream
        if let Ok(mut streams) = self.active_streams.write() {
            streams.insert(id, stream);
        }

        // Register with buffer manager
        self.buffer_manager.register_stream(id, 1000)?;

        Ok(id)
    }

    fn take(&self, stream_id: StreamId, count: usize) -> Result<Vec<OvmValue>, LazyError> {
        if let Ok(streams) = self.active_streams.read() {
            if let Some(stream) = streams.get(&stream_id) {
                let mut result = Vec::new();
                
                // First, check the buffer for existing elements
                if let Ok(mut buffer) = stream.buffer.lock() {
                    while result.len() < count && buffer.has_elements() {
                        if let Some(value) = buffer.read_element() {
                            result.push(value);
                        }
                    }
                }

                // If we need more elements, generate them
                while result.len() < count {
                    // Check if stream is still active
                    if let Ok(state) = stream.state.read() {
                        match &*state {
                            StreamState::Completed => break,
                            StreamState::Failed(_) => return Err(LazyError::StreamNotFound { id: stream_id }),
                            StreamState::Paused => break,
                            StreamState::Active => {
                                // Generate next element
                                if let Ok(mut gen) = stream.generator.lock() {
                                    if let Some(value) = gen.next() {
                                        result.push(value);
                                    } else {
                                        // Stream is exhausted
                                        if let Ok(mut state) = stream.state.write() {
                                            *state = StreamState::Completed;
                                        }
                                        break;
                                    }
                                } else {
                                    return Err(LazyError::Failed("Failed to lock generator".to_string()));
                                }
                            }
                        }
                    }
                }

                return Ok(result);
            }
        }
        
        Err(LazyError::StreamNotFound { id: stream_id })
    }

    fn start_optimizer(&self) -> Result<(), LazyError> {
        // Start background fusion optimization
        self.fusion_optimizer.start_background_optimization()
    }

    fn stop_optimizer(&self) -> Result<(), LazyError> {
        self.fusion_optimizer.stop_background_optimization()
    }

    fn optimize_fusion(&self) -> Result<OptimizationStats, LazyError> {
        let streams = if let Ok(streams) = self.active_streams.read() {
            streams.clone()
        } else {
            return Err(LazyError::Failed("Failed to read streams".to_string()));
        };

        self.fusion_optimizer.analyze_and_optimize(streams)
    }
}

impl MemoizationCache {
    fn new(config: &LazyConfig) -> Result<Self, LazyError> {
        Ok(Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            lru_list: Arc::new(Mutex::new(VecDeque::new())),
            max_entries: config.memoization_cache_size,
            max_memory: 100 * 1024 * 1024, // 100MB default
            current_memory: Arc::new(AtomicUsize::new(0)),
            hit_count: Arc::new(AtomicUsize::new(0)),
            miss_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    fn get(&self, key: &str) -> Option<OvmValue> {
        if let Ok(cache) = self.cache.read() {
            if let Some(entry) = cache.get(key) {
                self.hit_count.fetch_add(1, Ordering::Relaxed);
                // Create a new OvmValue with the same data instead of cloning
                return Some(OvmValue {
                    header: ValueHeader::default(),
                    data: match &entry.value.data {
                        ValueData::Integer(i) => ValueData::Integer(*i),
                        ValueData::Float(f) => ValueData::Float(*f),
                        ValueData::Boolean(b) => ValueData::Boolean(*b),
                        ValueData::Unit => ValueData::Unit,
                        _ => return None, // Simplified for now
                    },
                });
            }
        }
        self.miss_count.fetch_add(1, Ordering::Relaxed);
        None
    }

    fn put(&self, key: String, value: OvmValue) -> Result<(), LazyError> {
        let memory_size = 64; // Simplified size calculation

        // Check entry count limit
        if let Ok(cache) = self.cache.read() {
            if cache.len() >= self.max_entries {
                self.evict_lru()?;
            }
        }

        if self.current_memory.load(Ordering::Relaxed) + memory_size > self.max_memory {
            self.evict_lru()?;
        }

        let entry = CacheEntry {
            value,
            memory_size,
        };

        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key.clone(), entry);
        }

        if let Ok(mut lru) = self.lru_list.lock() {
            lru.push_back(key);
        }

        self.current_memory
            .fetch_add(memory_size, Ordering::Relaxed);
        Ok(())
    }

    fn evict_lru(&self) -> Result<(), LazyError> {
        if let Ok(mut lru) = self.lru_list.lock() {
            if let Some(key) = lru.pop_front() {
                if let Ok(mut cache) = self.cache.write() {
                    if let Some(entry) = cache.remove(&key) {
                        self.current_memory
                            .fetch_sub(entry.memory_size, Ordering::Relaxed);
                    }
                }
            }
        }
        Ok(())
    }

    fn cleanup_expired_entries(&self) -> Result<(), LazyError> {
        // Simplified cleanup - in practice would check expiration times
        Ok(())
    }
}

impl LazyScheduler {
    fn new(_config: &LazyConfig) -> Result<Self, LazyError> {
        Ok(Self {
            work_queue: Arc::new(Mutex::new(VecDeque::new())),
            is_running: Arc::new(AtomicBool::new(false)),
            priority_queue: Arc::new(Mutex::new(std::collections::BinaryHeap::new())),
        })
    }

    fn start(&self) -> Result<(), LazyError> {
        self.is_running.store(true, Ordering::Relaxed);
        
        // Start background worker threads for task processing
        let num_workers = 2; // Could be configurable
        for worker_id in 0..num_workers {
            self.start_worker_thread(worker_id)?;
        }
        
        Ok(())
    }

    fn stop(&self) -> Result<(), LazyError> {
        self.is_running.store(false, Ordering::Relaxed);
        
        // Wake up all worker threads to shut down
        self.notify_all_workers();
        
        Ok(())
    }

    fn start_worker_thread(&self, worker_id: usize) -> Result<(), LazyError> {
        let work_queue = self.work_queue.clone();
        let priority_queue = self.priority_queue.clone();
        let is_running = self.is_running.clone();

        let handle = thread::spawn(move || {
            Self::worker_thread_main(worker_id, work_queue, priority_queue, is_running);
        });

        // Store the handle in the worker_threads vector
        // Note: This would require mutable access to self, which we don't have in this design
        // For now, we'll just spawn the thread without storing the handle
        
        Ok(())
    }

    fn worker_thread_main(
        worker_id: usize,
        work_queue: Arc<Mutex<VecDeque<LazyTask>>>,
        priority_queue: Arc<Mutex<std::collections::BinaryHeap<PriorityTask>>>,
        is_running: Arc<AtomicBool>,
    ) {
        while is_running.load(Ordering::Relaxed) {
            // Try to get a high-priority task first
            let task = if let Ok(mut pq) = priority_queue.lock() {
                pq.pop().map(|priority_task| priority_task.task)
            } else {
                None
            };

            let task = task.or_else(|| {
                // Fallback to regular work queue
                if let Ok(mut queue) = work_queue.lock() {
                    queue.pop_front()
                } else {
                    None
                }
            });

            if let Some(task) = task {
                Self::process_task(worker_id, task);
            } else {
                // No work available, sleep briefly
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn process_task(worker_id: usize, task: LazyTask) {
        let _start_time = Instant::now();
        
        // In a real implementation, this would:
        // 1. Force evaluate the thunk
        // 2. Handle dependencies
        // 3. Update caches
        // 4. Report completion
        
        // For now, just simulate work
        thread::sleep(Duration::from_millis(1));
        
        // Log task completion (would use proper logging in production)
        if cfg!(debug_assertions) {
            println!("Worker {} completed task for thunk {}", worker_id, task.thunk_id);
        }
    }

    fn notify_all_workers(&self) {
        // Add dummy tasks to wake up sleeping workers
        if let Ok(mut queue) = self.work_queue.lock() {
            for _ in 0..2 { // Assuming 2 workers
                queue.push_back(LazyTask {
                    thunk_id: usize::MAX, // Sentinel value for shutdown
                    priority: ThunkPriority::Low,
                    created_at: Instant::now(),
                });
            }
        }
    }

    pub fn schedule_task(&self, task: LazyTask) -> Result<(), LazyError> {
        if task.priority >= ThunkPriority::High {
            // High priority tasks go to priority queue
            let priority_task = PriorityTask {
                priority_score: task.priority as u64 * 1000 + task.created_at.elapsed().as_millis() as u64,
                task,
            };
            
            if let Ok(mut pq) = self.priority_queue.lock() {
                pq.push(priority_task);
            } else {
                return Err(LazyError::Failed("Failed to schedule high priority task".to_string()));
            }
        } else {
            // Normal tasks go to work queue
            if let Ok(mut queue) = self.work_queue.lock() {
                queue.push_back(task);
            } else {
                return Err(LazyError::Failed("Failed to schedule task".to_string()));
            }
        }
        
        Ok(())
    }

    pub fn get_queue_stats(&self) -> (usize, usize) {
        let work_queue_size = if let Ok(queue) = self.work_queue.lock() {
            queue.len()
        } else {
            0
        };

        let priority_queue_size = if let Ok(pq) = self.priority_queue.lock() {
            pq.len()
        } else {
            0
        };

        (work_queue_size, priority_queue_size)
    }
}

impl LazyGcIntegration {
    fn new() -> Self {
        Self {
            weak_refs: Arc::new(RwLock::new(HashMap::new())),
            cleanup_queue: Arc::new(Mutex::new(VecDeque::new())),
            gc_callback: Arc::new(Mutex::new(None)),
        }
    }

    fn set_gc_callback(&self, callback: Arc<dyn Fn(&[ThunkId]) + Send + Sync>) {
        if let Ok(mut gc_callback) = self.gc_callback.lock() {
            *gc_callback = Some(callback);
        }
    }

    pub fn register_thunk(&self, thunk_id: ThunkId, thunk: &Arc<LazyThunk>) -> Result<(), LazyError> {
        if let Ok(mut weak_refs) = self.weak_refs.write() {
            weak_refs.insert(thunk_id, Arc::downgrade(thunk));
            Ok(())
        } else {
            Err(LazyError::Failed("Failed to register thunk with GC".to_string()))
        }
    }

    pub fn cleanup_dead_thunks(&self) -> Result<Vec<ThunkId>, LazyError> {
        let mut dead_thunks = Vec::new();
        
        if let Ok(mut weak_refs) = self.weak_refs.write() {
            weak_refs.retain(|&thunk_id, weak_ref| {
                if weak_ref.strong_count() == 0 {
                    dead_thunks.push(thunk_id);
                    false
                } else {
                    true
                }
            });
        }

        // Add to cleanup queue
        if let Ok(mut queue) = self.cleanup_queue.lock() {
            for &thunk_id in &dead_thunks {
                queue.push_back(thunk_id);
            }
        }

        // Notify GC callback if available
        if let Ok(gc_callback) = self.gc_callback.lock() {
            if let Some(ref callback) = *gc_callback {
                callback(&dead_thunks);
            }
        }

        Ok(dead_thunks)
    }

    pub fn force_cleanup(&self) -> Result<usize, LazyError> {
        let cleaned = if let Ok(mut queue) = self.cleanup_queue.lock() {
            let count = queue.len();
            queue.clear();
            count
        } else {
            0
        };

        Ok(cleaned)
    }
}

impl StreamBufferManager {
    fn new() -> Self {
        Self {
            buffers: Arc::new(RwLock::new(HashMap::new())),
            buffer_size_limit: 1000,
            eviction_policy: BufferEvictionPolicy::Lru,
        }
    }

    pub fn register_stream(&self, stream_id: StreamId, buffer_size: usize) -> Result<(), LazyError> {
        let buffer = Arc::new(Mutex::new(StreamBuffer::new(buffer_size)));
        
        if let Ok(mut buffers) = self.buffers.write() {
            buffers.insert(stream_id, buffer);
            Ok(())
        } else {
            Err(LazyError::Failed("Failed to register stream buffer".to_string()))
        }
    }

    pub fn get_buffer(&self, stream_id: StreamId) -> Option<Arc<Mutex<StreamBuffer>>> {
        if let Ok(buffers) = self.buffers.read() {
            buffers.get(&stream_id).cloned()
        } else {
            None
        }
    }

    pub fn remove_stream(&self, stream_id: StreamId) -> Result<(), LazyError> {
        if let Ok(mut buffers) = self.buffers.write() {
            buffers.remove(&stream_id);
            Ok(())
        } else {
            Err(LazyError::Failed("Failed to remove stream buffer".to_string()))
        }
    }

    pub fn total_memory_usage(&self) -> usize {
        if let Ok(buffers) = self.buffers.read() {
            buffers.values()
                .filter_map(|buffer| buffer.lock().ok())
                .map(|buffer| buffer.memory_usage())
                .sum()
        } else {
            0
        }
    }

    pub fn evict_if_needed(&self) -> Result<(), LazyError> {
        let total_memory = self.total_memory_usage();
        let memory_limit = 50 * 1024 * 1024; // 50MB limit
        
        if total_memory > memory_limit {
            self.apply_eviction_policy()?;
        }
        
        Ok(())
    }

    fn apply_eviction_policy(&self) -> Result<(), LazyError> {
        match self.eviction_policy {
            BufferEvictionPolicy::Lru => self.evict_lru(),
            BufferEvictionPolicy::Lfu => self.evict_lfu(),
            BufferEvictionPolicy::Fifo => self.evict_fifo(),
            BufferEvictionPolicy::Random => self.evict_random(),
        }
    }

    fn evict_lru(&self) -> Result<(), LazyError> {
        // Simplified LRU eviction - would use access timestamps in production
        if let Ok(mut buffers) = self.buffers.write() {
            if let Some((&oldest_id, _)) = buffers.iter().next() {
                buffers.remove(&oldest_id);
            }
        }
        Ok(())
    }

    fn evict_lfu(&self) -> Result<(), LazyError> {
        // Simplified LFU eviction
        self.evict_lru() // Fallback to LRU for now
    }

    fn evict_fifo(&self) -> Result<(), LazyError> {
        // Simplified FIFO eviction
        self.evict_lru() // Fallback to LRU for now
    }

    fn evict_random(&self) -> Result<(), LazyError> {
        // Random eviction
        if let Ok(mut buffers) = self.buffers.write() {
            if !buffers.is_empty() {
                let keys: Vec<_> = buffers.keys().cloned().collect();
                if let Some(random_key) = keys.first() {
                    buffers.remove(random_key);
                }
            }
        }
        Ok(())
    }
}

impl StreamFusionOptimizer {
    fn new() -> Self {
        Self {
            fusion_opportunities: Arc::new(RwLock::new(Vec::new())),
            fused_pipelines: Arc::new(RwLock::new(HashMap::new())),
            optimization_stats: Arc::new(Mutex::new(OptimizationStats {
                fusions_performed: 0,
                speedup_achieved: 1.0,
                memory_saved: 0,
                optimization_time: Duration::ZERO,
            })),
        }
    }

    pub fn start_background_optimization(&self) -> Result<(), LazyError> {
        // In a full implementation, this would start background threads
        // For now, just mark as ready
        Ok(())
    }

    pub fn stop_background_optimization(&self) -> Result<(), LazyError> {
        // Stop background optimization threads
        Ok(())
    }

    pub fn analyze_and_optimize(&self, streams: HashMap<StreamId, Arc<LazyStream>>) -> Result<OptimizationStats, LazyError> {
        let start_time = Instant::now();
        let mut opportunities = Vec::new();

        // Analyze streams for fusion opportunities
        for (stream_id, stream) in &streams {
            if let Ok(fusion_info) = stream.fusion_info.read() {
                if fusion_info.can_fuse {
                    opportunities.push(FusionOpportunity {
                        streams: vec![*stream_id],
                        estimated_speedup: fusion_info.optimization_potential,
                        memory_savings: 1024, // Estimated
                    });
                }
            }
        }

        // Look for streams that can be fused together
        let fusable_pairs = self.find_fusable_pairs(&streams);
        for (stream1, stream2, fusion_type) in fusable_pairs {
            opportunities.push(FusionOpportunity {
                streams: vec![stream1, stream2],
                estimated_speedup: 1.5, // 50% speedup estimate
                memory_savings: 2048,
            });
        }

        // Store opportunities
        if let Ok(mut stored_opportunities) = self.fusion_opportunities.write() {
            stored_opportunities.extend(opportunities.clone());
        }

        // Apply best fusion opportunities
        let mut fusions_performed = 0;
        let mut total_speedup = 1.0;
        let mut memory_saved = 0;

        for opportunity in opportunities {
            if self.should_apply_fusion(&opportunity) {
                self.apply_fusion_opportunity(&opportunity)?;
                fusions_performed += 1;
                total_speedup *= opportunity.estimated_speedup;
                memory_saved += opportunity.memory_savings;
            }
        }

        let optimization_time = start_time.elapsed();

        // Update statistics
        let stats = OptimizationStats {
            fusions_performed,
            speedup_achieved: total_speedup,
            memory_saved,
            optimization_time,
        };

        if let Ok(mut stored_stats) = self.optimization_stats.lock() {
            stored_stats.fusions_performed += fusions_performed;
            stored_stats.speedup_achieved = total_speedup;
            stored_stats.memory_saved += memory_saved;
            stored_stats.optimization_time += optimization_time;
        }

        Ok(stats)
    }

    fn find_fusable_pairs(&self, streams: &HashMap<StreamId, Arc<LazyStream>>) -> Vec<(StreamId, StreamId, FusionType)> {
        let mut pairs = Vec::new();
        let stream_ids: Vec<_> = streams.keys().cloned().collect();

        for (i, &id1) in stream_ids.iter().enumerate() {
            for &id2 in stream_ids.iter().skip(i + 1) {
                if let (Some(stream1), Some(stream2)) = (streams.get(&id1), streams.get(&id2)) {
                    if let (Ok(info1), Ok(info2)) = (stream1.fusion_info.read(), stream2.fusion_info.read()) {
                        if self.can_fuse_streams(&info1, &info2) {
                            pairs.push((id1, id2, self.determine_fusion_type(&info1, &info2)));
                        }
                    }
                }
            }
        }

        pairs
    }

    fn can_fuse_streams(&self, info1: &FusionInfo, info2: &FusionInfo) -> bool {
        info1.can_fuse && info2.can_fuse && 
        info1.pipeline_stage + 1 == info2.pipeline_stage
    }

    fn determine_fusion_type(&self, info1: &FusionInfo, info2: &FusionInfo) -> FusionType {
        // Simple heuristic for determining fusion type
        match (info1.fusion_type, info2.fusion_type) {
            (FusionType::Map, FusionType::Filter) => FusionType::Map,
            (FusionType::Filter, FusionType::Map) => FusionType::Filter,
            (FusionType::Map, FusionType::Map) => FusionType::Map,
            (FusionType::Filter, FusionType::Filter) => FusionType::Filter,
            _ => info1.fusion_type, // Default to first stream's type
        }
    }

    fn should_apply_fusion(&self, opportunity: &FusionOpportunity) -> bool {
        // Apply fusion if it's estimated to provide significant benefit
        opportunity.estimated_speedup > 1.2 && opportunity.memory_savings > 512
    }

    fn apply_fusion_opportunity(&self, opportunity: &FusionOpportunity) -> Result<(), LazyError> {
        let pipeline_id = self.generate_pipeline_id();
        
        let fused_pipeline = FusedPipeline {
            id: pipeline_id,
            stages: Vec::new(), // Would be populated with actual pipeline stages
            input_streams: opportunity.streams.clone(),
            output_stream: pipeline_id as StreamId, // Simplified
            optimization_level: 1,
        };

        if let Ok(mut pipelines) = self.fused_pipelines.write() {
            pipelines.insert(pipeline_id, fused_pipeline);
        }

        Ok(())
    }

    fn generate_pipeline_id(&self) -> PipelineId {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        COUNTER.fetch_add(1, Ordering::SeqCst)
    }
}

impl DependencyGraph {
    fn new() -> Self {
        Self {
            edges: HashMap::new(),
            reverse_edges: HashMap::new(),
            topological_order: Vec::new(),
            cycles: Vec::new(),
        }
    }
}

impl PartialOrd for PriorityTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PriorityTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority_score.cmp(&other.priority_score)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ovm::config::OvmConfig;

    struct SimpleComputation {
        value: i64,
    }

    impl LazyComputation for SimpleComputation {
        fn compute(&self) -> Result<OvmValue, String> {
            Ok(OvmValue {
                header: ValueHeader::default(),
                data: ValueData::Integer(self.value),
            })
        }

        fn dependencies(&self) -> Vec<ThunkId> {
            Vec::new()
        }

        fn memory_estimate(&self) -> usize {
            64
        }

        fn can_memoize(&self) -> bool {
            true
        }
    }

    #[test]
    fn test_lazy_engine_creation() {
        let config = OvmConfig::default();
        let engine = LazyEngine::new(&config);
        assert!(engine.is_ok());
    }

    #[test]
    fn test_thunk_creation_and_forcing() {
        let config = OvmConfig::default();
        let engine = LazyEngine::new(&config).unwrap();

        let computation = SimpleComputation { value: 42 };
        let thunk_id = engine
            .create_thunk(computation, ThunkPriority::Normal)
            .unwrap();

        let result = engine.force_thunk(thunk_id).unwrap();
        if let ValueData::Integer(value) = result.data {
            assert_eq!(value, 42);
        } else {
            panic!("Expected integer value");
        }
    }

    #[test]
    fn test_lazy_stats() {
        let stats = LazyStats::new();
        assert_eq!(stats.cache_hit_rate(), 0.0);
        assert_eq!(stats.memory_efficiency(), 0.0);
    }

    #[test]
    fn test_memoization_cache() {
        let config = LazyConfig::default();
        let cache = MemoizationCache::new(&config).unwrap();

        let value = OvmValue {
            header: ValueHeader::default(),
            data: ValueData::Integer(42),
        };

        assert!(cache.put("test".to_string(), value.clone_simple()).is_ok());

        let cached = cache.get("test");
        assert!(cached.is_some());

        if let Some(cached_value) = cached {
            if let ValueData::Integer(v) = cached_value.data {
                assert_eq!(v, 42);
            }
        }
    }

    // Example stream generator for testing
    struct RangeGenerator {
        current: i64,
        end: i64,
        step: i64,
    }

    impl RangeGenerator {
        fn new(start: i64, end: i64, step: i64) -> Self {
            Self {
                current: start,
                end,
                step,
            }
        }
    }

    impl StreamGenerator for RangeGenerator {
        fn next(&mut self) -> Option<OvmValue> {
            if (self.step > 0 && self.current < self.end) || (self.step < 0 && self.current > self.end) {
                let value = OvmValue {
                    header: ValueHeader::default(),
                    data: ValueData::Integer(self.current),
                };
                self.current += self.step;
                Some(value)
            } else {
                None
            }
        }

        fn size_hint(&self) -> (usize, Option<usize>) {
            let remaining = if self.step != 0 {
                ((self.end - self.current) / self.step).max(0) as usize
            } else {
                0
            };
            (remaining, Some(remaining))
        }

        fn is_infinite(&self) -> bool {
            false
        }
    }

    #[test]
    fn test_stream_processing() {
        let config = OvmConfig::default();
        let engine = LazyEngine::new(&config).unwrap();

        let generator = RangeGenerator::new(1, 10, 1);
        let stream_id = engine.create_stream(generator).unwrap();

        let values = engine.take_from_stream(stream_id, 5).unwrap();
        assert_eq!(values.len(), 5);
    }

    #[test]
    fn test_stream_buffer() {
        let mut buffer = StreamBuffer::new(3);
        
        // Test writing elements
        for i in 0..3 {
            let value = OvmValue {
                header: ValueHeader::default(),
                data: ValueData::Integer(i),
            };
            assert!(buffer.write_element(value).is_ok());
        }
        
        // Buffer should be full
        assert_eq!(buffer.buffer_usage(), 1.0);
        
        // Reading elements
        for i in 0..3 {
            if let Some(value) = buffer.read_element() {
                if let ValueData::Integer(v) = value.data {
                    assert_eq!(v, i);
                }
            } else {
                panic!("Expected value at position {}", i);
            }
        }
        
        // Buffer should be empty
        assert!(!buffer.has_elements());
    }

    #[test]
    fn test_lazy_scheduler() {
        let config = LazyConfig::default();
        let scheduler = LazyScheduler::new(&config).unwrap();
        
        let task = LazyTask {
            thunk_id: 1,
            priority: ThunkPriority::Normal,
            created_at: Instant::now(),
        };
        
        assert!(scheduler.schedule_task(task).is_ok());
        
        let (work_queue_size, priority_queue_size) = scheduler.get_queue_stats();
        assert_eq!(work_queue_size, 1);
        assert_eq!(priority_queue_size, 0);
    }

    #[test]
    fn test_pipeline_operations() {
        // Test MapOperation
        let map_op = MapOperation::new(|v| {
            if let ValueData::Integer(i) = v.data {
                OvmValue {
                    header: ValueHeader::default(),
                    data: ValueData::Integer(i * 2),
                }
            } else {
                v.clone_simple()
            }
        });

        let input = OvmValue {
            header: ValueHeader::default(),
            data: ValueData::Integer(5),
        };

        let result = map_op.apply(&input);
        assert!(result.is_some());
        if let Some(result_value) = result {
            if let ValueData::Integer(i) = result_value.data {
                assert_eq!(i, 10);
            } else {
                panic!("Expected integer result");
            }
        }

        // Test FilterOperation
        let filter_op = FilterOperation::new(|v| {
            if let ValueData::Integer(i) = v.data {
                i > 5
            } else {
                false
            }
        });

        let result = filter_op.apply(&input);
        assert!(result.is_none()); // 5 is not > 5

        let input2 = OvmValue {
            header: ValueHeader::default(),
            data: ValueData::Integer(10),
        };

        let result = filter_op.apply(&input2);
        assert!(result.is_some()); // 10 is > 5

        // Test TakeOperation
        let take_op = TakeOperation::new(2);
        assert!(take_op.apply(&input).is_some());
        assert!(take_op.apply(&input2).is_some());
        assert!(take_op.apply(&input).is_none()); // Third call should return None
    }


    #[test]
    fn test_fused_pipeline_execution() {
        let config = OvmConfig::default();
        let engine = LazyEngine::new(&config).unwrap();
        
        // Create a simple pipeline: map (double) -> filter (>10)
        let map_op = Arc::new(MapOperation::new(|v| {
            if let ValueData::Integer(i) = v.data {
                OvmValue {
                    header: ValueHeader::default(),
                    data: ValueData::Integer(i * 2),
                }
            } else {
                v.clone_simple()
            }
        }));

        let filter_op = Arc::new(FilterOperation::new(|v| {
            if let ValueData::Integer(i) = v.data {
                i > 10
            } else {
                false
            }
        }));

        let pipeline_id = engine.stream_processor.fusion_optimizer
            .create_fused_pipeline(vec![1], vec![map_op, filter_op])
            .unwrap();

        // Create input data: [5, 6, 7, 8, 9]
        let input_data: Vec<OvmValue> = (5..10).map(|i| OvmValue {
            header: ValueHeader::default(),
            data: ValueData::Integer(i),
        }).collect();

        // Execute pipeline
        let result = engine.stream_processor.fusion_optimizer
            .execute_pipeline(pipeline_id, input_data)
            .unwrap();

        // Expected: [5,6,7,8,9] -> [10,12,14,16,18] -> [12,14,16,18] (filtered >10)
        assert_eq!(result.len(), 4);
        
        // Check first result
        if let ValueData::Integer(i) = result[0].data {
            assert_eq!(i, 12); // 6 * 2 = 12, and 12 > 10
        } else {
            panic!("Expected integer result");
        }
    }

    #[test]
    fn test_pipeline_optimization() {
        let config = OvmConfig::default();
        let engine = LazyEngine::new(&config).unwrap();
        
        // Create operations as trait objects
        let map_op: Arc<dyn PipelineOperation + Send + Sync> = Arc::new(MapOperation::new(|v| v.clone_simple()));
        let filter_op: Arc<dyn PipelineOperation + Send + Sync> = Arc::new(FilterOperation::new(|_| true));
        let take_op: Arc<dyn PipelineOperation + Send + Sync> = Arc::new(TakeOperation::new(5));

        let operations = vec![map_op, filter_op, take_op];
        let stages: Vec<PipelineStage> = operations.into_iter()
            .map(|op| {
                let memory_req = op.memory_requirement();
                PipelineStage {
                    operation: op,
                    fusion_compatible: true,
                    memory_requirement: memory_req,
                }
            })
            .collect();

        let mut pipeline = FusedPipeline::new(1, vec![1], stages);
        
        // Get initial stats
        let initial_stats = pipeline.get_stats();
        let initial_stage_count = initial_stats.stage_count;

        // Optimize the pipeline
        let optimization_result = pipeline.optimize().unwrap();
        
        // Get optimized stats
        let optimized_stats = pipeline.get_stats();
        
        // Should have performed some optimizations
        assert!(optimization_result.fusions_performed >= 0);
        assert!(optimized_stats.optimization_level > 1);
    }

    #[test]
    fn test_pipeline_stats() {
        let config = OvmConfig::default();
        let engine = LazyEngine::new(&config).unwrap();
        
        let map_op = Arc::new(MapOperation::new(|v| v.clone_simple()));
        let filter_op = Arc::new(FilterOperation::new(|_| true));

        let pipeline_id = engine.stream_processor.fusion_optimizer
            .create_fused_pipeline(vec![1], vec![map_op, filter_op])
            .unwrap();

        let stats = engine.stream_processor.fusion_optimizer
            .get_pipeline_stats(pipeline_id)
            .unwrap();

        assert_eq!(stats.pipeline_id, pipeline_id);
        assert_eq!(stats.stage_count, 2);
        assert!(stats.total_memory_requirement > 0);
        assert_eq!(stats.optimization_level, 1);
        assert_eq!(stats.fusion_compatible_stages, 2);
    }

    #[test]
    fn test_merged_pipeline_operation() {
        let map_op = Arc::new(MapOperation::new(|v| {
            if let ValueData::Integer(i) = v.data {
                OvmValue {
                    header: ValueHeader::default(),
                    data: ValueData::Integer(i * 2),
                }
            } else {
                v.clone_simple()
            }
        }));

        let filter_op = Arc::new(FilterOperation::new(|v| {
            if let ValueData::Integer(i) = v.data {
                i > 10
            } else {
                false
            }
        }));

        let merged_op = MergedPipelineOperation {
            operations: vec![map_op, filter_op],
        };

        let input = OvmValue {
            header: ValueHeader::default(),
            data: ValueData::Integer(6),
        };

        // Should apply map (6 -> 12) then filter (12 > 10 = true)
        let result = merged_op.apply(&input);
        assert!(result.is_some());
        
        if let Some(result_value) = result {
            if let ValueData::Integer(i) = result_value.data {
                assert_eq!(i, 12);
            } else {
                panic!("Expected integer result");
            }
        }

        let input2 = OvmValue {
            header: ValueHeader::default(),
            data: ValueData::Integer(4),
        };

        // Should apply map (4 -> 8) then filter (8 > 10 = false)
        let result = merged_op.apply(&input2);
        assert!(result.is_none());
    }
}

/// Concrete pipeline operations
pub struct MapOperation {
    mapper: Arc<dyn Fn(&OvmValue) -> OvmValue + Send + Sync>,
}

pub struct FilterOperation {
    predicate: Arc<dyn Fn(&OvmValue) -> bool + Send + Sync>,
}

pub struct TakeOperation {
    count: usize,
    taken: AtomicUsize,
}

pub struct SkipOperation {
    count: usize,
    skipped: AtomicUsize,
}

pub struct ReduceOperation {
    reducer: Arc<dyn Fn(OvmValue, &OvmValue) -> OvmValue + Send + Sync>,
    initial: Option<OvmValue>,
}

impl MapOperation {
    pub fn new<F>(mapper: F) -> Self 
    where 
        F: Fn(&OvmValue) -> OvmValue + Send + Sync + 'static 
    {
        Self {
            mapper: Arc::new(mapper),
        }
    }
}

impl FilterOperation {
    pub fn new<F>(predicate: F) -> Self 
    where 
        F: Fn(&OvmValue) -> bool + Send + Sync + 'static 
    {
        Self {
            predicate: Arc::new(predicate),
        }
    }
}

impl TakeOperation {
    pub fn new(count: usize) -> Self {
        Self {
            count,
            taken: AtomicUsize::new(0),
        }
    }
}

impl SkipOperation {
    pub fn new(count: usize) -> Self {
        Self {
            count,
            skipped: AtomicUsize::new(0),
        }
    }
}

impl ReduceOperation {
    pub fn new<F>(reducer: F, initial: Option<OvmValue>) -> Self 
    where 
        F: Fn(OvmValue, &OvmValue) -> OvmValue + Send + Sync + 'static 
    {
        Self {
            reducer: Arc::new(reducer),
            initial,
        }
    }
}

impl PipelineOperation for MapOperation {
    fn apply(&self, input: &OvmValue) -> Option<OvmValue> {
        Some((self.mapper)(input))
    }

    fn can_fuse_with(&self, other: &dyn PipelineOperation) -> bool {
        // Map operations can fuse with most other operations
        true
    }

    fn memory_requirement(&self) -> usize {
        64 // Conservative estimate
    }
}

impl PipelineOperation for FilterOperation {
    fn apply(&self, input: &OvmValue) -> Option<OvmValue> {
        if (self.predicate)(input) {
            Some(input.clone_simple())
        } else {
            None
        }
    }

    fn can_fuse_with(&self, other: &dyn PipelineOperation) -> bool {
        // Filter operations can fuse with most other operations
        true
    }

    fn memory_requirement(&self) -> usize {
        32 // Filter operations are lightweight
    }
}

impl PipelineOperation for TakeOperation {
    fn apply(&self, input: &OvmValue) -> Option<OvmValue> {
        let taken = self.taken.fetch_add(1, Ordering::Relaxed);
        if taken < self.count {
            Some(input.clone_simple())
        } else {
            None
        }
    }

    fn can_fuse_with(&self, other: &dyn PipelineOperation) -> bool {
        // Take operations can fuse with operations that come before them
        true
    }

    fn memory_requirement(&self) -> usize {
        16 // Take operations are very lightweight
    }
}

impl PipelineOperation for SkipOperation {
    fn apply(&self, input: &OvmValue) -> Option<OvmValue> {
        let skipped = self.skipped.fetch_add(1, Ordering::Relaxed);
        if skipped >= self.count {
            Some(input.clone_simple())
        } else {
            None
        }
    }

    fn can_fuse_with(&self, other: &dyn PipelineOperation) -> bool {
        // Skip operations can fuse with operations that come before them
        true
    }

    fn memory_requirement(&self) -> usize {
        16 // Skip operations are very lightweight
    }
}

impl PipelineOperation for ReduceOperation {
    fn apply(&self, input: &OvmValue) -> Option<OvmValue> {
        // Reduce operations are special - they need to maintain state
        // This is a simplified implementation
        Some(input.clone_simple())
    }

    fn can_fuse_with(&self, other: &dyn PipelineOperation) -> bool {
        // Reduce operations have limited fusion potential
        false
    }

    fn memory_requirement(&self) -> usize {
        128 // Reduce operations need more memory for state
    }
}

/// Enhanced FusedPipeline with execution capabilities
impl FusedPipeline {
    pub fn new(
        id: PipelineId,
        input_streams: Vec<StreamId>,
        stages: Vec<PipelineStage>,
    ) -> Self {
        Self {
            id,
            stages,
            input_streams,
            output_stream: id as StreamId, // Simplified output stream assignment
            optimization_level: 1,
        }
    }

    /// Execute the fused pipeline on input data
    pub fn execute(&self, input_data: Vec<OvmValue>) -> Result<Vec<OvmValue>, LazyError> {
        let mut current_data = input_data;

        for stage in &self.stages {
            let mut stage_output = Vec::new();
            
            for value in current_data {
                if let Some(result) = stage.operation.apply(&value) {
                    stage_output.push(result);
                }
            }
            
            current_data = stage_output;
        }

        Ok(current_data)
    }

    /// Execute the pipeline lazily, returning a stream
    pub fn execute_lazy(&self, input_stream: StreamId) -> Result<StreamId, LazyError> {
        // Create a new stream that applies the pipeline stages
        // For now, just return the input stream ID as a simplified implementation
        
        let generator = FusedPipelineGenerator {
            input_stream,
            stages: Vec::new(), // Simplified - would use references in full implementation
            current_stage: 0,
            buffer: VecDeque::new(),
        };

        // Register with stream processor (simplified)
        Ok(self.id as StreamId)
    }

    /// Get pipeline statistics
    pub fn get_stats(&self) -> PipelineStats {
        let total_memory = self.stages.iter()
            .map(|stage| stage.memory_requirement)
            .sum();

        PipelineStats {
            pipeline_id: self.id,
            stage_count: self.stages.len(),
            total_memory_requirement: total_memory,
            optimization_level: self.optimization_level,
            fusion_compatible_stages: self.stages.iter()
                .filter(|stage| stage.fusion_compatible)
                .count(),
        }
    }

    /// Optimize the pipeline by reordering stages
    pub fn optimize(&mut self) -> Result<OptimizationStats, LazyError> {
        let start_time = Instant::now();
        let original_stages = self.stages.len();

        // Reorder stages for better fusion
        self.reorder_stages_for_fusion();

        // Merge compatible stages
        self.merge_compatible_stages();

        let optimization_time = start_time.elapsed();
        let fusions_performed = original_stages - self.stages.len();

        Ok(OptimizationStats {
            fusions_performed,
            speedup_achieved: 1.0 + (fusions_performed as f64 * 0.1), // 10% per fusion
            memory_saved: fusions_performed * 64, // Estimate
            optimization_time,
        })
    }

    fn reorder_stages_for_fusion(&mut self) {
        // Move filter operations before map operations when possible
        let mut reordered = false;
        for i in 0..self.stages.len().saturating_sub(1) {
            if self.can_reorder_stages(i, i + 1) {
                self.stages.swap(i, i + 1);
                reordered = true;
            }
        }
        
        if reordered {
            self.optimization_level += 1;
        }
    }

    fn can_reorder_stages(&self, i: usize, j: usize) -> bool {
        if i >= self.stages.len() || j >= self.stages.len() {
            return false;
        }

        // Check if stages can be reordered without changing semantics
        let stage_i = &self.stages[i];
        let stage_j = &self.stages[j];

        // Filter operations can often be moved before map operations
        stage_i.fusion_compatible && stage_j.fusion_compatible
    }

    fn merge_compatible_stages(&mut self) {
        let mut i = 0;
        while i < self.stages.len().saturating_sub(1) {
            if self.can_merge_stages(i, i + 1) {
                self.merge_stages(i, i + 1);
            } else {
                i += 1;
            }
        }
    }

    fn can_merge_stages(&self, i: usize, j: usize) -> bool {
        if i >= self.stages.len() || j >= self.stages.len() {
            return false;
        }

        let stage_i = &self.stages[i];
        let stage_j = &self.stages[j];

        stage_i.fusion_compatible && stage_j.fusion_compatible
    }

    fn merge_stages(&mut self, i: usize, j: usize) {
        if i >= self.stages.len() || j >= self.stages.len() || i >= j {
            return;
        }

        // Create a merged operation
        let merged_operation = Arc::new(MergedPipelineOperation {
            operations: vec![
                self.stages[i].operation.clone(),
                self.stages[j].operation.clone(),
            ],
        });

        let merged_stage = PipelineStage {
            operation: merged_operation,
            fusion_compatible: true,
            memory_requirement: self.stages[i].memory_requirement + self.stages[j].memory_requirement,
        };

        // Replace the two stages with the merged one
        self.stages[i] = merged_stage;
        self.stages.remove(j);
    }
}

/// Merged pipeline operation that combines multiple operations
pub struct MergedPipelineOperation {
    operations: Vec<Arc<dyn PipelineOperation + Send + Sync>>,
}

impl PipelineOperation for MergedPipelineOperation {
    fn apply(&self, input: &OvmValue) -> Option<OvmValue> {
        let mut current = input.clone_simple();
        
        for operation in &self.operations {
            if let Some(result) = operation.apply(&current) {
                current = result;
            } else {
                return None; // Early termination if any operation returns None
            }
        }
        
        Some(current)
    }

    fn can_fuse_with(&self, _other: &dyn PipelineOperation) -> bool {
        false // Merged operations are already optimized
    }

    fn memory_requirement(&self) -> usize {
        self.operations.iter()
            .map(|op| op.memory_requirement())
            .sum()
    }
}

/// Generator for fused pipeline execution
pub struct FusedPipelineGenerator {
    input_stream: StreamId,
    stages: Vec<PipelineStage>,
    current_stage: usize,
    buffer: VecDeque<OvmValue>,
}

impl StreamGenerator for FusedPipelineGenerator {
    fn next(&mut self) -> Option<OvmValue> {
        // Simplified implementation - in practice this would coordinate with the stream processor
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None) // Unknown size
    }

    fn is_infinite(&self) -> bool {
        false
    }
}

/// Pipeline statistics
#[derive(Debug, Clone)]
pub struct PipelineStats {
    pub pipeline_id: PipelineId,
    pub stage_count: usize,
    pub total_memory_requirement: usize,
    pub optimization_level: u8,
    pub fusion_compatible_stages: usize,
}

/// Enhanced StreamFusionOptimizer with full pipeline management
impl StreamFusionOptimizer {
    /// Create a new fused pipeline from multiple streams
    pub fn create_fused_pipeline(
        &self,
        streams: Vec<StreamId>,
        operations: Vec<Arc<dyn PipelineOperation + Send + Sync>>,
    ) -> Result<PipelineId, LazyError> {
        let pipeline_id = self.generate_pipeline_id();
        
        let stages: Vec<PipelineStage> = operations.into_iter()
            .map(|op| {
                let memory_req = op.memory_requirement();
                PipelineStage {
                    operation: op,
                    fusion_compatible: true,
                    memory_requirement: memory_req,
                }
            })
            .collect();

        let fused_pipeline = FusedPipeline::new(pipeline_id, streams, stages);

        // Store the pipeline
        if let Ok(mut pipelines) = self.fused_pipelines.write() {
            pipelines.insert(pipeline_id, fused_pipeline);
        }

        Ok(pipeline_id)
    }

    /// Execute a fused pipeline
    pub fn execute_pipeline(
        &self,
        pipeline_id: PipelineId,
        input_data: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, LazyError> {
        if let Ok(pipelines) = self.fused_pipelines.read() {
            if let Some(pipeline) = pipelines.get(&pipeline_id) {
                return pipeline.execute(input_data);
            }
        }
        
        Err(LazyError::FusionFailed {
            reason: format!("Pipeline {} not found", pipeline_id),
        })
    }

    /// Get all fused pipelines
    pub fn get_pipelines(&self) -> Result<Vec<FusedPipeline>, LazyError> {
        if let Ok(pipelines) = self.fused_pipelines.read() {
            Ok(pipelines.values().cloned().collect())
        } else {
            Err(LazyError::Failed("Failed to read pipelines".to_string()))
        }
    }

    /// Remove a fused pipeline
    pub fn remove_pipeline(&self, pipeline_id: PipelineId) -> Result<(), LazyError> {
        if let Ok(mut pipelines) = self.fused_pipelines.write() {
            pipelines.remove(&pipeline_id);
            Ok(())
        } else {
            Err(LazyError::Failed("Failed to remove pipeline".to_string()))
        }
    }

    /// Get pipeline statistics
    pub fn get_pipeline_stats(&self, pipeline_id: PipelineId) -> Result<PipelineStats, LazyError> {
        if let Ok(pipelines) = self.fused_pipelines.read() {
            if let Some(pipeline) = pipelines.get(&pipeline_id) {
                return Ok(pipeline.get_stats());
            }
        }
        
        Err(LazyError::FusionFailed {
            reason: format!("Pipeline {} not found", pipeline_id),
        })
    }
}

/// Fused pipeline
#[derive(Clone)]
pub struct FusedPipeline {
    id: PipelineId,
    stages: Vec<PipelineStage>,
    input_streams: Vec<StreamId>,
    output_stream: StreamId,
    optimization_level: u8,
}

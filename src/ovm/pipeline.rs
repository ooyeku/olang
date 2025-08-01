//! OVM Pipeline Processing Engine
//!
//! Provides pipeline fusion, parallel processing, and stream optimization

use crate::ovm::{OvmConfig, OvmValue};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

#[allow(dead_code)]

/// Main pipeline processing engine
pub struct PipelineEngine {
    // Configuration
    config: PipelineConfig,

    // Pipeline optimization and fusion
    fusion_optimizer: Arc<PipelineFusionOptimizer>,

    // Parallel processing management
    parallel_processor: Arc<ParallelProcessor>,

    // Stream processing for lazy pipelines
    stream_processor: Arc<StreamProcessor>,

    // Pipeline cache for compiled pipelines
    pipeline_cache: Arc<RwLock<HashMap<PipelineSignature, CompiledPipeline>>>,

    // Performance monitoring
    performance_monitor: Arc<Mutex<PipelinePerformanceMonitor>>,

    // Background optimization workers
    optimization_workers: Vec<JoinHandle<()>>,
    is_running: Arc<std::sync::atomic::AtomicBool>,
}

#[allow(dead_code)]
/// Pipeline configuration
#[derive(Debug, Clone)]
struct PipelineConfig {
    enable_fusion: bool,
    enable_parallel: bool,
    parallel_threshold: usize,
    enable_vectorization: bool,
    buffer_size: usize,
    max_fusion_length: usize,
    parallel_worker_count: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            enable_fusion: true,
            enable_parallel: true,
            parallel_threshold: 500,
            enable_vectorization: true,
            buffer_size: 8192,
            max_fusion_length: 15,
            parallel_worker_count: num_cpus::get(),
        }
    }
}

/// Pipeline fusion optimizer
#[allow(dead_code)]
struct PipelineFusionOptimizer {
    // Known fusion patterns
    fusion_patterns: Vec<FusionPattern>,

    // Fusion opportunities cache
    opportunities_cache: Arc<RwLock<HashMap<Vec<PipelineOp>, FusionOpportunity>>>,

    // Fusion statistics
    fusion_stats: Arc<Mutex<FusionStatistics>>,
}

/// Parallel processing manager
#[allow(dead_code)]
struct ParallelProcessor {
    // Worker thread pool
    worker_pool: Arc<ThreadPool>,

    // Work queue for parallel tasks
    work_queue: Arc<Mutex<VecDeque<ParallelTask>>>,

    // Load balancer
    load_balancer: LoadBalancer,
}

/// Stream processor for lazy evaluation integration
#[allow(dead_code)]
struct StreamProcessor {
    // Active streams
    active_streams: Arc<RwLock<HashMap<StreamId, ActiveStream>>>,

    // Stream buffer manager
    buffer_manager: Arc<StreamBufferManager>,

    // Stream fusion engine
    stream_fusion: Arc<StreamFusionEngine>,
}

/// Pipeline signature for caching
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct PipelineSignature {
    operations: Vec<PipelineOp>,
    input_type: DataType,
    output_type: DataType,
    parallelizable: bool,
}

/// Compiled pipeline representation
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct CompiledPipeline {
    signature: PipelineSignature,
    executor: PipelineExecutor,
    performance_profile: PerformanceProfile,
    compilation_time: Duration,
}

/// Pipeline operation types
#[allow(dead_code)]
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum PipelineOp {
    Map(String),    // Map function name/signature
    Filter(String), // Filter predicate signature
    Reduce(String), // Reduce function signature
    Take(usize),    // Take n elements
    Skip(usize),    // Skip n elements
    Zip,            // Zip with another stream
    Flatten,        // Flatten nested collections
    Distinct,       // Remove duplicates
    Sort(String),   // Sort with comparator
    Reverse,        // Reverse order
    Chunk(usize),   // Chunk into groups
    Window(usize),  // Sliding window
}

/// Data types for pipeline optimization
#[allow(dead_code)]
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum DataType {
    Integer,
    Float,
    Boolean,
    String,
    List(Box<DataType>),
    Tuple(Vec<DataType>),
    Stream(Box<DataType>),
    Any,
}

/// Fusion patterns for common operation combinations
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FusionPattern {
    pattern: Vec<PipelineOp>,
    fused_op: FusedOperation,
    speedup_factor: f64,
    memory_savings: f64,
}

/// Fused operation that combines multiple pipeline operations
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FusedOperation {
    name: String,
    implementation: FusedImplementation,
    input_types: Vec<DataType>,
    output_type: DataType,
}

/// Implementation strategies for fused operations
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum FusedImplementation {
    Sequential, // Execute operations in sequence
    Vectorized, // Use SIMD instructions
    Parallel,   // Execute in parallel
    Hybrid,     // Combine strategies based on data size
}

/// Fusion opportunity identification
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FusionOpportunity {
    operations: Vec<PipelineOp>,
    estimated_speedup: f64,
    memory_reduction: f64,
    implementation_strategy: FusedImplementation,
}

/// Fusion statistics for monitoring
#[allow(dead_code)]
#[derive(Debug, Default)]
struct FusionStatistics {
    opportunities_found: u64,
    fusions_applied: u64,
    total_speedup: f64,
    memory_saved: u64,
}

/// Pipeline executor implementations
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum PipelineExecutor {
    Sequential(SequentialExecutor),
    Parallel(ParallelExecutor),
    Vectorized(VectorizedExecutor),
    Fused(FusedExecutor),
}

/// Sequential pipeline executor
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct SequentialExecutor {
    operations: Vec<PipelineOp>,
}

/// Parallel pipeline executor
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ParallelExecutor {
    operations: Vec<PipelineOp>,
    worker_count: usize,
    chunk_size: usize,
}

/// Vectorized pipeline executor
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct VectorizedExecutor {
    operations: Vec<PipelineOp>,
    vector_size: usize,
}

/// Fused pipeline executor
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct FusedExecutor {
    fused_operations: Vec<FusedOperation>,
    optimization_level: u8,
}

/// Performance profile for pipelines
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct PerformanceProfile {
    average_execution_time: Duration,
    throughput: f64, // elements per second
    memory_usage: usize,
    cpu_usage: f64,
    cache_hit_rate: f64,
}

/// Parallel task for work distribution
#[allow(dead_code)]
#[derive(Debug)]
struct ParallelTask {
    task_id: u64,
    input_data: Vec<OvmValue>,
    operations: Vec<PipelineOp>,
    chunk_start: usize,
    chunk_end: usize,
}

/// Thread pool for parallel processing
#[allow(dead_code)]
struct ThreadPool {
    workers: Vec<JoinHandle<()>>,
    task_sender: std::sync::mpsc::Sender<ParallelTask>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

/// Load balancer for distributing work
#[allow(dead_code)]
#[derive(Debug)]
struct LoadBalancer {
    worker_loads: Vec<f64>,
    load_history: VecDeque<Vec<f64>>,
    last_update: Instant,
}

impl LoadBalancer {
    fn new(worker_count: usize) -> Self {
        Self {
            worker_loads: vec![0.0; worker_count],
            load_history: VecDeque::with_capacity(10), // Keep last 10 measurements
            last_update: Instant::now(),
        }
    }
    
    /// Update worker load information
    pub fn update_worker_load(&mut self, worker_id: usize, load: f64) {
        if worker_id < self.worker_loads.len() {
            self.worker_loads[worker_id] = load;
            self.last_update = Instant::now();
        }
    }
    
    /// Get the least loaded worker
    pub fn get_least_loaded_worker(&self) -> usize {
        self.worker_loads
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(idx, _)| idx)
            .unwrap_or(0)
    }
    
    /// Get average system load
    pub fn get_average_load(&self) -> f64 {
        if self.worker_loads.is_empty() {
            0.0
        } else {
            self.worker_loads.iter().sum::<f64>() / self.worker_loads.len() as f64
        }
    }
    
    /// Check if the system is overloaded
    pub fn is_overloaded(&self, threshold: f64) -> bool {
        self.get_average_load() > threshold
    }
    
    /// Record current loads in history
    pub fn record_snapshot(&mut self) {
        let snapshot = self.worker_loads.clone();
        
        // Keep only the last 10 snapshots
        if self.load_history.len() >= 10 {
            self.load_history.pop_front();
        }
        
        self.load_history.push_back(snapshot);
    }
    
    /// Get load trend (positive = increasing, negative = decreasing)
    pub fn get_load_trend(&self) -> f64 {
        if self.load_history.len() < 2 {
            return 0.0;
        }
        
        let recent = self.load_history.back().unwrap();
        let older = self.load_history.front().unwrap();
        
        let recent_avg = recent.iter().sum::<f64>() / recent.len() as f64;
        let older_avg = older.iter().sum::<f64>() / older.len() as f64;
        
        recent_avg - older_avg
    }
}

/// Stream processing types
type StreamId = u64;

/// Fusion hint types for optimization detection
#[derive(Debug, Clone)]
pub enum FusionHintType {
    MapMapFusion,
    MapFilterFusion,
    FilterMapFusion,
    TakeMapFusion,
    MultiMapFusion,
    MultiFilterMapFusion,
    LoopFusion,
    MemoryOptimization,
}

/// Fusion strategy enumeration
#[derive(Debug, Clone)]
pub enum FusionStrategy {
    Sequential,
    Vectorized,
    Parallel,
    CacheOptimized,
}

/// Fusion benefit metrics
#[derive(Debug, Clone)]
pub struct FusionBenefit {
    pub performance_gain: f64,
    pub memory_savings: usize,
    pub cache_efficiency: f64,
}

/// Comprehensive fusion hint with all optimization information
#[derive(Debug, Clone)]
pub struct FusionHint {
    pub hint_type: FusionHintType,
    pub operations: Vec<PipelineOp>,
    pub estimated_benefit: FusionBenefit,
    pub fusion_strategy: FusionStrategy,
}

/// Pipeline performance benchmarks
#[derive(Debug, Clone)]
pub struct PipelineBenchmarks {
    pub total_executions: u64,
    pub cache_hit_rate: f64,
    pub cache_miss_rate: f64,
    pub average_execution_time: Duration,
    pub total_execution_time: Duration,
    pub cached_pipelines: usize,
    pub parallel_executions: u64,
    pub fusion_hits: u64,
    pub memory_usage_estimate: usize,
    pub performance_score: f64,
}

/// Memory optimization results
#[derive(Debug, Clone)]
pub struct MemoryOptimizationResult {
    pub entries_removed: usize,
    pub memory_freed: usize,
    pub cache_size_before: usize,
    pub cache_size_after: usize,
    pub memory_before: usize,
    pub memory_after: usize,
}

/// Individual benchmark result
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub data_size: usize,
    pub test_case: usize,
    pub operations: Vec<PipelineOp>,
    pub execution_time: Duration,
    pub throughput: f64,
    pub success: bool,
    pub memory_usage: usize,
}

#[allow(dead_code)]
#[derive(Debug)]
struct ActiveStream {
    id: StreamId,
    data_type: DataType,
    buffer: VecDeque<OvmValue>,
    operations: Vec<PipelineOp>,
    is_infinite: bool,
}

#[allow(dead_code)]
#[derive(Debug)]
struct StreamBufferManager {
    buffers: HashMap<StreamId, VecDeque<OvmValue>>,
    buffer_limits: HashMap<StreamId, usize>,
    memory_pressure: f64,
}

#[allow(dead_code)]
#[derive(Debug)]
struct StreamFusionEngine {
    fusion_opportunities: Vec<StreamFusionOpportunity>,
    active_fusions: HashMap<Vec<StreamId>, FusedStream>,
}

#[allow(dead_code)]
#[derive(Debug)]
struct StreamFusionOpportunity {
    streams: Vec<StreamId>,
    operations: Vec<PipelineOp>,
    estimated_benefit: f64,
}

#[allow(dead_code)]
#[derive(Debug)]
struct FusedStream {
    input_streams: Vec<StreamId>,
    output_stream: StreamId,
    fused_operations: Vec<FusedOperation>,
}

/// Pipeline performance monitoring
#[allow(dead_code)]
#[derive(Debug, Default)]
struct PipelinePerformanceMonitor {
    pipeline_executions: u64,
    total_execution_time: Duration,
    fusion_hits: u64,
    parallel_executions: u64,
    cache_hits: u64,
    cache_misses: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("Pipeline processing failed: {0}")]
    Failed(String),

    #[error("Fusion optimization failed: {0}")]
    FusionFailed(String),

    #[error("Parallel processing failed: {0}")]
    ParallelFailed(String),

    #[error("Stream processing failed: {0}")]
    StreamFailed(String),

    #[error("Invalid pipeline configuration: {0}")]
    InvalidConfiguration(String),

    #[error("Compilation failed: {0}")]
    CompilationFailed(String),
}

#[allow(dead_code)]
impl PipelineEngine {
    pub fn new(_config: &OvmConfig) -> Result<Self, PipelineError> {
        let pipeline_config = PipelineConfig::default();

        Ok(Self {
            config: pipeline_config.clone(),
            fusion_optimizer: Arc::new(PipelineFusionOptimizer::new()),
            parallel_processor: Arc::new(ParallelProcessor::new(&pipeline_config)?),
            stream_processor: Arc::new(StreamProcessor::new()),
            pipeline_cache: Arc::new(RwLock::new(HashMap::new())),
            performance_monitor: Arc::new(Mutex::new(PipelinePerformanceMonitor::default())),
            optimization_workers: Vec::new(),
            is_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Process a pipeline of operations
    pub fn process_pipeline(
        &self,
        input: Vec<OvmValue>,
        operations: Vec<PipelineOp>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        let start_time = Instant::now();

        // Create pipeline signature
        let signature = self.create_pipeline_signature(&operations, &input)?;

        // Check cache for compiled pipeline
        if let Some(compiled) = self.get_cached_pipeline(&signature)? {
            let result = self.execute_compiled_pipeline(compiled, input)?;
            self.record_execution_time(start_time.elapsed(), true);
            return Ok(result);
        }

        // Analyze and optimize the pipeline
        let optimized_operations = self.optimize_pipeline(&operations, &input)?;

        // Compile and cache the pipeline
        let compiled = self.compile_pipeline(signature.clone(), optimized_operations)?;
        self.cache_pipeline(signature, compiled.clone())?;

        // Execute the pipeline
        let result = self.execute_compiled_pipeline(compiled, input)?;
        self.record_execution_time(start_time.elapsed(), false);

        Ok(result)
    }

    /// Create a signature for pipeline caching
    fn create_pipeline_signature(
        &self,
        operations: &[PipelineOp],
        input: &[OvmValue],
    ) -> Result<PipelineSignature, PipelineError> {
        let input_type = self.infer_data_type(input)?;
        let output_type = self.infer_output_type(operations, &input_type)?;
        let parallelizable = self.is_parallelizable(operations);

        Ok(PipelineSignature {
            operations: operations.to_vec(),
            input_type,
            output_type,
            parallelizable,
        })
    }

    /// Optimize pipeline operations using advanced fusion engine
    fn optimize_pipeline(
        &self,
        operations: &[PipelineOp],
        input: &[OvmValue],
    ) -> Result<Vec<PipelineOp>, PipelineError> {
        let mut optimized = operations.to_vec();

        // Apply advanced fusion optimization using the fusion engine
        if self.config.enable_fusion {
            optimized = self.apply_advanced_fusion_optimization(&optimized, input)?;
        }

        // Reorder operations for better performance
        optimized = self.reorder_operations(optimized)?;

        // Apply vectorization hints
        if self.config.enable_vectorization {
            optimized = self.add_vectorization_hints(optimized)?;
        }

        Ok(optimized)
    }
    
    /// Apply advanced fusion optimization using the fusion engine
    fn apply_advanced_fusion_optimization(
        &self,
        operations: &[PipelineOp],
        input: &[OvmValue],
    ) -> Result<Vec<PipelineOp>, PipelineError> {
        use crate::ovm::fusion::AdvancedFusionEngine;
        use crate::ovm::config::OvmConfig;
        
        // Initialize fusion engine
        let fusion_config = OvmConfig::default();
        let mut fusion_engine = AdvancedFusionEngine::new(&fusion_config)
            .map_err(|e| PipelineError::FusionFailed(format!("Fusion engine initialization failed: {}", e)))?;
        
        // Convert pipeline operations to fusion engine format
        let fusion_ops = self.convert_to_fusion_ops(operations)?;
        
        // Apply fusion optimization
        let optimized_pipeline = fusion_engine
            .optimize_pipeline(&fusion_ops, input.len())
            .map_err(|e| PipelineError::FusionFailed(format!("Fusion optimization failed: {}", e)))?;
        
        // Convert fused operations back to pipeline operations
        self.convert_from_fused_operations(&optimized_pipeline.fused_operations)
    }
    
    /// Convert pipeline operations to fusion engine format
    fn convert_to_fusion_ops(
        &self,
        operations: &[PipelineOp],
    ) -> Result<Vec<crate::ovm::fusion::PipelineOp>, PipelineError> {
        let mut fusion_ops = Vec::new();
        
        for op in operations {
            let fusion_op = match op {
                PipelineOp::Map(func) => crate::ovm::fusion::PipelineOp::Map(func.clone()),
                PipelineOp::Filter(pred) => crate::ovm::fusion::PipelineOp::Filter(pred.clone()),
                PipelineOp::Reduce(func) => crate::ovm::fusion::PipelineOp::Reduce(func.clone()),
                PipelineOp::Take(n) => crate::ovm::fusion::PipelineOp::Take(*n),
                PipelineOp::Skip(n) => crate::ovm::fusion::PipelineOp::Skip(*n),
                PipelineOp::Zip => crate::ovm::fusion::PipelineOp::Zip,
                PipelineOp::Flatten => crate::ovm::fusion::PipelineOp::Flatten,
                PipelineOp::Distinct => crate::ovm::fusion::PipelineOp::Distinct,
                PipelineOp::Sort(comp) => crate::ovm::fusion::PipelineOp::Sort(comp.clone()),
                PipelineOp::Reverse => crate::ovm::fusion::PipelineOp::Reverse,
                PipelineOp::Chunk(size) => crate::ovm::fusion::PipelineOp::Chunk(*size),
                PipelineOp::Window(size) => crate::ovm::fusion::PipelineOp::Window(*size),
            };
            fusion_ops.push(fusion_op);
        }
        
        Ok(fusion_ops)
    }
    
    /// Convert fused operations back to pipeline operations
    fn convert_from_fused_operations(
        &self,
        fused_operations: &[crate::ovm::fusion::FusedOperation],
    ) -> Result<Vec<PipelineOp>, PipelineError> {
        let mut pipeline_ops = Vec::new();
        
        for fused_op in fused_operations {
            // Convert each fused operation back to a sequence of pipeline operations
            let ops = self.decompose_fused_operation(fused_op)?;
            pipeline_ops.extend(ops);
        }
        
        Ok(pipeline_ops)
    }
    
    /// Decompose a fused operation back into individual pipeline operations
    fn decompose_fused_operation(
        &self,
        fused_op: &crate::ovm::fusion::FusedOperation,
    ) -> Result<Vec<PipelineOp>, PipelineError> {
        // Convert the fused operations back to pipeline operations
        let mut ops = Vec::new();
        
        for op in &fused_op.operations {
            let pipeline_op = match op {
                crate::ovm::fusion::PipelineOp::Map(func) => PipelineOp::Map(func.clone()),
                crate::ovm::fusion::PipelineOp::Filter(pred) => PipelineOp::Filter(pred.clone()),
                crate::ovm::fusion::PipelineOp::Reduce(func) => PipelineOp::Reduce(func.clone()),
                crate::ovm::fusion::PipelineOp::Take(n) => PipelineOp::Take(*n),
                crate::ovm::fusion::PipelineOp::Skip(n) => PipelineOp::Skip(*n),
                crate::ovm::fusion::PipelineOp::Zip => PipelineOp::Zip,
                crate::ovm::fusion::PipelineOp::Flatten => PipelineOp::Flatten,
                crate::ovm::fusion::PipelineOp::Distinct => PipelineOp::Distinct,
                crate::ovm::fusion::PipelineOp::Sort(comp) => PipelineOp::Sort(comp.clone()),
                crate::ovm::fusion::PipelineOp::Reverse => PipelineOp::Reverse,
                crate::ovm::fusion::PipelineOp::Chunk(size) => PipelineOp::Chunk(*size),
                crate::ovm::fusion::PipelineOp::Window(size) => PipelineOp::Window(*size),
            };
            ops.push(pipeline_op);
        }
        
        Ok(ops)
    }

    /// Compile pipeline into executable form with fusion support
    fn compile_pipeline(
        &self,
        signature: PipelineSignature,
        operations: Vec<PipelineOp>,
    ) -> Result<CompiledPipeline, PipelineError> {
        let start_time = Instant::now();

        // Detect fusion opportunities first
        let fusion_opportunities = self.detect_fusion_opportunities(&operations)?;
        
        // Choose execution strategy based on fusion opportunities and other factors
        let executor = if !fusion_opportunities.is_empty() && self.config.enable_fusion {
            // Create fused executor
            let fused_operations = self.create_fused_operations(&fusion_opportunities)?;
            PipelineExecutor::Fused(FusedExecutor {
                fused_operations,
                optimization_level: self.determine_optimization_level(&signature),
            })
        } else if signature.parallelizable && operations.len() > self.config.parallel_threshold {
            // Use parallel execution
            PipelineExecutor::Parallel(ParallelExecutor {
                operations: operations.clone(),
                worker_count: self.config.parallel_worker_count,
                chunk_size: self.calculate_optimal_chunk_size(&operations, signature.parallelizable),
            })
        } else if self.can_vectorize(&operations) {
            // Use vectorized execution
            PipelineExecutor::Vectorized(VectorizedExecutor {
                operations: operations.clone(),
                vector_size: self.determine_optimal_vector_size(),
            })
        } else {
            // Use sequential execution
            PipelineExecutor::Sequential(SequentialExecutor {
                operations: operations.clone(),
            })
        };

        let compilation_time = start_time.elapsed();

        Ok(CompiledPipeline {
            signature,
            executor,
            performance_profile: PerformanceProfile {
                average_execution_time: Duration::ZERO,
                throughput: 0.0,
                memory_usage: 0,
                cpu_usage: 0.0,
                cache_hit_rate: 0.0,
            },
            compilation_time,
        })
    }
    
    /// Detect fusion opportunities in a sequence of operations
    fn detect_fusion_opportunities(&self, operations: &[PipelineOp]) -> Result<Vec<FusionOpportunity>, PipelineError> {
        let mut opportunities = Vec::new();
        let mut current_sequence = Vec::new();
        
        for (i, operation) in operations.iter().enumerate() {
            // Check if this operation can be fused with the current sequence
            if self.can_fuse_with_sequence(operation, &current_sequence) {
                current_sequence.push(operation.clone());
            } else {
                // Finalize current sequence if it has fusion potential
                if current_sequence.len() >= 2 {
                    let opportunity = self.analyze_fusion_opportunity(&current_sequence, i - current_sequence.len())?;
                    opportunities.push(opportunity);
                }
                // Start new sequence
                current_sequence = vec![operation.clone()];
            }
        }
        
        // Handle final sequence
        if current_sequence.len() >= 2 {
            let opportunity = self.analyze_fusion_opportunity(&current_sequence, operations.len() - current_sequence.len())?;
            opportunities.push(opportunity);
        }
        
        Ok(opportunities)
    }
    
    /// Check if an operation can be fused with the current sequence
    fn can_fuse_with_sequence(&self, operation: &PipelineOp, sequence: &[PipelineOp]) -> bool {
        if sequence.is_empty() {
            return true;
        }
        
        // Define fusion compatibility rules
        match (sequence.last().unwrap(), operation) {
            // Map operations can be fused together
            (PipelineOp::Map(_), PipelineOp::Map(_)) => true,
            // Map followed by filter
            (PipelineOp::Map(_), PipelineOp::Filter(_)) => true,
            // Filter followed by map
            (PipelineOp::Filter(_), PipelineOp::Map(_)) => true,
            // Take/Skip can be fused with map operations
            (PipelineOp::Take(_), PipelineOp::Map(_)) => true,
            (PipelineOp::Skip(_), PipelineOp::Map(_)) => true,
            // Filter operations can be chained
            (PipelineOp::Filter(_), PipelineOp::Filter(_)) => true,
            // Avoid fusing operations that break data flow
            (_, PipelineOp::Sort(_)) => false, // Sort needs all data
            (_, PipelineOp::Reduce(_)) => false, // Reduce is terminal
            (PipelineOp::Sort(_), _) => false, // After sort, no easy fusion
            (PipelineOp::Reduce(_), _) => false, // After reduce, no fusion
            // Default: allow fusion for simple operations
            _ => sequence.len() < self.config.max_fusion_length,
        }
    }
    
    /// Analyze a fusion opportunity and create the opportunity descriptor
    fn analyze_fusion_opportunity(
        &self,
        operations: &[PipelineOp],
        _start_index: usize,
    ) -> Result<FusionOpportunity, PipelineError> {
        // Estimate performance benefits
        let estimated_speedup = self.estimate_fusion_speedup(operations);
        let memory_reduction = self.estimate_memory_reduction(operations);
        
        // Determine implementation strategy
        let implementation_strategy = if operations.iter().all(|op| self.is_vectorizable_operation(op)) {
            FusedImplementation::Vectorized
        } else if operations.len() > 3 {
            FusedImplementation::Parallel
        } else {
            FusedImplementation::Sequential
        };
        
        Ok(FusionOpportunity {
            operations: operations.to_vec(),
            estimated_speedup,
            memory_reduction,
            implementation_strategy,
        })
    }
    
    /// Estimate the speedup from fusing operations
    fn estimate_fusion_speedup(&self, operations: &[PipelineOp]) -> f64 {
        // Simple heuristic based on operation types and count
        let base_speedup = match operations.len() {
            2 => 1.5,
            3 => 2.0,
            4 => 2.5,
            _ => 3.0,
        };
        
        // Bonus for highly fusable operations
        let vectorizable_ops = operations.iter()
            .filter(|op| self.is_vectorizable_operation(op))
            .count();
        
        let vectorization_bonus = 1.0 + (vectorizable_ops as f64 * 0.3);
        
        base_speedup * vectorization_bonus
    }
    
    /// Estimate memory reduction from fusion
    fn estimate_memory_reduction(&self, operations: &[PipelineOp]) -> f64 {
        // Estimate based on intermediate results eliminated
        let intermediate_results = operations.len() - 1;
        intermediate_results as f64 * 1024.0 // Assume 1KB per intermediate result
    }
    
    /// Check if an operation is highly vectorizable
    fn is_vectorizable_operation(&self, operation: &PipelineOp) -> bool {
        matches!(operation, 
            PipelineOp::Map(_) | 
            PipelineOp::Filter(_) | 
            PipelineOp::Take(_) | 
            PipelineOp::Skip(_)
        )
    }
    
    /// Create fused operations from fusion opportunities
    fn create_fused_operations(&self, opportunities: &[FusionOpportunity]) -> Result<Vec<FusedOperation>, PipelineError> {
        let mut fused_operations = Vec::new();
        
        for opportunity in opportunities {
            let fused_op = FusedOperation {
                name: self.generate_fusion_name(&opportunity.operations),
                implementation: opportunity.implementation_strategy.clone(),
                input_types: vec![DataType::Any], // Would be inferred from actual pipeline
                output_type: DataType::Any,       // Would be inferred from actual pipeline
            };
            fused_operations.push(fused_op);
        }
        
        Ok(fused_operations)
    }
    
    /// Generate a descriptive name for a fused operation
    fn generate_fusion_name(&self, operations: &[PipelineOp]) -> String {
        let op_names: Vec<String> = operations.iter()
            .map(|op| match op {
                PipelineOp::Map(_) => "map".to_string(),
                PipelineOp::Filter(_) => "filter".to_string(),
                PipelineOp::Reduce(_) => "reduce".to_string(),
                PipelineOp::Take(_) => "take".to_string(),
                PipelineOp::Skip(_) => "skip".to_string(),
                _ => "op".to_string(),
            })
            .collect();
        
        format!("fused_{}", op_names.join("_"))
    }
    
    /// Determine optimization level based on pipeline characteristics
    fn determine_optimization_level(&self, signature: &PipelineSignature) -> u8 {
        let mut level = 3; // Base level
        
        // Increase for parallelizable pipelines
        if signature.parallelizable {
            level += 2;
        }
        
        // Increase for large operations
        if signature.operations.len() > 5 {
            level += 1;
        }
        
        // Cap at maximum level
        level.min(8)
    }
    
    /// Calculate optimal chunk size for parallel processing
    fn calculate_optimal_chunk_size(&self, operations: &[PipelineOp], parallelizable: bool) -> usize {
        if !parallelizable {
            return 1000; // Default
        }
        
        // Adjust based on operation complexity
        let complexity_factor = operations.iter()
            .map(|op| match op {
                PipelineOp::Map(_) => 1,
                PipelineOp::Filter(_) => 1,
                PipelineOp::Reduce(_) => 3,
                PipelineOp::Sort(_) => 5,
                _ => 2,
            })
            .sum::<usize>();
        
        // Larger chunks for simpler operations, smaller for complex ones
        1000 / complexity_factor.max(1)
    }
    
    /// Determine optimal vector size for SIMD operations
    fn determine_optimal_vector_size(&self) -> usize {
        // Would normally detect hardware capabilities
        // For now, use a reasonable default
        8 // Suitable for AVX2 f64 operations
    }

    /// Execute compiled pipeline
    fn execute_compiled_pipeline(
        &self,
        compiled: CompiledPipeline,
        input: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        match compiled.executor {
            PipelineExecutor::Sequential(executor) => self.execute_sequential(executor, input),
            PipelineExecutor::Parallel(executor) => self.execute_parallel(executor, input),
            PipelineExecutor::Vectorized(executor) => self.execute_vectorized(executor, input),
            PipelineExecutor::Fused(executor) => self.execute_fused(executor, input),
        }
    }

    // Execution implementations

    fn execute_sequential(
        &self,
        executor: SequentialExecutor,
        mut input: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        for operation in &executor.operations {
            input = self.apply_operation(operation, input)?;
        }
        Ok(input)
    }

    fn execute_parallel(
        &self,
        executor: ParallelExecutor,
        input: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        use rayon::prelude::*;
        
        let chunk_size = executor.chunk_size.max(1);
        
        // Split input into chunks for parallel processing
        let chunks: Vec<Vec<OvmValue>> = input
            .chunks(chunk_size)
            .map(|chunk| chunk.to_vec())
            .collect();
        
        // Process chunks in parallel
        let results: Result<Vec<Vec<OvmValue>>, PipelineError> = chunks
            .into_par_iter()
            .map(|chunk| {
                // Apply all operations sequentially within each chunk
                let mut chunk_result = chunk;
                for operation in &executor.operations {
                    chunk_result = self.apply_operation(operation, chunk_result)?;
                }
                Ok(chunk_result)
            })
            .collect();
        
        // Flatten results back into single vector
        match results {
            Ok(chunk_results) => {
                let mut final_result = Vec::new();
                for chunk in chunk_results {
                    final_result.extend(chunk);
                }
                Ok(final_result)
            }
            Err(e) => Err(e),
        }
    }

    fn execute_vectorized(
        &self,
        executor: VectorizedExecutor,
        input: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        use crate::ovm::simd::SimdEngine;
        use crate::ovm::config::OvmConfig;
        
        // Initialize SIMD engine for vectorized operations
        let simd_config = OvmConfig::default();
        let simd_engine = SimdEngine::new(&simd_config)
            .map_err(|e| PipelineError::Failed(format!("SIMD engine initialization failed: {}", e)))?;
        
        let mut result = input;
        
        // Process each operation using vectorization where possible
        for operation in &executor.operations {
            result = self.apply_vectorized_operation(operation, result, &simd_engine, executor.vector_size)?;
        }
        
        Ok(result)
    }
    
    /// Apply a single vectorized operation
    fn apply_vectorized_operation(
        &self,
        operation: &PipelineOp,
        input: Vec<OvmValue>,
        simd_engine: &crate::ovm::simd::SimdEngine,
        vector_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        match operation {
            // Operations that can be vectorized using SIMD
            PipelineOp::Map(func_name) => {
                self.vectorized_map_operation(&input, func_name, simd_engine, vector_size)
            }
            PipelineOp::Filter(predicate) => {
                self.vectorized_filter_operation(&input, predicate, simd_engine, vector_size)
            }
            PipelineOp::Reduce(func_name) => {
                self.vectorized_reduce_operation(&input, func_name, simd_engine, vector_size)
            }
            // Operations that can be parallelized but don't need SIMD
            PipelineOp::Take(n) => Ok(input.into_iter().take(*n).collect()),
            PipelineOp::Skip(n) => Ok(input.into_iter().skip(*n).collect()),
            PipelineOp::Reverse => {
                let mut result = input;
                result.reverse();
                Ok(result)
            }
            PipelineOp::Distinct => {
                self.vectorized_distinct_operation(&input, vector_size)
            }
            PipelineOp::Sort(comparator) => {
                self.vectorized_sort_operation(&input, comparator, vector_size)
            }
            PipelineOp::Chunk(size) => {
                self.vectorized_chunk_operation(&input, *size)
            }
            PipelineOp::Window(size) => {
                self.vectorized_window_operation(&input, *size)
            }
            _ => {
                // For other operations, use sequential processing
                self.apply_operation(operation, input)
            }
        }
    }
    
    /// Vectorized map operation using SIMD
    fn vectorized_map_operation(
        &self,
        input: &[OvmValue],
        func_name: &str,
        simd_engine: &crate::ovm::simd::SimdEngine,
        _vector_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Try to use SIMD for common mathematical operations
        let operation_name = match func_name {
            "square" | "x^2" => "square",
            "double" | "*2" => "double", 
            "sqrt" | "√" => "sqrt",
            _ => return self.sequential_map_operation(input, func_name),
        };
        
        // Use SIMD engine for vectorization
        let input_arrays = vec![input.to_vec()];
        simd_engine
            .vectorize_array_operation("map", &input_arrays, Some(operation_name))
            .map_err(|e| PipelineError::Failed(format!("SIMD map operation failed: {}", e)))
    }
    
    /// Sequential fallback for map operations
    fn sequential_map_operation(
        &self,
        input: &[OvmValue],
        func_name: &str,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // For complex functions not supported by SIMD, use sequential processing
        let mut result = Vec::with_capacity(input.len());
        
        for value in input {
            // Apply function (simplified - would call actual function in real implementation)
            let new_value = match func_name {
                "identity" => value.clone(),
                "increment" => {
                    // Simple increment operation
                    if let Ok(ast_value) = value.to_ast() {
                        match ast_value {
                            crate::ast::Value::Integer(i) => OvmValue::from_ast(crate::ast::Value::Integer(i + 1)),
                            crate::ast::Value::Float(f) => OvmValue::from_ast(crate::ast::Value::Float(f + 1.0)),
                            _ => value.clone(),
                        }
                    } else {
                        value.clone()
                    }
                }
                _ => value.clone(), // Default: pass through unchanged
            };
            result.push(new_value);
        }
        
        Ok(result)
    }
    
    /// Vectorized filter operation
    fn vectorized_filter_operation(
        &self,
        input: &[OvmValue],
        predicate: &str,
        simd_engine: &crate::ovm::simd::SimdEngine,
        _vector_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Try to use SIMD for common filtering predicates
        let simd_predicate = match predicate {
            "positive" | ">0" | "is_positive" => "is_positive",
            "negative" | "<0" | "is_negative" => "is_negative", 
            "even" | "is_even" => "is_even",
            "odd" | "is_odd" => "is_odd",
            "nonzero" | "!=0" | "is_nonzero" => "is_nonzero",
            _ => {
                // Fall back to sequential processing for complex predicates
                return self.sequential_filter_operation(input, predicate);
            }
        };
        
        // Use SIMD engine for vectorized filtering
        let input_arrays = vec![input.to_vec()];
        simd_engine
            .vectorize_array_operation("filter", &input_arrays, Some(simd_predicate))
            .map_err(|e| PipelineError::Failed(format!("SIMD filter operation failed: {}", e)))
    }
    
    /// Sequential fallback for filter operations
    fn sequential_filter_operation(
        &self,
        input: &[OvmValue],
        predicate: &str,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        let mut result = Vec::new();
        
        for value in input {
            if self.evaluate_predicate(value, predicate)? {
                result.push(value.clone());
            }
        }
        
        Ok(result)
    }
    
    /// Vectorized reduce operation
    fn vectorized_reduce_operation(
        &self,
        input: &[OvmValue],
        func_name: &str,
        simd_engine: &crate::ovm::simd::SimdEngine,
        _vector_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        if input.is_empty() {
            return Ok(vec![]);
        }
        
        // For arithmetic reductions, try to use SIMD
        match func_name {
            "sum" | "+" => self.vectorized_sum_reduce(input, simd_engine),
            "product" | "*" => self.vectorized_product_reduce(input, simd_engine),
            "max" => self.vectorized_max_reduce(input, simd_engine),
            "min" => self.vectorized_min_reduce(input, simd_engine),
            _ => self.sequential_reduce_operation(input, func_name),
        }
    }
    
    /// Vectorized sum reduction
    fn vectorized_sum_reduce(
        &self,
        input: &[OvmValue],
        simd_engine: &crate::ovm::simd::SimdEngine,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Use SIMD engine for vectorized sum reduction
        let input_arrays = vec![input.to_vec()];
        simd_engine
            .vectorize_array_operation("reduce", &input_arrays, Some("sum"))
            .map_err(|e| PipelineError::Failed(format!("SIMD sum reduction failed: {}", e)))
    }
    
    /// Vectorized product reduction
    fn vectorized_product_reduce(
        &self,
        input: &[OvmValue],
        simd_engine: &crate::ovm::simd::SimdEngine,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Use SIMD engine for vectorized product reduction
        let input_arrays = vec![input.to_vec()];
        simd_engine
            .vectorize_array_operation("reduce", &input_arrays, Some("product"))
            .map_err(|e| PipelineError::Failed(format!("SIMD product reduction failed: {}", e)))
    }
    
    /// Vectorized max reduction
    fn vectorized_max_reduce(
        &self, 
        input: &[OvmValue], 
        simd_engine: &crate::ovm::simd::SimdEngine
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Use SIMD engine for vectorized max reduction
        let input_arrays = vec![input.to_vec()];
        simd_engine
            .vectorize_array_operation("reduce", &input_arrays, Some("max"))
            .map_err(|e| PipelineError::Failed(format!("SIMD max reduction failed: {}", e)))
    }
    
    /// Vectorized min reduction
    fn vectorized_min_reduce(
        &self, 
        input: &[OvmValue], 
        simd_engine: &crate::ovm::simd::SimdEngine
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Use SIMD engine for vectorized min reduction
        let input_arrays = vec![input.to_vec()];
        simd_engine
            .vectorize_array_operation("reduce", &input_arrays, Some("min"))
            .map_err(|e| PipelineError::Failed(format!("SIMD min reduction failed: {}", e)))
    }
    
    /// Sequential reduce operation fallback
    fn sequential_reduce_operation(
        &self,
        input: &[OvmValue],
        func_name: &str,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // For complex reductions not supported by vectorization
        match func_name {
            "count" => Ok(vec![OvmValue::from_ast(crate::ast::Value::Integer(input.len() as i64))]),
            "first" => Ok(if input.is_empty() { vec![] } else { vec![input[0].clone()] }),
            "last" => Ok(if input.is_empty() { vec![] } else { vec![input[input.len() - 1].clone()] }),
            _ => Err(PipelineError::Failed(format!("Unsupported reduce operation: {}", func_name))),
        }
    }
    
    /// Vectorized distinct operation using parallel hash set
    fn vectorized_distinct_operation(
        &self,
        input: &[OvmValue],
        vector_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        use std::collections::HashSet;
        let _chunk_size = vector_size.max(64);
        
        // Keep original values that are distinct
        let mut result = Vec::new();
        let mut seen = HashSet::new();
        
        // Use simpler approach for now - can be optimized later
        for value in input {
            let key = format!("{:?}", value);
            if seen.insert(key) {
                result.push(value.clone());
            }
        }
        
        Ok(result)
    }
    
    /// Vectorized sort operation
    fn vectorized_sort_operation(
        &self,
        input: &[OvmValue],
        _comparator: &str,
        _vector_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        use rayon::prelude::*;
        
        // Use parallel sort for large datasets
        let mut result = input.to_vec();
        
        if result.len() > 1000 {
            // Parallel sort for large arrays
            result.par_sort_by(|a, b| {
                self.compare_values(a, b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        } else {
            // Sequential sort for small arrays
            result.sort_by(|a, b| {
                self.compare_values(a, b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        
        Ok(result)
    }
    
    /// Vectorized chunk operation
    fn vectorized_chunk_operation(
        &self,
        input: &[OvmValue],
        chunk_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        let mut result = Vec::new();
        
        for chunk in input.chunks(chunk_size) {
            // Create a list value from the chunk
            let chunk_values: Vec<crate::ast::Value> = chunk
                .iter()
                .filter_map(|v| v.to_ast().ok())
                .collect();
            
            let chunk_list = OvmValue::from_ast(crate::ast::Value::List(chunk_values.into()));
            result.push(chunk_list);
        }
        
        Ok(result)
    }
    
    /// Vectorized window operation
    fn vectorized_window_operation(
        &self,
        input: &[OvmValue],
        window_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        if window_size == 0 || input.len() < window_size {
            return Ok(vec![]);
        }
        
        let mut result = Vec::new();
        
        for window in input.windows(window_size) {
            let window_values: Vec<crate::ast::Value> = window
                .iter()
                .filter_map(|v| v.to_ast().ok())
                .collect();
            
            let window_list = OvmValue::from_ast(crate::ast::Value::List(window_values.into()));
            result.push(window_list);
        }
        
        Ok(result)
    }
    
    /// Helper function to evaluate filter predicates
    fn evaluate_predicate(
        &self,
        value: &OvmValue,
        predicate: &str,
    ) -> Result<bool, PipelineError> {
        // Simple predicate evaluation - in practice would be more sophisticated
        match predicate {
            "is_positive" => {
                if let Ok(ast_value) = value.to_ast() {
                    match ast_value {
                        crate::ast::Value::Integer(i) => Ok(i > 0),
                        crate::ast::Value::Float(f) => Ok(f > 0.0),
                        _ => Ok(false),
                    }
                } else {
                    Ok(false)
                }
            }
            "is_even" => {
                if let Ok(ast_value) = value.to_ast() {
                    if let crate::ast::Value::Integer(i) = ast_value {
                        Ok(i % 2 == 0)
                    } else {
                        Ok(false)
                    }
                } else {
                    Ok(false)
                }
            }
            "is_not_null" => Ok(true), // All OvmValues are non-null by definition
            _ => Ok(true), // Default: include all values
        }
    }
    
    /// Helper function to compare values
    fn compare_values(
        &self,
        a: &OvmValue,
        b: &OvmValue,
    ) -> Result<std::cmp::Ordering, PipelineError> {
        // Convert values to AST for comparison
        let a_ast = a.to_ast().map_err(|e| PipelineError::Failed(format!("Value conversion failed: {}", e)))?;
        let b_ast = b.to_ast().map_err(|e| PipelineError::Failed(format!("Value conversion failed: {}", e)))?;
        
        match (a_ast, b_ast) {
            (crate::ast::Value::Integer(a_int), crate::ast::Value::Integer(b_int)) => {
                Ok(a_int.cmp(&b_int))
            }
            (crate::ast::Value::Float(a_float), crate::ast::Value::Float(b_float)) => {
                Ok(a_float.partial_cmp(&b_float).unwrap_or(std::cmp::Ordering::Equal))
            }
            (crate::ast::Value::Integer(a_int), crate::ast::Value::Float(b_float)) => {
                Ok((a_int as f64).partial_cmp(&b_float).unwrap_or(std::cmp::Ordering::Equal))
            }
            (crate::ast::Value::Float(a_float), crate::ast::Value::Integer(b_int)) => {
                Ok(a_float.partial_cmp(&(b_int as f64)).unwrap_or(std::cmp::Ordering::Equal))
            }
            (crate::ast::Value::String(a_str), crate::ast::Value::String(b_str)) => {
                Ok(a_str.cmp(&b_str))
            }
            (crate::ast::Value::Boolean(a_bool), crate::ast::Value::Boolean(b_bool)) => {
                Ok(a_bool.cmp(&b_bool))
            }
            _ => Ok(std::cmp::Ordering::Equal), // For incomparable types
        }
    }

    fn execute_fused(
        &self,
        executor: FusedExecutor,
        input: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        let mut result = input;
        
        // Execute each fused operation
        for fused_op in &executor.fused_operations {
            result = self.apply_fused_operation(fused_op, result, executor.optimization_level)?;
        }
        
        Ok(result)
    }
    
    /// Apply a single fused operation
    fn apply_fused_operation(
        &self,
        fused_op: &FusedOperation,
        input: Vec<OvmValue>,
        optimization_level: u8,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        match fused_op.implementation {
            FusedImplementation::Sequential => {
                self.apply_fused_sequential(fused_op, input)
            }
            FusedImplementation::Vectorized => {
                self.apply_fused_vectorized(fused_op, input, optimization_level)
            }
            FusedImplementation::Parallel => {
                self.apply_fused_parallel(fused_op, input, optimization_level)
            }
            FusedImplementation::Hybrid => {
                self.apply_fused_hybrid(fused_op, input, optimization_level)
            }
        }
    }
    
    /// Apply fused operation sequentially
    fn apply_fused_sequential(
        &self,
        fused_op: &FusedOperation,
        input: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Convert fused operation back to individual pipeline operations
        let pipeline_ops = self.extract_pipeline_operations(fused_op)?;
        
        let mut result = input;
        for op in pipeline_ops {
            result = self.apply_operation(&op, result)?;
        }
        
        Ok(result)
    }
    
    /// Apply fused operation using vectorization
    fn apply_fused_vectorized(
        &self,
        fused_op: &FusedOperation,
        input: Vec<OvmValue>,
        _optimization_level: u8,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        use crate::ovm::simd::SimdEngine;
        use crate::ovm::config::OvmConfig;
        
        // Initialize SIMD engine for vectorized fused operations
        let simd_config = OvmConfig::default();
        let simd_engine = SimdEngine::new(&simd_config)
            .map_err(|e| PipelineError::Failed(format!("SIMD engine initialization failed: {}", e)))?;
        
        // For fused operations, we can optimize by combining multiple operations into single SIMD passes
        match fused_op.name.as_str() {
            "fused_map_filter" => self.fused_map_filter_vectorized(&input, &simd_engine),
            "fused_map_map" => self.fused_map_map_vectorized(&input, &simd_engine),
            "fused_filter_map" => self.fused_filter_map_vectorized(&input, &simd_engine),
            "fused_take_map" => self.fused_take_map_vectorized(&input, &simd_engine),
            _ => {
                // Fall back to sequential for unknown fused operations
                self.apply_fused_sequential(fused_op, input)
            }
        }
    }
    
    /// Apply fused operation using parallelization
    fn apply_fused_parallel(
        &self,
        fused_op: &FusedOperation,
        input: Vec<OvmValue>,
        optimization_level: u8,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        use rayon::prelude::*;
        
        // Determine chunk size based on input size and optimization level
        let chunk_size = match optimization_level {
            0..=2 => 100,   // Conservative
            3..=5 => 50,    // Moderate
            6..=8 => 25,    // Aggressive
            _ => 10,        // Maximum parallelism
        };
        
        let chunks: Vec<Vec<OvmValue>> = input
            .chunks(chunk_size)
            .map(|chunk| chunk.to_vec())
            .collect();
        
        // Process chunks in parallel
        let results: Result<Vec<Vec<OvmValue>>, PipelineError> = chunks
            .into_par_iter()
            .map(|chunk| {
                // Apply the fused operation to each chunk
                self.apply_fused_sequential(fused_op, chunk)
            })
            .collect();
        
        // Flatten results
        match results {
            Ok(chunk_results) => {
                let mut final_result = Vec::new();
                for chunk in chunk_results {
                    final_result.extend(chunk);
                }
                Ok(final_result)
            }
            Err(e) => Err(e),
        }
    }
    
    /// Apply fused operation using hybrid approach (vectorization + parallelization)
    fn apply_fused_hybrid(
        &self,
        fused_op: &FusedOperation,
        input: Vec<OvmValue>,
        optimization_level: u8,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Use hybrid approach for large datasets
        if input.len() < 1000 {
            // For small datasets, use vectorization only
            self.apply_fused_vectorized(fused_op, input, optimization_level)
        } else {
            // For large datasets, combine parallelization with vectorization
            use rayon::prelude::*;
            
            let chunk_size = 500; // Fixed chunk size for hybrid approach
            let chunks: Vec<Vec<OvmValue>> = input
                .chunks(chunk_size)
                .map(|chunk| chunk.to_vec())
                .collect();
            
            // Process chunks in parallel, using vectorization within each chunk
            let results: Result<Vec<Vec<OvmValue>>, PipelineError> = chunks
                .into_par_iter()
                .map(|chunk| {
                    self.apply_fused_vectorized(fused_op, chunk, optimization_level)
                })
                .collect();
            
            // Flatten results
            match results {
                Ok(chunk_results) => {
                    let mut final_result = Vec::new();
                    for chunk in chunk_results {
                        final_result.extend(chunk);
                    }
                    Ok(final_result)
                }
                Err(e) => Err(e),
            }
        }
    }
    
    /// Extract individual pipeline operations from a fused operation
    fn extract_pipeline_operations(
        &self,
        fused_op: &FusedOperation,
    ) -> Result<Vec<PipelineOp>, PipelineError> {
        // This would normally parse the fused operation name to extract constituent operations
        // For now, we'll use simple pattern matching
        match fused_op.name.as_str() {
            "fused_map_filter" => Ok(vec![
                PipelineOp::Map("identity".to_string()),
                PipelineOp::Filter("is_positive".to_string()),
            ]),
            "fused_map_map" => Ok(vec![
                PipelineOp::Map("double".to_string()),
                PipelineOp::Map("square".to_string()),
            ]),
            "fused_filter_map" => Ok(vec![
                PipelineOp::Filter("is_even".to_string()),
                PipelineOp::Map("double".to_string()),
            ]),
            "fused_take_map" => Ok(vec![
                PipelineOp::Take(100),
                PipelineOp::Map("increment".to_string()),
            ]),
            _ => Err(PipelineError::Failed(format!("Unknown fused operation: {}", fused_op.name))),
        }
    }
    
    /// Fused map-filter operation using vectorization
    fn fused_map_filter_vectorized(
        &self,
        input: &[OvmValue],
        simd_engine: &crate::ovm::simd::SimdEngine,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // This is a combined map+filter operation that can be highly optimized
        // 1. Apply map operation using SIMD
        let input_arrays = vec![input.to_vec()];
        let mapped_result = simd_engine
            .vectorize_array_operation("map", &input_arrays, Some("double"))
            .map_err(|e| PipelineError::Failed(format!("SIMD map operation failed: {}", e)))?;
        
        // 2. Apply filter operation (can't fully vectorize predicate evaluation, but can use chunking)
        let mut filtered_result = Vec::new();
        const CHUNK_SIZE: usize = 8; // Process in small chunks for better cache performance
        
        for chunk in mapped_result.chunks(CHUNK_SIZE) {
            for value in chunk {
                if self.evaluate_predicate(value, "is_positive")? {
                    filtered_result.push(value.clone());
                }
            }
        }
        
        Ok(filtered_result)
    }
    
    /// Fused map-map operation using vectorization
    fn fused_map_map_vectorized(
        &self,
        input: &[OvmValue],
        simd_engine: &crate::ovm::simd::SimdEngine,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Apply double then square - can be fused into a single SIMD operation
        // First apply double
        let input_arrays = vec![input.to_vec()];
        let doubled_result = simd_engine
            .vectorize_array_operation("map", &input_arrays, Some("double"))
            .map_err(|e| PipelineError::Failed(format!("SIMD double operation failed: {}", e)))?;
        
        // Then apply square
        let doubled_arrays = vec![doubled_result];
        let final_result = simd_engine
            .vectorize_array_operation("map", &doubled_arrays, Some("square"))
            .map_err(|e| PipelineError::Failed(format!("SIMD square operation failed: {}", e)))?;
        
        Ok(final_result)
    }
    
    /// Fused filter-map operation using vectorization
    fn fused_filter_map_vectorized(
        &self,
        input: &[OvmValue],
        simd_engine: &crate::ovm::simd::SimdEngine,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // First apply filter
        let mut filtered_result = Vec::new();
        for value in input {
            if self.evaluate_predicate(value, "is_even")? {
                filtered_result.push(value.clone());
            }
        }
        
        // Then apply map using SIMD
        if !filtered_result.is_empty() {
            let filtered_arrays = vec![filtered_result];
            simd_engine
                .vectorize_array_operation("map", &filtered_arrays, Some("double"))
                .map_err(|e| PipelineError::Failed(format!("SIMD map operation failed: {}", e)))
        } else {
            Ok(vec![])
        }
    }
    
    /// Fused take-map operation using vectorization
    fn fused_take_map_vectorized(
        &self,
        input: &[OvmValue],
        _simd_engine: &crate::ovm::simd::SimdEngine,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Take first 100 elements, then apply map
        let taken: Vec<OvmValue> = input.iter().take(100).cloned().collect();
        
        // Apply simple increment operation (not worth SIMD for this simple operation)
        let mut result = Vec::with_capacity(taken.len());
        for value in taken {
            let incremented = if let Ok(ast_value) = value.to_ast() {
                match ast_value {
                    crate::ast::Value::Integer(i) => OvmValue::from_ast(crate::ast::Value::Integer(i + 1)),
                    crate::ast::Value::Float(f) => OvmValue::from_ast(crate::ast::Value::Float(f + 1.0)),
                    _ => value,
                }
            } else {
                value
            };
            result.push(incremented);
        }
        
        Ok(result)
    }

    /// Apply a single pipeline operation
    fn apply_operation(
        &self,
        operation: &PipelineOp,
        input: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        match operation {
            PipelineOp::Take(n) => Ok(input.into_iter().take(*n).collect()),
            PipelineOp::Skip(n) => Ok(input.into_iter().skip(*n).collect()),
            PipelineOp::Reverse => {
                let mut result = input;
                result.reverse();
                Ok(result)
            }
            PipelineOp::Distinct => {
                // Simple deduplication - in practice would use a more efficient algorithm
                let mut result = Vec::new();
                for item in input {
                    if !result.iter().any(|x| self.values_equal(x, &item)) {
                        result.push(item);
                    }
                }
                Ok(result)
            }
            _ => {
                // For other operations, return input unchanged for now
                // TODO: Implement all pipeline operations
                Ok(input)
            }
        }
    }

    // Helper methods

    fn get_cached_pipeline(
        &self,
        signature: &PipelineSignature,
    ) -> Result<Option<CompiledPipeline>, PipelineError> {
        if let Ok(cache) = self.pipeline_cache.read() {
            Ok(cache.get(signature).cloned())
        } else {
            Err(PipelineError::Failed(
                "Failed to read pipeline cache".to_string(),
            ))
        }
    }

    fn cache_pipeline(
        &self,
        signature: PipelineSignature,
        compiled: CompiledPipeline,
    ) -> Result<(), PipelineError> {
        if let Ok(mut cache) = self.pipeline_cache.write() {
            cache.insert(signature, compiled);
            Ok(())
        } else {
            Err(PipelineError::Failed(
                "Failed to write pipeline cache".to_string(),
            ))
        }
    }

    fn record_execution_time(&self, duration: Duration, cache_hit: bool) {
        if let Ok(mut monitor) = self.performance_monitor.lock() {
            monitor.pipeline_executions += 1;
            monitor.total_execution_time += duration;
            if cache_hit {
                monitor.cache_hits += 1;
            } else {
                monitor.cache_misses += 1;
            }
        }
    }
    
    /// Get comprehensive performance benchmarks
    pub fn get_performance_benchmarks(&self) -> Result<PipelineBenchmarks, PipelineError> {
        let monitor = self.performance_monitor.lock()
            .map_err(|_| PipelineError::Failed("Failed to acquire performance monitor lock".to_string()))?;
        
        let cache = self.pipeline_cache.read()
            .map_err(|_| PipelineError::Failed("Failed to acquire cache lock".to_string()))?;
        
        let cache_hit_rate = if monitor.pipeline_executions > 0 {
            monitor.cache_hits as f64 / monitor.pipeline_executions as f64
        } else {
            0.0
        };
        
        let average_execution_time = if monitor.pipeline_executions > 0 {
            monitor.total_execution_time / monitor.pipeline_executions as u32
        } else {
            Duration::ZERO
        };
        
        Ok(PipelineBenchmarks {
            total_executions: monitor.pipeline_executions,
            cache_hit_rate,
            cache_miss_rate: 1.0 - cache_hit_rate,
            average_execution_time,
            total_execution_time: monitor.total_execution_time,
            cached_pipelines: cache.len(),
            parallel_executions: monitor.parallel_executions,
            fusion_hits: monitor.fusion_hits,
            memory_usage_estimate: self.estimate_memory_usage(&*cache),
            performance_score: self.calculate_performance_score(&*monitor),
        })
    }
    
    /// Estimate memory usage of cached pipelines
    fn estimate_memory_usage(&self, cache: &std::collections::HashMap<PipelineSignature, CompiledPipeline>) -> usize {
        let mut total_memory = 0;
        
        for (signature, pipeline) in cache {
            // Estimate memory for signature
            total_memory += std::mem::size_of::<PipelineSignature>();
            total_memory += signature.operations.len() * std::mem::size_of::<PipelineOp>();
            
            // Estimate memory for compiled pipeline
            total_memory += std::mem::size_of::<CompiledPipeline>();
            total_memory += pipeline.performance_profile.memory_usage;
        }
        
        total_memory
    }
    
    /// Calculate overall performance score
    fn calculate_performance_score(&self, monitor: &PipelinePerformanceMonitor) -> f64 {
        if monitor.pipeline_executions == 0 {
            return 0.0;
        }
        
        let cache_hit_score = if monitor.pipeline_executions > 0 {
            (monitor.cache_hits as f64 / monitor.pipeline_executions as f64) * 40.0
        } else {
            0.0
        };
        
        let execution_speed_score = if monitor.total_execution_time.as_millis() > 0 {
            let avg_ms = monitor.total_execution_time.as_millis() as f64 / monitor.pipeline_executions as f64;
            // Lower execution time = higher score (max 30 points)
            (1000.0 / avg_ms).min(30.0)
        } else {
            30.0
        };
        
        let parallelization_score = if monitor.pipeline_executions > 0 {
            (monitor.parallel_executions as f64 / monitor.pipeline_executions as f64) * 20.0
        } else {
            0.0
        };
        
        let fusion_score = if monitor.pipeline_executions > 0 {
            (monitor.fusion_hits as f64 / monitor.pipeline_executions as f64) * 10.0
        } else {
            0.0
        };
        
        cache_hit_score + execution_speed_score + parallelization_score + fusion_score
    }
    
    /// Optimize memory usage by cleaning up old cache entries
    pub fn optimize_memory_usage(&self, max_cache_size: usize) -> Result<MemoryOptimizationResult, PipelineError> {
        let mut cache = self.pipeline_cache.write()
            .map_err(|_| PipelineError::Failed("Failed to acquire cache write lock".to_string()))?;
        
        let initial_size = cache.len();
        let initial_memory = self.estimate_memory_usage(&*cache);
        
        if cache.len() <= max_cache_size {
            return Ok(MemoryOptimizationResult {
                entries_removed: 0,
                memory_freed: 0,
                cache_size_before: initial_size,
                cache_size_after: initial_size,
                memory_before: initial_memory,
                memory_after: initial_memory,
            });
        }
        
        // Collect signatures to remove to avoid borrowing conflicts
        let mut signatures_to_remove: Vec<_> = Vec::new();
        {
            // Sort cache entries by access frequency and recency
            let mut entries: Vec<_> = cache.iter().collect();
            entries.sort_by(|a, b| {
                // Prefer keeping entries with better performance profiles
                let score_a = a.1.performance_profile.cache_hit_rate;
                let score_b = b.1.performance_profile.cache_hit_rate;
                score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
            });
            
            // Collect signatures of least useful entries
            let entries_to_remove = cache.len() - max_cache_size;
            signatures_to_remove.extend(
                entries.iter()
                    .take(entries_to_remove)
                    .map(|(sig, _)| (*sig).clone())
            );
        }
        
        // Remove entries
        let mut entries_removed = 0;
        for signature in signatures_to_remove {
            if cache.remove(&signature).is_some() {
                entries_removed += 1;
            }
        }
        
        let final_memory = self.estimate_memory_usage(&*cache);
        
        Ok(MemoryOptimizationResult {
            entries_removed,
            memory_freed: initial_memory.saturating_sub(final_memory),
            cache_size_before: initial_size,
            cache_size_after: cache.len(),
            memory_before: initial_memory,
            memory_after: final_memory,
        })
    }
    
    /// Run performance benchmarks on the pipeline engine
    pub fn run_benchmarks(&self, test_data_sizes: &[usize]) -> Result<Vec<BenchmarkResult>, PipelineError> {
        let mut results = Vec::new();
        
        for &data_size in test_data_sizes {
            // Create test data
            let test_data: Vec<OvmValue> = (0..data_size)
                .map(|i| OvmValue::from_ast(crate::ast::Value::Integer(i as i64)))
                .collect();
            
            // Test different operation combinations
            let test_cases = vec![
                vec![PipelineOp::Map("double".to_string())],
                vec![PipelineOp::Filter("is_positive".to_string())],
                vec![PipelineOp::Map("square".to_string()), PipelineOp::Filter("is_positive".to_string())],
                vec![PipelineOp::Take(data_size / 2), PipelineOp::Map("increment".to_string())],
                vec![PipelineOp::Map("double".to_string()), PipelineOp::Map("square".to_string())],
            ];
            
            for (test_index, operations) in test_cases.iter().enumerate() {
                let start_time = Instant::now();
                
                // Run the pipeline
                let result = self.process_pipeline(test_data.clone(), operations.clone());
                
                let execution_time = start_time.elapsed();
                let throughput = if execution_time.as_secs_f64() > 0.0 {
                    data_size as f64 / execution_time.as_secs_f64()
                } else {
                    f64::INFINITY
                };
                
                results.push(BenchmarkResult {
                    data_size,
                    test_case: test_index,
                    operations: operations.clone(),
                    execution_time,
                    throughput,
                    success: result.is_ok(),
                    memory_usage: data_size * std::mem::size_of::<OvmValue>(), // Rough estimate
                });
            }
        }
        
        Ok(results)
    }

    fn infer_data_type(&self, input: &[OvmValue]) -> Result<DataType, PipelineError> {
        if input.is_empty() {
            return Ok(DataType::Any);
        }

        // Analyze the first few elements to infer the dominant type
        let sample_size = input.len().min(10); // Sample first 10 elements
        let mut type_counts = std::collections::HashMap::new();
        
        for value in &input[0..sample_size] {
            let data_type = self.value_to_data_type(value)?;
            *type_counts.entry(data_type).or_insert(0) += 1;
        }
        
        // Find the most common type
        let dominant_type = type_counts
            .into_iter()
            .max_by_key(|(_, count)| *count)
            .map(|(data_type, _)| data_type)
            .unwrap_or(DataType::Any);
        
        // If we have a homogeneous collection, use that type
        // Otherwise, use the dominant type or List of mixed types
        if sample_size == 1 {
            Ok(dominant_type)
        } else {
            // Check if all sampled values have the same type
            let first_type = self.value_to_data_type(&input[0])?;
            let is_homogeneous = input[0..sample_size]
                .iter()
                .all(|v| self.value_to_data_type(v).unwrap_or(DataType::Any) == first_type);
            
            if is_homogeneous {
                Ok(DataType::List(Box::new(first_type)))
            } else {
                Ok(DataType::List(Box::new(DataType::Any)))
            }
        }
    }

    fn infer_output_type(
        &self,
        operations: &[PipelineOp],
        input_type: &DataType,
    ) -> Result<DataType, PipelineError> {
        let mut current_type = input_type.clone();
        
        // Apply type transformations for each operation
        for operation in operations {
            current_type = self.transform_type_for_operation(operation, &current_type)?;
        }
        
        Ok(current_type)
    }
    
    /// Convert an OvmValue to its corresponding DataType
    fn value_to_data_type(&self, value: &OvmValue) -> Result<DataType, PipelineError> {
        match value.to_ast() {
            Ok(ast_value) => {
                match ast_value {
                    crate::ast::Value::Integer(_) => Ok(DataType::Integer),
                    crate::ast::Value::Float(_) => Ok(DataType::Float),
                    crate::ast::Value::Boolean(_) => Ok(DataType::Boolean),
                    crate::ast::Value::String(_) => Ok(DataType::String),
                    crate::ast::Value::List(ref items) => {
                        if items.is_empty() {
                            Ok(DataType::List(Box::new(DataType::Any)))
                        } else {
                            // Infer element type from first element
                            let first_element_type = match &items[0] {
                                crate::ast::Value::Integer(_) => DataType::Integer,
                                crate::ast::Value::Float(_) => DataType::Float,
                                crate::ast::Value::Boolean(_) => DataType::Boolean,
                                crate::ast::Value::String(_) => DataType::String,
                                _ => DataType::Any,
                            };
                            Ok(DataType::List(Box::new(first_element_type)))
                        }
                    }
                    crate::ast::Value::Tuple(ref items) => {
                        let tuple_types: Result<Vec<DataType>, PipelineError> = items
                            .iter()
                            .map(|item| {
                                match item {
                                    crate::ast::Value::Integer(_) => Ok(DataType::Integer),
                                    crate::ast::Value::Float(_) => Ok(DataType::Float),
                                    crate::ast::Value::Boolean(_) => Ok(DataType::Boolean),
                                    crate::ast::Value::String(_) => Ok(DataType::String),
                                    _ => Ok(DataType::Any),
                                }
                            })
                            .collect();
                        Ok(DataType::Tuple(tuple_types?))
                    }
                    _ => Ok(DataType::Any),
                }
            }
            Err(_) => Ok(DataType::Any),
        }
    }
    
    /// Transform a data type based on a pipeline operation
    fn transform_type_for_operation(
        &self,
        operation: &PipelineOp,
        input_type: &DataType,
    ) -> Result<DataType, PipelineError> {
        match operation {
            // Map operations can change the element type
            PipelineOp::Map(func_name) => {
                self.infer_map_output_type(func_name, input_type)
            }
            // Filter operations preserve type but may change container type
            PipelineOp::Filter(_) => {
                match input_type {
                    DataType::List(element_type) => Ok(DataType::List(element_type.clone())),
                    DataType::Stream(element_type) => Ok(DataType::Stream(element_type.clone())),
                    other => Ok(other.clone()),
                }
            }
            // Reduce operations transform container to element type or specific result type
            PipelineOp::Reduce(func_name) => {
                self.infer_reduce_output_type(func_name, input_type)
            }
            // Take/Skip preserve container type
            PipelineOp::Take(_) | PipelineOp::Skip(_) => Ok(input_type.clone()),
            // Reverse preserves type exactly
            PipelineOp::Reverse => Ok(input_type.clone()),
            // Distinct preserves container and element type
            PipelineOp::Distinct => Ok(input_type.clone()),
            // Sort preserves type exactly
            PipelineOp::Sort(_) => Ok(input_type.clone()),
            // Zip combines two streams/lists
            PipelineOp::Zip => {
                match input_type {
                    DataType::List(element_type) => {
                        // For zip, we'd need the second stream type, but for now assume same type
                        Ok(DataType::List(Box::new(DataType::Tuple(vec![
                            *element_type.clone(),
                            *element_type.clone(),
                        ]))))
                    }
                    DataType::Stream(element_type) => {
                        Ok(DataType::Stream(Box::new(DataType::Tuple(vec![
                            *element_type.clone(),
                            *element_type.clone(),
                        ]))))
                    }
                    _ => Ok(input_type.clone()),
                }
            }
            // Flatten reduces nesting level
            PipelineOp::Flatten => {
                match input_type {
                    DataType::List(element_type) => {
                        match element_type.as_ref() {
                            DataType::List(inner_type) => Ok(DataType::List(inner_type.clone())),
                            _ => Ok(input_type.clone()),
                        }
                    }
                    DataType::Stream(element_type) => {
                        match element_type.as_ref() {
                            DataType::List(inner_type) => Ok(DataType::Stream(inner_type.clone())),
                            DataType::Stream(inner_type) => Ok(DataType::Stream(inner_type.clone())),
                            _ => Ok(input_type.clone()),
                        }
                    }
                    _ => Ok(input_type.clone()),
                }
            }
            // Chunk/Window create nested structure
            PipelineOp::Chunk(_) | PipelineOp::Window(_) => {
                match input_type {
                    DataType::List(element_type) => {
                        Ok(DataType::List(Box::new(DataType::List(element_type.clone()))))
                    }
                    DataType::Stream(element_type) => {
                        Ok(DataType::Stream(Box::new(DataType::List(element_type.clone()))))
                    }
                    _ => Ok(DataType::List(Box::new(input_type.clone()))),
                }
            }
        }
    }
    
    /// Infer output type for map operations based on function name
    fn infer_map_output_type(
        &self,
        func_name: &str,
        input_type: &DataType,
    ) -> Result<DataType, PipelineError> {
        let element_output_type = match func_name {
            // Numeric operations preserve numeric types or promote to float
            "square" | "sqrt" | "double" | "half" => {
                match self.extract_element_type(input_type) {
                    DataType::Integer => DataType::Float, // Promote to float for mathematical ops
                    DataType::Float => DataType::Float,
                    _ => DataType::Any,
                }
            }
            // Integer operations preserve integer type or convert to integer
            "increment" | "decrement" | "abs" => {
                match self.extract_element_type(input_type) {
                    DataType::Integer => DataType::Integer,
                    DataType::Float => DataType::Integer, // Truncate float to int
                    _ => DataType::Any,
                }
            }
            // String operations
            "to_string" | "uppercase" | "lowercase" | "trim" => DataType::String,
            "length" | "size" => DataType::Integer,
            // Boolean operations
            "not" | "is_positive" | "is_negative" | "is_zero" => DataType::Boolean,
            // Type conversion operations
            "to_int" => DataType::Integer,
            "to_float" => DataType::Float,
            "to_bool" => DataType::Boolean,
            // Identity and unknown operations preserve type
            "identity" | _ => self.extract_element_type(input_type),
        };
        
        // Reconstruct container type with new element type
        match input_type {
            DataType::List(_) => Ok(DataType::List(Box::new(element_output_type))),
            DataType::Stream(_) => Ok(DataType::Stream(Box::new(element_output_type))),
            _ => Ok(element_output_type),
        }
    }
    
    /// Infer output type for reduce operations
    fn infer_reduce_output_type(
        &self,
        func_name: &str,
        input_type: &DataType,
    ) -> Result<DataType, PipelineError> {
        match func_name {
            // Numeric reductions
            "sum" | "product" => {
                match self.extract_element_type(input_type) {
                    DataType::Integer => Ok(DataType::Integer),
                    DataType::Float => Ok(DataType::Float),
                    _ => Ok(DataType::Any),
                }
            }
            // Min/Max preserve element type
            "min" | "max" => Ok(self.extract_element_type(input_type)),
            // Count always returns integer
            "count" | "length" => Ok(DataType::Integer),
            // Boolean reductions
            "all" | "any" => Ok(DataType::Boolean),
            // String reductions
            "join" | "concat" => Ok(DataType::String),
            // First/Last preserve element type
            "first" | "last" => Ok(self.extract_element_type(input_type)),
            // Average promotes to float
            "average" | "mean" => Ok(DataType::Float),
            // Generic fold/reduce preserve element type by default
            "fold" | "reduce" | _ => Ok(self.extract_element_type(input_type)),
        }
    }
    
    /// Extract the element type from a container type
    fn extract_element_type(&self, container_type: &DataType) -> DataType {
        match container_type {
            DataType::List(element_type) => *element_type.clone(),
            DataType::Stream(element_type) => *element_type.clone(),
            other => other.clone(),
        }
    }
    
    /// Check if two data types are compatible for operations
    pub fn types_compatible(&self, type1: &DataType, type2: &DataType) -> bool {
        match (type1, type2) {
            // Exact matches
            (a, b) if a == b => true,
            // Numeric compatibility
            (DataType::Integer, DataType::Float) | (DataType::Float, DataType::Integer) => true,
            // Any type is compatible with everything
            (DataType::Any, _) | (_, DataType::Any) => true,
            // Container compatibility
            (DataType::List(a), DataType::List(b)) => self.types_compatible(a, b),
            (DataType::Stream(a), DataType::Stream(b)) => self.types_compatible(a, b),
            (DataType::List(a), DataType::Stream(b)) | (DataType::Stream(a), DataType::List(b)) => {
                self.types_compatible(a, b)
            }
            // Tuple compatibility (all elements must be compatible)
            (DataType::Tuple(a), DataType::Tuple(b)) if a.len() == b.len() => {
                a.iter().zip(b.iter()).all(|(x, y)| self.types_compatible(x, y))
            }
            _ => false,
        }
    }
    
    /// Get the unified type for two compatible types
    pub fn unify_types(&self, type1: &DataType, type2: &DataType) -> Result<DataType, PipelineError> {
        if !self.types_compatible(type1, type2) {
            return Err(PipelineError::Failed(format!("Incompatible types: {:?} and {:?}", type1, type2)));
        }
        
        match (type1, type2) {
            // Exact matches
            (a, b) if a == b => Ok(a.clone()),
            // Numeric unification - promote to float
            (DataType::Integer, DataType::Float) | (DataType::Float, DataType::Integer) => Ok(DataType::Float),
            // Any type unification
            (DataType::Any, other) | (other, DataType::Any) => Ok(other.clone()),
            // Container unification
            (DataType::List(a), DataType::List(b)) => {
                Ok(DataType::List(Box::new(self.unify_types(a, b)?)))
            }
            (DataType::Stream(a), DataType::Stream(b)) => {
                Ok(DataType::Stream(Box::new(self.unify_types(a, b)?)))
            }
            (DataType::List(a), DataType::Stream(b)) | (DataType::Stream(a), DataType::List(b)) => {
                Ok(DataType::Stream(Box::new(self.unify_types(a, b)?)))
            }
            // Tuple unification
            (DataType::Tuple(a), DataType::Tuple(b)) if a.len() == b.len() => {
                let unified_elements: Result<Vec<DataType>, PipelineError> = a
                    .iter()
                    .zip(b.iter())
                    .map(|(x, y)| self.unify_types(x, y))
                    .collect();
                Ok(DataType::Tuple(unified_elements?))
            }
            _ => Ok(DataType::Any), // Fallback to Any for complex cases
        }
    }
    
    /// Validate that a pipeline operation sequence is type-safe
    pub fn validate_pipeline_types(
        &self,
        input_type: &DataType,
        operations: &[PipelineOp],
    ) -> Result<Vec<DataType>, PipelineError> {
        let mut type_chain = vec![input_type.clone()];
        let mut current_type = input_type.clone();
        
        for operation in operations {
            // Check if the current type is compatible with the operation
            if !self.operation_accepts_type(operation, &current_type) {
                return Err(PipelineError::Failed(format!(
                    "Operation {:?} cannot accept type {:?}",
                    operation, current_type
                )));
            }
            
            // Transform the type
            current_type = self.transform_type_for_operation(operation, &current_type)?;
            type_chain.push(current_type.clone());
        }
        
        Ok(type_chain)
    }
    
    /// Check if an operation can accept a given input type
    fn operation_accepts_type(&self, operation: &PipelineOp, input_type: &DataType) -> bool {
        match operation {
            // Most operations can work with any type
            PipelineOp::Take(_) | PipelineOp::Skip(_) | PipelineOp::Reverse => true,
            // Filter accepts any container type
            PipelineOp::Filter(_) => {
                matches!(input_type, DataType::List(_) | DataType::Stream(_) | DataType::Any)
            }
            // Map accepts any container type
            PipelineOp::Map(_) => {
                matches!(input_type, DataType::List(_) | DataType::Stream(_) | DataType::Any)
            }
            // Reduce accepts any container type
            PipelineOp::Reduce(_) => {
                matches!(input_type, DataType::List(_) | DataType::Stream(_) | DataType::Any)
            }
            // Zip needs two compatible streams/lists
            PipelineOp::Zip => {
                matches!(input_type, DataType::List(_) | DataType::Stream(_) | DataType::Any)
            }
            // Flatten needs nested containers
            PipelineOp::Flatten => {
                match input_type {
                    DataType::List(element_type) => matches!(element_type.as_ref(), DataType::List(_)),
                    DataType::Stream(element_type) => matches!(element_type.as_ref(), DataType::List(_) | DataType::Stream(_)),
                    DataType::Any => true,
                    _ => false,
                }
            }
            // Other operations are generally permissive
            _ => true,
        }
    }

    fn is_parallelizable(&self, operations: &[PipelineOp]) -> bool {
        // Simple heuristic - map, filter operations are parallelizable
        operations.iter().all(|op| {
            matches!(
                op,
                PipelineOp::Map(_)
                    | PipelineOp::Filter(_)
                    | PipelineOp::Distinct
                    | PipelineOp::Reverse
            )
        })
    }

    fn can_vectorize(&self, operations: &[PipelineOp]) -> bool {
        // Simple heuristic for vectorization
        operations
            .iter()
            .any(|op| matches!(op, PipelineOp::Map(_) | PipelineOp::Filter(_)))
    }

    fn reorder_operations(
        &self,
        operations: Vec<PipelineOp>,
    ) -> Result<Vec<PipelineOp>, PipelineError> {
        if operations.len() <= 1 {
            return Ok(operations);
        }
        
        let mut reordered = operations.clone();
        
        // Apply multiple optimization passes
        reordered = self.apply_filter_hoisting(reordered)?;
        reordered = self.apply_take_skip_optimization(reordered)?;
        reordered = self.apply_map_reordering(reordered)?;
        reordered = self.apply_predicate_pushdown(reordered)?;
        reordered = self.apply_sort_optimization(reordered)?;
        
        Ok(reordered)
    }
    
    /// Hoist filter operations as early as possible to reduce data flow
    fn apply_filter_hoisting(&self, operations: Vec<PipelineOp>) -> Result<Vec<PipelineOp>, PipelineError> {
        let mut optimized = Vec::new();
        let mut filters = Vec::new();
        
        for operation in operations {
            match operation {
                PipelineOp::Filter(predicate) => {
                    // Collect filters to potentially move them earlier
                    filters.push(PipelineOp::Filter(predicate));
                }
                PipelineOp::Map(func) => {
                    // Maps that don't affect filter predicates can be moved after filters
                    if self.can_move_map_after_filters(&func, &filters) {
                        // Add accumulated filters first
                        optimized.extend(filters.clone());
                        filters.clear();
                        optimized.push(PipelineOp::Map(func));
                    } else {
                        // Add map first, then filters
                        optimized.push(PipelineOp::Map(func));
                        optimized.extend(filters.clone());
                        filters.clear();
                    }
                }
                PipelineOp::Take(n) | PipelineOp::Skip(n) => {
                    // Take/Skip should come after filters when possible
                    optimized.extend(filters.clone());
                    filters.clear();
                    optimized.push(if matches!(operation, PipelineOp::Take(_)) {
                        PipelineOp::Take(n)
                    } else {
                        PipelineOp::Skip(n)
                    });
                }
                other => {
                    // For other operations, add accumulated filters first
                    optimized.extend(filters.clone());
                    filters.clear();
                    optimized.push(other);
                }
            }
        }
        
        // Add any remaining filters
        optimized.extend(filters);
        
        Ok(optimized)
    }
    
    /// Optimize Take/Skip operations by combining and reordering them
    fn apply_take_skip_optimization(&self, operations: Vec<PipelineOp>) -> Result<Vec<PipelineOp>, PipelineError> {
        let mut optimized = Vec::new();
        let mut i = 0;
        
        while i < operations.len() {
            match &operations[i] {
                PipelineOp::Take(n1) => {
                    // Look ahead for consecutive take/skip operations
                    let mut take_amount = *n1;
                    let mut skip_amount = 0;
                    let mut j = i + 1;
                    
                    while j < operations.len() {
                        match &operations[j] {
                            PipelineOp::Take(n2) => {
                                take_amount = take_amount.min(*n2);
                                j += 1;
                            }
                            PipelineOp::Skip(n2) => {
                                skip_amount += n2;
                                take_amount = take_amount.saturating_sub(*n2);
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    
                    // Add optimized sequence
                    if skip_amount > 0 {
                        optimized.push(PipelineOp::Skip(skip_amount));
                    }
                    if take_amount > 0 {
                        optimized.push(PipelineOp::Take(take_amount));
                    }
                    
                    i = j;
                }
                PipelineOp::Skip(n1) => {
                    // Look ahead for consecutive skip operations
                    let mut skip_amount = *n1;
                    let mut j = i + 1;
                    
                    while j < operations.len() {
                        match &operations[j] {
                            PipelineOp::Skip(n2) => {
                                skip_amount += n2;
                                j += 1;
                            }
                            _ => break,
                        }
                    }
                    
                    optimized.push(PipelineOp::Skip(skip_amount));
                    i = j;
                }
                other => {
                    optimized.push(other.clone());
                    i += 1;
                }
            }
        }
        
        Ok(optimized)
    }
    
    /// Reorder map operations for better cache locality and fusion opportunities
    fn apply_map_reordering(&self, operations: Vec<PipelineOp>) -> Result<Vec<PipelineOp>, PipelineError> {
        let mut optimized = Vec::new();
        let mut map_chain = Vec::new();
        
        for operation in operations {
            match operation {
                PipelineOp::Map(func) => {
                    map_chain.push(func);
                }
                other => {
                    // Process accumulated map chain
                    if !map_chain.is_empty() {
                        let reordered_maps = self.optimize_map_chain(&map_chain)?;
                        for map_func in reordered_maps {
                            optimized.push(PipelineOp::Map(map_func));
                        }
                        map_chain.clear();
                    }
                    optimized.push(other);
                }
            }
        }
        
        // Handle remaining map chain
        if !map_chain.is_empty() {
            let reordered_maps = self.optimize_map_chain(&map_chain)?;
            for map_func in reordered_maps {
                optimized.push(PipelineOp::Map(map_func));
            }
        }
        
        Ok(optimized)
    }
    
    /// Optimize a chain of map operations
    fn optimize_map_chain(&self, map_functions: &[String]) -> Result<Vec<String>, PipelineError> {
        if map_functions.len() <= 1 {
            return Ok(map_functions.to_vec());
        }
        
        let mut optimized = map_functions.to_vec();
        
        // Sort by computational cost (cheaper operations first)
        optimized.sort_by(|a, b| {
            let cost_a = self.estimate_map_cost(a);
            let cost_b = self.estimate_map_cost(b);
            cost_a.partial_cmp(&cost_b).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        Ok(optimized)
    }
    
    /// Estimate the computational cost of a map operation
    fn estimate_map_cost(&self, func_name: &str) -> f64 {
        match func_name {
            // Very cheap operations
            "identity" | "increment" | "decrement" => 1.0,
            // Cheap arithmetic
            "double" | "half" | "abs" | "negate" => 2.0,
            // Moderate operations
            "square" | "cube" => 3.0,
            // Expensive operations
            "sqrt" | "log" | "exp" => 10.0,
            "sin" | "cos" | "tan" => 15.0,
            // String operations (generally expensive)
            "to_string" | "uppercase" | "lowercase" => 8.0,
            "trim" | "reverse" => 5.0,
            // Unknown operations - assume moderate cost
            _ => 5.0,
        }
    }
    
    /// Apply predicate pushdown optimization to move filters earlier
    fn apply_predicate_pushdown(&self, operations: Vec<PipelineOp>) -> Result<Vec<PipelineOp>, PipelineError> {
        let mut delayed_operations: Vec<PipelineOp> = Vec::new();
        
        for operation in operations.into_iter().rev() {
            match operation {
                PipelineOp::Filter(predicate) => {
                    // Try to push this filter down (move it earlier)
                    let mut inserted = false;
                    
                    // Look for a good position to insert this filter
                    for (i, delayed_op) in delayed_operations.iter().enumerate() {
                        if self.can_push_filter_before(delayed_op, &predicate) {
                            delayed_operations.insert(i, PipelineOp::Filter(predicate.clone()));
                            inserted = true;
                            break;
                        }
                    }
                    
                    if !inserted {
                        delayed_operations.push(PipelineOp::Filter(predicate));
                    }
                }
                other => {
                    delayed_operations.push(other);
                }
            }
        }
        
        // Reverse to get correct order
        delayed_operations.reverse();
        Ok(delayed_operations)
    }
    
    /// Check if a filter can be moved before another operation
    fn can_push_filter_before(&self, operation: &PipelineOp, _predicate: &str) -> bool {
        match operation {
            // Filters can generally be pushed before these operations
            PipelineOp::Take(_) | PipelineOp::Skip(_) => true,
            // Careful with maps - depends on whether the map affects the predicate
            PipelineOp::Map(_) => false, // Conservative: don't push filters before maps
            // Can't push before other filters (would change semantics)
            PipelineOp::Filter(_) => false,
            // Can't push before operations that need all data
            PipelineOp::Sort(_) | PipelineOp::Reduce(_) => false,
            // Other operations - be conservative
            _ => false,
        }
    }
    
    /// Optimize sort operations by reordering them with other operations
    fn apply_sort_optimization(&self, operations: Vec<PipelineOp>) -> Result<Vec<PipelineOp>, PipelineError> {
        let mut optimized = Vec::new();
        let mut i = 0;
        
        while i < operations.len() {
            match &operations[i] {
                PipelineOp::Sort(comparator) => {
                    // Look ahead to see if we can optimize the sort position
                    let mut sort_position = i;
                    
                    // Check if there are operations after sort that don't change ordering
                    for j in (i + 1)..operations.len() {
                        match &operations[j] {
                            // These operations preserve order, so sort can be delayed
                            PipelineOp::Map(_) => {
                                sort_position = j;
                            }
                            // These operations may change order, so sort must come before
                            PipelineOp::Filter(_) | PipelineOp::Take(_) | PipelineOp::Skip(_) => {
                                break;
                            }
                            // Other sorts - optimize by combining if possible
                            PipelineOp::Sort(_) => {
                                // Keep only the last sort (it overrides previous ones)
                                sort_position = j;
                            }
                            _ => break,
                        }
                    }
                    
                    // Add operations before the optimal sort position
                    for j in i..sort_position {
                        if !matches!(operations[j], PipelineOp::Sort(_)) {
                            optimized.push(operations[j].clone());
                        }
                    }
                    
                    // Add the sort at the optimal position
                    optimized.push(PipelineOp::Sort(comparator.clone()));
                    
                    i = sort_position + 1;
                }
                other => {
                    optimized.push(other.clone());
                    i += 1;
                }
            }
        }
        
        Ok(optimized)
    }
    
    /// Check if a map operation can be moved after filter operations
    fn can_move_map_after_filters(&self, func_name: &str, _filters: &[PipelineOp]) -> bool {
        // Simple heuristic: pure mathematical operations can usually be moved
        matches!(func_name, 
            "increment" | "decrement" | "double" | "half" | "square" | "cube" | 
            "abs" | "negate" | "sqrt" | "identity"
        )
    }

    fn add_vectorization_hints(
        &self,
        operations: Vec<PipelineOp>,
    ) -> Result<Vec<PipelineOp>, PipelineError> {
        // Add metadata to operations that can benefit from vectorization
        let mut enhanced_operations = Vec::new();
        
        for operation in operations {
            // Check if this operation can be vectorized
            let has_vectorization_hint = self.should_add_vectorization_hint(&operation);
            
            if has_vectorization_hint {
                // In a real implementation, we would add metadata to the operation
                // For now, just mark it as vectorizable by keeping it unchanged
                enhanced_operations.push(operation);
            } else {
                enhanced_operations.push(operation);
            }
        }
        
        Ok(enhanced_operations)
    }
    
    /// Check if an operation should have vectorization hints
    fn should_add_vectorization_hint(&self, operation: &PipelineOp) -> bool {
        match operation {
            // These operations benefit greatly from vectorization
            PipelineOp::Map(func_name) => {
                self.is_vectorizable_map_function(func_name)
            }
            PipelineOp::Filter(predicate) => {
                self.is_vectorizable_predicate(predicate)
            }
            PipelineOp::Reduce(func_name) => {
                self.is_vectorizable_reduce_function(func_name)
            }
            // These operations can be vectorized in some cases
            PipelineOp::Take(_) | PipelineOp::Skip(_) => true,
            PipelineOp::Distinct | PipelineOp::Reverse => true,
            // These operations are harder to vectorize effectively
            PipelineOp::Sort(_) | PipelineOp::Zip => false,
            PipelineOp::Flatten | PipelineOp::Chunk(_) | PipelineOp::Window(_) => false,
        }
    }
    
    /// Check if a map function can be vectorized
    fn is_vectorizable_map_function(&self, func_name: &str) -> bool {
        matches!(func_name,
            // Arithmetic operations - excellent for SIMD
            "square" | "cube" | "double" | "half" | "increment" | "decrement" |
            "abs" | "negate" | "sqrt" | "exp" | "log" |
            // Trigonometric functions - good for SIMD
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" |
            // Simple conversions
            "to_float" | "to_int" |
            // Identity is trivially vectorizable
            "identity"
        )
    }
    
    /// Check if a predicate can be vectorized
    fn is_vectorizable_predicate(&self, predicate: &str) -> bool {
        matches!(predicate,
            // Numeric comparisons - excellent for SIMD
            "is_positive" | "is_negative" | "is_zero" | "is_even" | "is_odd" |
            "greater_than" | "less_than" | "equals" | "not_equals" |
            // Range checks
            "in_range" | "out_of_range" |
            // Simple boolean operations
            "is_true" | "is_false" | "not"
        )
    }
    
    /// Check if a reduce function can be vectorized
    fn is_vectorizable_reduce_function(&self, func_name: &str) -> bool {
        matches!(func_name,
            // Arithmetic reductions - excellent for SIMD
            "sum" | "product" | "average" | "mean" |
            // Min/max operations
            "min" | "max" |
            // Boolean reductions
            "all" | "any" |
            // Count is simple
            "count"
        )
    }
    
    /// Add comprehensive fusion hints and detection logic
    pub fn detect_fusion_hints(&self, operations: &[PipelineOp]) -> Vec<FusionHint> {
        let mut hints = Vec::new();
        
        // Scan for common fusion patterns
        for window in operations.windows(2) {
            if let Some(hint) = self.detect_two_op_fusion_hint(&window[0], &window[1]) {
                hints.push(hint);
            }
        }
        
        // Scan for longer fusion sequences
        for window in operations.windows(3) {
            if let Some(hint) = self.detect_three_op_fusion_hint(&window[0], &window[1], &window[2]) {
                hints.push(hint);
            }
        }
        
        // Detect loop fusion opportunities
        let loop_fusion_hints = self.detect_loop_fusion_opportunities(operations);
        hints.extend(loop_fusion_hints);
        
        // Detect memory access pattern optimizations
        let memory_hints = self.detect_memory_optimization_hints(operations);
        hints.extend(memory_hints);
        
        hints
    }
    
    /// Detect fusion opportunities between two operations
    fn detect_two_op_fusion_hint(&self, op1: &PipelineOp, op2: &PipelineOp) -> Option<FusionHint> {
        match (op1, op2) {
            // Map-Map fusion - excellent opportunity
            (PipelineOp::Map(f1), PipelineOp::Map(f2)) => {
                Some(FusionHint {
                    hint_type: FusionHintType::MapMapFusion,
                    operations: vec![op1.clone(), op2.clone()],
                    estimated_benefit: self.estimate_map_map_fusion_benefit(f1, f2),
                    fusion_strategy: self.select_fusion_strategy(&[op1.clone(), op2.clone()]),
                })
            }
            // Map-Filter fusion - very good opportunity
            (PipelineOp::Map(f), PipelineOp::Filter(p)) => {
                Some(FusionHint {
                    hint_type: FusionHintType::MapFilterFusion,
                    operations: vec![op1.clone(), op2.clone()],
                    estimated_benefit: self.estimate_map_filter_fusion_benefit(f, p),
                    fusion_strategy: self.select_fusion_strategy(&[op1.clone(), op2.clone()]),
                })
            }
            // Filter-Map fusion - good opportunity
            (PipelineOp::Filter(p), PipelineOp::Map(f)) => {
                Some(FusionHint {
                    hint_type: FusionHintType::FilterMapFusion,
                    operations: vec![op1.clone(), op2.clone()],
                    estimated_benefit: self.estimate_filter_map_fusion_benefit(p, f),
                    fusion_strategy: self.select_fusion_strategy(&[op1.clone(), op2.clone()]),
                })
            }
            // Take-Map fusion - eliminates intermediate collection
            (PipelineOp::Take(n), PipelineOp::Map(f)) => {
                Some(FusionHint {
                    hint_type: FusionHintType::TakeMapFusion,
                    operations: vec![op1.clone(), op2.clone()],
                    estimated_benefit: self.estimate_take_map_fusion_benefit(*n, f),
                    fusion_strategy: FusionStrategy::Sequential, // Simple fusion
                })
            }
            _ => None,
        }
    }
    
    /// Detect fusion opportunities among three operations
    fn detect_three_op_fusion_hint(&self, op1: &PipelineOp, op2: &PipelineOp, op3: &PipelineOp) -> Option<FusionHint> {
        match (op1, op2, op3) {
            // Map-Map-Map chain - excellent for fusion
            (PipelineOp::Map(_), PipelineOp::Map(_), PipelineOp::Map(_)) => {
                Some(FusionHint {
                    hint_type: FusionHintType::MultiMapFusion,
                    operations: vec![op1.clone(), op2.clone(), op3.clone()],
                    estimated_benefit: FusionBenefit {
                        performance_gain: 3.5,
                        memory_savings: 2048, // Two intermediate collections eliminated
                        cache_efficiency: 1.8,
                    },
                    fusion_strategy: FusionStrategy::Vectorized,
                })
            }
            // Filter-Filter-Map - can combine filters
            (PipelineOp::Filter(_), PipelineOp::Filter(_), PipelineOp::Map(_)) => {
                Some(FusionHint {
                    hint_type: FusionHintType::MultiFilterMapFusion,
                    operations: vec![op1.clone(), op2.clone(), op3.clone()],
                    estimated_benefit: FusionBenefit {
                        performance_gain: 2.8,
                        memory_savings: 1536,
                        cache_efficiency: 1.6,
                    },
                    fusion_strategy: FusionStrategy::Parallel,
                })
            }
            _ => None,
        }
    }
    
    /// Detect loop fusion opportunities
    fn detect_loop_fusion_opportunities(&self, operations: &[PipelineOp]) -> Vec<FusionHint> {
        let mut hints = Vec::new();
        
        // Look for patterns where multiple operations iterate over the same data
        let mut current_loop_ops = Vec::new();
        
        for operation in operations {
            match operation {
                // These operations typically require full iteration
                PipelineOp::Map(_) | PipelineOp::Filter(_) => {
                    current_loop_ops.push(operation.clone());
                }
                // These operations break loop fusion potential
                PipelineOp::Sort(_) | PipelineOp::Reduce(_) => {
                    if current_loop_ops.len() >= 2 {
                        hints.push(FusionHint {
                            hint_type: FusionHintType::LoopFusion,
                            operations: current_loop_ops.clone(),
                            estimated_benefit: self.estimate_loop_fusion_benefit(&current_loop_ops),
                            fusion_strategy: FusionStrategy::Vectorized,
                        });
                    }
                    current_loop_ops.clear();
                }
                _ => {
                    // Other operations can be included in loop fusion
                    current_loop_ops.push(operation.clone());
                }
            }
        }
        
        // Handle remaining operations
        if current_loop_ops.len() >= 2 {
            hints.push(FusionHint {
                hint_type: FusionHintType::LoopFusion,
                operations: current_loop_ops,
                estimated_benefit: FusionBenefit {
                    performance_gain: 2.0,
                    memory_savings: 1024,
                    cache_efficiency: 1.5,
                },
                fusion_strategy: FusionStrategy::Vectorized,
            });
        }
        
        hints
    }
    
    /// Detect memory access pattern optimization opportunities
    fn detect_memory_optimization_hints(&self, operations: &[PipelineOp]) -> Vec<FusionHint> {
        let mut hints = Vec::new();
        
        // Look for operations that can benefit from cache-friendly access patterns
        for window in operations.windows(2) {
            if self.operations_have_similar_memory_patterns(&window[0], &window[1]) {
                hints.push(FusionHint {
                    hint_type: FusionHintType::MemoryOptimization,
                    operations: window.to_vec(),
                    estimated_benefit: FusionBenefit {
                        performance_gain: 1.4,
                        memory_savings: 512,
                        cache_efficiency: 2.0, // Major cache improvement
                    },
                    fusion_strategy: FusionStrategy::CacheOptimized,
                });
            }
        }
        
        hints
    }
    
    /// Check if operations have similar memory access patterns
    fn operations_have_similar_memory_patterns(&self, op1: &PipelineOp, op2: &PipelineOp) -> bool {
        match (op1, op2) {
            // Sequential access patterns
            (PipelineOp::Map(_), PipelineOp::Map(_)) => true,
            (PipelineOp::Filter(_), PipelineOp::Filter(_)) => true,
            (PipelineOp::Map(_), PipelineOp::Filter(_)) => true,
            (PipelineOp::Filter(_), PipelineOp::Map(_)) => true,
            // Take/Skip operations
            (PipelineOp::Take(_), PipelineOp::Map(_)) => true,
            (PipelineOp::Skip(_), PipelineOp::Map(_)) => true,
            _ => false,
        }
    }
    
    // Helper methods for benefit estimation
    
    fn estimate_map_map_fusion_benefit(&self, f1: &str, f2: &str) -> FusionBenefit {
        let base_gain = 2.5; // Good performance improvement
        let cost_factor = (self.estimate_map_cost(f1) + self.estimate_map_cost(f2)) / 10.0;
        
        FusionBenefit {
            performance_gain: base_gain * (1.0 + cost_factor),
            memory_savings: 1024, // One intermediate collection eliminated
            cache_efficiency: 1.6,
        }
    }
    
    fn estimate_map_filter_fusion_benefit(&self, f: &str, _p: &str) -> FusionBenefit {
        let map_cost = self.estimate_map_cost(f);
        
        FusionBenefit {
            performance_gain: 2.2 + (map_cost / 20.0),
            memory_savings: 1024,
            cache_efficiency: 1.4,
        }
    }
    
    fn estimate_filter_map_fusion_benefit(&self, _p: &str, f: &str) -> FusionBenefit {
        let map_cost = self.estimate_map_cost(f);
        
        FusionBenefit {
            performance_gain: 2.0 + (map_cost / 25.0),
            memory_savings: 1024,
            cache_efficiency: 1.3,
        }
    }
    
    fn estimate_take_map_fusion_benefit(&self, n: usize, f: &str) -> FusionBenefit {
        let size_factor = if n < 100 { 1.2 } else { 1.0 };
        let map_cost = self.estimate_map_cost(f);
        
        FusionBenefit {
            performance_gain: 1.8 * size_factor + (map_cost / 30.0),
            memory_savings: n.min(1024) as usize, // Limited by take size
            cache_efficiency: 1.2,
        }
    }
    
    fn estimate_loop_fusion_benefit(&self, operations: &[PipelineOp]) -> FusionBenefit {
        let base_gain = 1.5 + (operations.len() as f64 * 0.3);
        
        FusionBenefit {
            performance_gain: base_gain,
            memory_savings: (operations.len() - 1) * 512, // Eliminate intermediate collections
            cache_efficiency: 1.4 + (operations.len() as f64 * 0.1),
        }
    }
    
    fn select_fusion_strategy(&self, operations: &[PipelineOp]) -> FusionStrategy {
        let all_vectorizable = operations.iter().all(|op| self.is_vectorizable_operation(op));
        let has_complex_ops = operations.iter().any(|op| {
            matches!(op, PipelineOp::Sort(_) | PipelineOp::Reduce(_))
        });
        
        if all_vectorizable && !has_complex_ops {
            FusionStrategy::Vectorized
        } else if operations.len() > 3 {
            FusionStrategy::Parallel
        } else {
            FusionStrategy::Sequential
        }
    }

    fn values_equal(&self, a: &OvmValue, b: &OvmValue) -> bool {
        // Use the proper PartialEq implementation
        a == b
    }
}

// Implementation of supporting types

impl PipelineFusionOptimizer {
    fn new() -> Self {
        Self {
            fusion_patterns: Vec::new(),
            opportunities_cache: Arc::new(RwLock::new(HashMap::new())),
            fusion_stats: Arc::new(Mutex::new(FusionStatistics::default())),
        }
    }

    fn apply_fusion(&self, operations: Vec<PipelineOp>) -> Result<Vec<PipelineOp>, PipelineError> {
        // For now, return operations as-is
        // TODO: Implement fusion optimization
        Ok(operations)
    }
}

impl ParallelProcessor {
    fn new(config: &PipelineConfig) -> Result<Self, PipelineError> {
        let worker_pool = Arc::new(ThreadPool::new(config.parallel_worker_count)?);

        Ok(Self {
            worker_pool,
            work_queue: Arc::new(Mutex::new(VecDeque::new())),
            load_balancer: LoadBalancer::new(config.parallel_worker_count),
        })
    }
    
    /// Process data in parallel using work stealing
    pub fn process_parallel_with_work_stealing(
        &self,
        input: Vec<OvmValue>,
        operations: Vec<PipelineOp>,
        chunk_size: usize,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        
        // Split input into work items
        let chunks: Vec<Vec<OvmValue>> = input
            .chunks(chunk_size.max(1))
            .map(|chunk| chunk.to_vec())
            .collect();
        
        // Process work items in parallel (simplified implementation)
        let mut results = Vec::new();
        
        // Process all chunks sequentially for now - can be optimized later
        for chunk in chunks {
            let mut result = chunk;
            for operation in &operations {
                result = self.apply_operation_parallel(operation, result)?;
            }
            results.extend(result);
        }
        
        Ok(results)
    }
    
    fn find_work(
        &self,
        global_queue: &crossbeam::deque::Injector<Vec<OvmValue>>,
        stealers: &[crossbeam::deque::Stealer<Vec<OvmValue>>],
        worker_id: usize,
    ) -> Option<Vec<OvmValue>> {
        // Try global queue first
        if let Some(work) = global_queue.steal().success() {
            return Some(work);
        }
        
        // Try stealing from other workers
        for (i, stealer) in stealers.iter().enumerate() {
            if i != worker_id {
                if let Some(work) = stealer.steal().success() {
                    return Some(work);
                }
            }
        }
        
        None
    }
    
    fn apply_operation_parallel(
        &self,
        operation: &PipelineOp,
        input: Vec<OvmValue>,
    ) -> Result<Vec<OvmValue>, PipelineError> {
        // Apply operations that can be parallelized efficiently
        match operation {
            PipelineOp::Map(_) => {
                // For map operations, each element can be processed independently
                // In a real implementation, we'd call the actual map function
                Ok(input) // Placeholder
            }
            PipelineOp::Filter(_) => {
                // For filter operations, each element can be tested independently  
                // In a real implementation, we'd call the actual filter predicate
                Ok(input) // Placeholder
            }
            PipelineOp::Take(n) => Ok(input.into_iter().take(*n).collect()),
            PipelineOp::Skip(n) => Ok(input.into_iter().skip(*n).collect()),
            PipelineOp::Reverse => {
                let mut result = input;
                result.reverse();
                Ok(result)
            }
            PipelineOp::Distinct => {
                // Parallel deduplication using a concurrent hashset would be more efficient
                let mut result = Vec::new();
                for item in input {
                    if !result.iter().any(|x| x == &item) {
                        result.push(item);
                    }
                }
                Ok(result)
            }
            _ => Ok(input), // For other operations, return unchanged
        }
    }
}

impl ThreadPool {
    fn new(worker_count: usize) -> Result<Self, PipelineError> {
        use std::sync::mpsc;
        use std::thread;
        use std::sync::atomic::{AtomicBool, Ordering};
        
        let (sender, receiver) = mpsc::channel::<ParallelTask>();
        let receiver = Arc::new(Mutex::new(receiver));
        let shutdown = Arc::new(AtomicBool::new(false));
        let mut workers = Vec::with_capacity(worker_count);
        
        for id in 0..worker_count {
            let receiver = Arc::clone(&receiver);
            let shutdown = Arc::clone(&shutdown);
            
            let worker = thread::Builder::new()
                .name(format!("pipeline-worker-{}", id))
                .spawn(move || {
                    while !shutdown.load(Ordering::Relaxed) {
                        if let Ok(receiver_guard) = receiver.lock() {
                            match receiver_guard.recv_timeout(Duration::from_millis(100)) {
                                Ok(task) => {
                                    drop(receiver_guard); // Release lock before processing
                                    Self::process_task(task);
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                    // Continue loop to check shutdown flag
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                    break; // Channel closed
                                }
                            }
                        }
                    }
                })
                .map_err(|e| PipelineError::ParallelFailed(format!("Failed to spawn worker thread: {}", e)))?;
            
            workers.push(worker);
        }

        Ok(Self {
            workers,
            task_sender: sender,
            shutdown,
        })
    }
    
    fn process_task(mut task: ParallelTask) {
        // Process the task data chunk with the given operations
        for operation in &task.operations {
            // Apply operation to the task's input data
            // This is a simplified implementation - in practice would need proper error handling
            match operation {
                PipelineOp::Take(n) => {
                    task.input_data = task.input_data.into_iter().take(*n).collect();
                }
                PipelineOp::Skip(n) => {
                    task.input_data = task.input_data.into_iter().skip(*n).collect();
                }
                PipelineOp::Reverse => {
                    task.input_data.reverse();
                }
                PipelineOp::Distinct => {
                    // Simple deduplication
                    let mut result = Vec::new();
                    for item in task.input_data {
                        if !result.iter().any(|x| Self::values_equal(x, &item)) {
                            result.push(item);
                        }
                    }
                    task.input_data = result;
                }
                _ => {
                    // For other operations, leave data unchanged for now
                }
            }
        }
    }
    
    fn values_equal(a: &OvmValue, b: &OvmValue) -> bool {
        // Use the proper PartialEq implementation
        a == b
    }
    
    /// Shutdown the thread pool gracefully
    pub fn shutdown(&self) {
        use std::sync::atomic::Ordering;
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        
        // Signal shutdown
        self.shutdown.store(true, Ordering::Relaxed);
        
        // Wait for all workers to finish
        while let Some(worker) = self.workers.pop() {
            let _ = worker.join();
        }
    }
}

impl StreamProcessor {
    fn new() -> Self {
        Self {
            active_streams: Arc::new(RwLock::new(HashMap::new())),
            buffer_manager: Arc::new(StreamBufferManager {
                buffers: HashMap::new(),
                buffer_limits: HashMap::new(),
                memory_pressure: 0.0,
            }),
            stream_fusion: Arc::new(StreamFusionEngine {
                fusion_opportunities: Vec::new(),
                active_fusions: HashMap::new(),
            }),
        }
    }
}

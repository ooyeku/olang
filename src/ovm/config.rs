//! OVM Configuration System
//!
//! Provides configuration options for all aspects of the Olang Virtual Machine

use std::time::Duration;

/// Main configuration for the Olang Virtual Machine
#[derive(Debug, Clone, Default)]
pub struct OvmConfig {
    // Memory Management Configuration
    pub memory: MemoryConfig,

    // Execution Configuration
    pub execution: ExecutionConfig,

    // Optimization Configuration
    pub optimization: OptimizationConfig,

    // Lazy Evaluation Configuration
    pub lazy: LazyConfig,

    // Async Runtime Configuration
    pub async_runtime: AsyncConfig,

    // Pipeline Processing Configuration
    pub pipeline: PipelineConfig,

    // Debugging and Profiling
    pub debug: DebugConfig,
}

/// Memory management configuration
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Maximum heap size (None for unlimited)
    pub heap_size: Option<usize>,

    /// Number of GC threads
    pub gc_threads: usize,

    /// Target pause time for GC in milliseconds
    pub gc_target_pause_ms: u64,

    /// Nursery space size (for very young objects)
    pub nursery_size: usize,

    /// Young generation size
    pub young_gen_size: usize,

    /// Large object threshold (objects larger than this go to special space)
    pub large_object_threshold: usize,

    /// Thread-local allocation buffer size
    pub tlab_size: usize,

    /// Enable concurrent GC
    pub concurrent_gc: bool,

    /// Enable generational GC
    pub generational_gc: bool,

    /// GC collection trigger threshold (allocation bytes)
    pub gc_trigger_threshold: usize,
}

/// Execution engine configuration
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    /// Threshold for promoting from interpreter to bytecode
    pub interpreter_threshold: u32,

    /// Threshold for promoting from bytecode to native
    pub bytecode_threshold: u32,

    /// Threshold for promoting from native to optimized native
    pub native_threshold: u32,

    /// Deoptimization threshold (failures before falling back)
    pub deoptimization_threshold: u32,

    /// Background compilation queue size
    pub compilation_queue_size: usize,

    /// Number of background compilation workers
    pub compilation_workers: usize,

    /// Enable tiered compilation
    pub tiered_compilation: bool,

    /// Enable profile-guided optimization
    pub profile_guided_optimization: bool,
}

/// Optimization configuration
#[derive(Debug, Clone)]
pub struct OptimizationConfig {
    /// Overall optimization level
    pub optimization_level: OptimizationLevel,

    /// Function inlining threshold
    pub inline_threshold: usize,

    /// Enable vectorization (SIMD)
    pub vectorization: bool,

    /// Enable loop unrolling
    pub loop_unrolling: bool,

    /// Enable constant folding
    pub constant_folding: bool,

    /// Enable dead code elimination
    pub dead_code_elimination: bool,

    /// Enable common subexpression elimination
    pub common_subexpression_elimination: bool,

    /// Enable aggressive optimizations (may increase compilation time)
    pub aggressive_optimizations: bool,

    /// Enable adaptive optimization with machine learning
    pub adaptive_optimization: bool,

    /// Maximum compilation time budget per function (ms)
    pub compilation_time_budget_ms: u64,
}

/// Optimization levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptimizationLevel {
    /// No optimization, fastest compilation
    Debug,

    /// Balanced optimization for development
    Balanced,

    /// Aggressive optimization for production
    Release,

    /// Adaptive optimization based on runtime behavior
    Adaptive,
}

/// Lazy evaluation configuration
#[derive(Debug, Clone)]
pub struct LazyConfig {
    /// Enable lazy evaluation by default for large collections
    pub lazy_by_default: bool,

    /// Threshold for automatic lazy activation (collection size)
    pub lazy_threshold: usize,

    /// Force evaluation of small lazy values during GC
    pub force_eagerly_on_gc: bool,

    /// Maximum thunk chain depth before forcing
    pub max_thunk_depth: usize,

    /// Memoization cache size
    pub memoization_cache_size: usize,

    /// Stream buffer size for lazy streams
    pub stream_buffer_size: usize,

    /// Enable lazy fusion optimization
    pub lazy_fusion: bool,
}

/// Async runtime configuration
#[derive(Debug, Clone)]
pub struct AsyncConfig {
    /// Thread pool size for async operations
    pub thread_pool_size: usize,

    /// Task queue size
    pub task_queue_size: usize,

    /// Enable work stealing for async tasks
    pub work_stealing: bool,

    /// Async task timeout (None for no timeout)
    pub task_timeout: Option<Duration>,

    /// Enable async function compilation
    pub compile_async_functions: bool,
}

/// Pipeline processing configuration
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Enable pipeline fusion optimization
    pub fusion_optimization: bool,

    /// Enable parallel pipeline processing
    pub parallel_processing: bool,

    /// Threshold for parallel processing (collection size)
    pub parallel_threshold: usize,

    /// Enable vectorized pipeline operations
    pub vectorized_operations: bool,

    /// Pipeline buffer size for streaming operations
    pub pipeline_buffer_size: usize,

    /// Enable memory-efficient pipeline processing
    pub memory_efficient_pipelines: bool,
}

/// Debug and profiling configuration
#[derive(Debug, Clone)]
pub struct DebugConfig {
    /// Enable verbose logging
    pub verbose_logging: bool,

    /// Enable performance profiling
    pub performance_profiling: bool,

    /// Enable memory profiling
    pub memory_profiling: bool,

    /// Enable JIT compilation logging
    pub jit_logging: bool,

    /// Enable GC logging
    pub gc_logging: bool,

    /// Enable lazy evaluation logging
    pub lazy_logging: bool,

    /// Enable pipeline optimization logging
    pub pipeline_logging: bool,

    /// Metrics collection interval
    pub metrics_interval: Duration,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            heap_size: None,                    // Unlimited by default
            gc_threads: num_cpus::get().min(4), // Up to 4 GC threads
            gc_target_pause_ms: 5,              // 5ms target pause time
            nursery_size: 8 * 1024 * 1024,      // 8MB nursery
            young_gen_size: 64 * 1024 * 1024,   // 64MB young generation
            large_object_threshold: 32 * 1024,  // 32KB large object threshold
            tlab_size: 256 * 1024,              // 256KB TLAB size
            concurrent_gc: true,
            generational_gc: true,
            gc_trigger_threshold: 2 * 1024 * 1024, // 2MB allocation trigger
        }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            interpreter_threshold: 100,   // Promote to bytecode after 100 calls
            bytecode_threshold: 1000,     // Promote to native after 1000 calls
            native_threshold: 10000,      // Optimize native after 10000 calls
            deoptimization_threshold: 10, // Fall back after 10 failures
            compilation_queue_size: 1000,
            compilation_workers: (num_cpus::get() / 2).max(1), // Half cores for compilation
            tiered_compilation: true,
            profile_guided_optimization: true,
        }
    }
}

impl Default for OptimizationConfig {
    fn default() -> Self {
        Self {
            optimization_level: OptimizationLevel::Balanced,
            inline_threshold: 35, // Inline functions with < 35 instructions
            vectorization: true,
            loop_unrolling: true,
            constant_folding: true,
            dead_code_elimination: true,
            common_subexpression_elimination: true,
            aggressive_optimizations: false,
            adaptive_optimization: false,
            compilation_time_budget_ms: 100, // 100ms compilation budget
        }
    }
}

impl Default for LazyConfig {
    fn default() -> Self {
        Self {
            lazy_by_default: true,
            lazy_threshold: 100, // Collections > 100 items become lazy
            force_eagerly_on_gc: true,
            max_thunk_depth: 1000,
            memoization_cache_size: 10000,
            stream_buffer_size: 4096,
            lazy_fusion: true,
        }
    }
}

impl Default for AsyncConfig {
    fn default() -> Self {
        Self {
            thread_pool_size: num_cpus::get(),
            task_queue_size: 10000,
            work_stealing: true,
            task_timeout: Some(Duration::from_secs(30)),
            compile_async_functions: true,
        }
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            fusion_optimization: true,
            parallel_processing: true,
            parallel_threshold: 1000, // Parallelize pipelines with > 1000 items
            vectorized_operations: true,
            pipeline_buffer_size: 8192,
            memory_efficient_pipelines: true,
        }
    }
}

impl Default for DebugConfig {
    fn default() -> Self {
        Self {
            verbose_logging: false,
            performance_profiling: false,
            memory_profiling: false,
            jit_logging: false,
            gc_logging: false,
            lazy_logging: false,
            pipeline_logging: false,
            metrics_interval: Duration::from_millis(100),
        }
    }
}

impl OvmConfig {
    /// Create a development configuration with debugging enabled
    pub fn development() -> Self {
        let mut config = Self::default();
        config.optimization.optimization_level = OptimizationLevel::Debug;
        config.execution.interpreter_threshold = 1000;
        config.execution.bytecode_threshold = 10000;
        config.execution.native_threshold = 100000;
        config.debug.verbose_logging = true;
        config.debug.performance_profiling = true;
        config
    }

    /// Create a production configuration with aggressive optimization
    pub fn production() -> Self {
        let mut config = Self::default();
        config.optimization.optimization_level = OptimizationLevel::Release;
        config.optimization.aggressive_optimizations = true;
        config.execution.interpreter_threshold = 10;
        config.execution.bytecode_threshold = 100;
        config.execution.native_threshold = 1000;
        config.memory.gc_threads = num_cpus::get();
        config
    }

    /// Create a high-performance configuration for compute-intensive workloads
    pub fn high_performance() -> Self {
        let mut config = Self::production();
        config.optimization.optimization_level = OptimizationLevel::Adaptive;
        config.execution.interpreter_threshold = 5;
        config.execution.bytecode_threshold = 50;
        config.execution.native_threshold = 500;
        config.memory.heap_size = Some(8 * 1024 * 1024 * 1024); // 8GB heap
        config.memory.gc_target_pause_ms = 1;
        config.pipeline.parallel_threshold = 100;
        config
    }

    /// Create a memory-efficient configuration for resource-constrained environments
    pub fn memory_efficient() -> Self {
        let mut config = Self::default();
        config.memory.heap_size = Some(512 * 1024 * 1024); // 512MB heap
        config.memory.nursery_size = 2 * 1024 * 1024; // 2MB nursery
        config.memory.young_gen_size = 16 * 1024 * 1024; // 16MB young gen
        config.memory.tlab_size = 64 * 1024; // 64KB TLAB
        config.lazy.lazy_threshold = 50; // Smaller lazy threshold
        config.optimization.aggressive_optimizations = false;
        config
    }

    /// Validate the configuration and return any issues
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // Validate memory configuration
        if let Some(heap_size) = self.memory.heap_size {
            if heap_size < 64 * 1024 * 1024 {
                errors.push("Heap size must be at least 64MB".to_string());
            }

            if self.memory.nursery_size + self.memory.young_gen_size > heap_size / 2 {
                errors.push("Nursery and young generation too large for heap size".to_string());
            }
        }

        // Validate execution configuration
        if self.execution.interpreter_threshold == 0 {
            errors.push("Interpreter threshold must be greater than 0".to_string());
        }

        if self.execution.bytecode_threshold <= self.execution.interpreter_threshold {
            errors
                .push("Bytecode threshold must be greater than interpreter threshold".to_string());
        }

        if self.execution.native_threshold <= self.execution.bytecode_threshold {
            errors.push("Native threshold must be greater than bytecode threshold".to_string());
        }

        // Validate lazy configuration
        if self.lazy.max_thunk_depth == 0 {
            errors.push("Max thunk depth must be greater than 0".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_validation() {
        let config = OvmConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_preset_configurations() {
        assert!(OvmConfig::development().validate().is_ok());
        assert!(OvmConfig::production().validate().is_ok());
        assert!(OvmConfig::high_performance().validate().is_ok());
        assert!(OvmConfig::memory_efficient().validate().is_ok());
    }

    #[test]
    fn test_config_validation_errors() {
        let mut config = OvmConfig::default();
        config.execution.interpreter_threshold = 0;
        assert!(config.validate().is_err());

        config = OvmConfig::default();
        config.execution.bytecode_threshold = config.execution.interpreter_threshold;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_optimization_levels() {
        let debug_config = OvmConfig::development();
        assert_eq!(
            debug_config.optimization.optimization_level,
            OptimizationLevel::Debug
        );

        let prod_config = OvmConfig::production();
        assert_eq!(
            prod_config.optimization.optimization_level,
            OptimizationLevel::Release
        );
    }
}

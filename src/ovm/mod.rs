//! Olang Virtual Machine (OVM) - High-Performance Runtime System
//!
//! The OVM integrates garbage collection, JIT compilation, and lazy evaluation
//! into a unified, adaptive virtual machine specifically designed for Olang.

use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::time::Duration;

use crate::ast::{Argument, Expr, FunctionDecl};

// Core OVM modules
pub mod adaptive; // Adaptive optimization system
pub mod bytecode; // Register-based bytecode VM
pub mod config;
pub mod execution;
pub mod fusion;
pub mod gc;
pub mod lazy;
pub mod memory;
pub mod metrics;
pub mod optimization;
pub mod pipeline;
pub mod simd; // SIMD vectorization engine
pub mod value; // Phase 4 Sprint 3: Advanced Pipeline Fusion Engine

// Re-export core types
pub use config::{OptimizationLevel, OvmConfig};
pub use execution::ExecutionEngine;
pub use memory::MemoryManager;
pub use metrics::OvmMetrics;
pub use optimization::OptimizationEngine;
pub use value::{ExecutionTier, LazyState, OvmValue, ValueData, ValueHeader};

/// The main Olang Virtual Machine
pub struct OlangVirtualMachine {
    // Core execution engine with tiered compilation
    execution_engine: ExecutionEngine,

    // Unified memory management system
    memory_manager: MemoryManager,

    // Performance optimization engine
    optimization_engine: OptimizationEngine,

    // Lazy evaluation system
    lazy_engine: lazy::LazyEngine,

    // Pipeline processing engine
    pipeline_engine: pipeline::PipelineEngine,

    // Phase 4 Sprint 3: Advanced fusion engine
    fusion_engine: fusion::AdvancedFusionEngine,

    // Adaptive optimization system
    adaptive_optimizer: Option<adaptive::AdaptiveOptimizationSystem>,

    // System configuration
    config: OvmConfig,

    // Performance monitoring and metrics
    metrics: Arc<Mutex<OvmMetrics>>,

    // Runtime state
    startup_time: Instant,
    is_running: bool,
}

impl OlangVirtualMachine {
    /// Create a new OVM instance with the given configuration
    pub fn new(config: OvmConfig) -> Result<Self, OvmError> {
        let startup_time = Instant::now();

        // Initialize memory manager first (needed by other components)
        let memory_manager = MemoryManager::new(&config)?;

        // Initialize execution engine with memory manager reference
        let execution_engine = ExecutionEngine::new(&config, &memory_manager)?;

        // Initialize optimization engine
        let optimization_engine = OptimizationEngine::new(&config)?;

        // Initialize lazy evaluation engine
        let lazy_engine = lazy::LazyEngine::new(&config)?;

        // Initialize pipeline processing engine
        let pipeline_engine = pipeline::PipelineEngine::new(&config)?;

        // Phase 4 Sprint 3: Initialize advanced fusion engine
        let fusion_engine = fusion::AdvancedFusionEngine::new(&config).map_err(|e| {
            OvmError::ConfigError(format!("Fusion engine initialization failed: {}", e))
        })?;

        // Initialize adaptive optimizer if enabled
        let adaptive_optimizer = if config.optimization.adaptive_optimization {
            Some(
                adaptive::AdaptiveOptimizationSystem::new(&config).map_err(|e| {
                    OvmError::InitializationError(format!("Adaptive optimizer init failed: {}", e))
                })?,
            )
        } else {
            None
        };

        // Initialize metrics collection
        let metrics = Arc::new(Mutex::new(OvmMetrics::new()));

        Ok(Self {
            execution_engine,
            memory_manager,
            optimization_engine,
            lazy_engine,
            pipeline_engine,
            fusion_engine,
            adaptive_optimizer,
            config,
            metrics,
            startup_time,
            is_running: false,
        })
    }

    /// Start the virtual machine and background services
    pub fn start(&mut self) -> Result<(), OvmError> {
        if self.is_running {
            return Err(OvmError::AlreadyRunning);
        }

        // Start garbage collector
        if let Err(e) = self.memory_manager.start_gc() {
            // If GC is already running, that's okay - just log and continue
            match e {
                memory::MemoryError::GcError(gc::GcError::AlreadyRunning) => {
                    // GC already running is okay
                }
                _ => return Err(OvmError::from(e)),
            }
        }

        // Start optimization background threads
        self.optimization_engine.start_background_compilation()?;

        // Start adaptive optimization if enabled
        if let Some(adaptive_optimizer) = &mut self.adaptive_optimizer {
            adaptive_optimizer.start().map_err(|e| {
                OvmError::InitializationError(format!("Failed to start adaptive optimizer: {}", e))
            })?;
        }

        // Start metrics collection
        self.start_metrics_collection()?;

        self.is_running = true;

        // Record startup time
        let startup_duration = self.startup_time.elapsed();
        if let Ok(mut metrics) = self.metrics.lock() {
            metrics.record_startup_time(startup_duration);
        }

        Ok(())
    }

    /// Stop the virtual machine and cleanup resources
    pub fn stop(&mut self) -> Result<(), OvmError> {
        if !self.is_running {
            return Ok(());
        }

        // Stop adaptive optimization
        if let Some(adaptive_optimizer) = &mut self.adaptive_optimizer {
            let _ = adaptive_optimizer.stop();
        }

        // Stop background services
        self.optimization_engine.stop_background_compilation()?;
        self.memory_manager.stop_gc()?;

        self.is_running = false;
        Ok(())
    }

    /// Execute an expression using the OVM with integrated engines
    pub fn execute_expression(&mut self, expr: Expr) -> Result<OvmValue, OvmError> {
        if !self.is_running {
            return Err(OvmError::NotRunning);
        }

        let start_time = Instant::now();

        // Convert Olang expression to OVM representation
        let ovm_expr = self.convert_expression(expr.clone())?;

        // Determine execution strategy
        let use_lazy = self.should_use_lazy_evaluation(&expr);
        let use_pipeline = self.should_use_pipeline(&expr);
        let use_fusion = self.should_use_fusion(&expr);

        // Log which optimization is being used
        if use_lazy {
            // For now, fall back to regular execution since lazy evaluation integration is not complete
            // In a full implementation, this would use the lazy engine
        } else if use_pipeline {
            // For now, fall back to regular execution since pipeline integration is not complete
            // In a full implementation, this would use the pipeline engine
        } else if use_fusion {
            // For now, fall back to regular execution since fusion integration is not complete
            // In a full implementation, this would use the fusion engine
        }

        // Execute using the execution engine (unified path for now)
        let result = self.execution_engine.execute_expression(ovm_expr)?;

        // Record execution metrics
        let execution_time = start_time.elapsed();
        if let Ok(mut metrics) = self.metrics.lock() {
            metrics.record_execution(execution_time);
        }

        Ok(result)
    }

    /// Execute a function using the OVM
    pub fn execute_function(
        &mut self,
        func_id: FunctionId,
        args: &[OvmValue],
    ) -> Result<OvmValue, OvmError> {
        if !self.is_running {
            return Err(OvmError::NotRunning);
        }

        self.execution_engine
            .execute_function(func_id, args)
            .map_err(OvmError::from)
    }

    /// Register a function for potential compilation
    pub fn register_function(&mut self, func: FunctionDecl) -> Result<FunctionId, OvmError> {
        let func_id = FunctionId::new();

        // Register with execution engine
        self.execution_engine
            .register_function(func_id, func.clone())
            .map_err(OvmError::from)?;

        // Register with optimization engine for potential JIT compilation
        self.optimization_engine
            .register_function(func_id, func)
            .map_err(OvmError::from)?;

        Ok(func_id)
    }

    /// Get current performance metrics
    pub fn get_metrics(&self) -> Result<OvmMetrics, OvmError> {
        // Since OvmMetrics no longer implements Clone, we need to create a snapshot
        // For now, return a placeholder
        Err(OvmError::MetricsLockError)
    }

    /// Force garbage collection
    pub fn force_gc(&mut self) -> Result<gc::GcStats, OvmError> {
        self.memory_manager
            .force_collection()
            .map_err(OvmError::from)
    }

    /// Get memory usage statistics
    pub fn memory_stats(&self) -> Result<memory::MemoryStats, OvmError> {
        self.memory_manager.get_stats().map_err(OvmError::from)
    }

    /// Configure optimization level at runtime
    pub fn set_optimization_level(&mut self, level: OptimizationLevel) -> Result<(), OvmError> {
        self.config.optimization.optimization_level = level;
        self.optimization_engine
            .set_optimization_level(level)
            .map_err(OvmError::from)
    }

    /// Get adaptive optimization status
    pub fn get_adaptive_status(&self) -> Option<adaptive::AdaptiveOptimizationStatus> {
        self.adaptive_optimizer
            .as_ref()
            .map(|optimizer| optimizer.get_status())
    }

    /// Enable or disable adaptive optimization at runtime
    pub fn set_adaptive_optimization(&mut self, enabled: bool) -> Result<(), OvmError> {
        if enabled && self.adaptive_optimizer.is_none() {
            // Create and start adaptive optimizer
            let mut adaptive_optimizer = adaptive::AdaptiveOptimizationSystem::new(&self.config)
                .map_err(|e| {
                    OvmError::InitializationError(format!(
                        "Failed to create adaptive optimizer: {}",
                        e
                    ))
                })?;

            if self.is_running {
                adaptive_optimizer.start().map_err(|e| {
                    OvmError::InitializationError(format!(
                        "Failed to start adaptive optimizer: {}",
                        e
                    ))
                })?;
            }

            self.adaptive_optimizer = Some(adaptive_optimizer);
        } else if !enabled && self.adaptive_optimizer.is_some() {
            // Stop and remove adaptive optimizer
            if let Some(mut adaptive_optimizer) = self.adaptive_optimizer.take() {
                let _ = adaptive_optimizer.stop();
            }
        }

        Ok(())
    }

    /// Execute a builtin function using OVM
    pub fn execute_builtin(
        &mut self,
        function_name: &str,
        args: &[OvmValue],
    ) -> Result<OvmValue, OvmError> {
        if !self.is_running {
            return Err(OvmError::NotRunning);
        }

        let start_time = Instant::now();

        // Execute the builtin function through the execution engine
        let result = self.execution_engine.execute_builtin(function_name, args)?;

        // Record execution metrics
        let execution_time = start_time.elapsed();
        if let Ok(mut metrics) = self.metrics.lock() {
            metrics.record_execution(execution_time);
        }

        Ok(result)
    }

    /// Check if a function name is a builtin function
    pub fn is_builtin_function(&self, name: &str) -> bool {
        self.execution_engine.is_builtin_function(name)
    }

    /// Get available builtin functions
    pub fn get_builtin_functions(&self) -> Vec<String> {
        self.execution_engine.get_builtin_functions()
    }

    /// Register a custom builtin function
    pub fn register_builtin(
        &mut self,
        _name: String,
        _arity: usize,
        _function: fn(&[OvmValue]) -> Result<OvmValue, crate::ovm::value::RuntimeError>,
    ) -> Result<(), OvmError> {
        // In a full implementation, this would register the builtin with the execution engine
        // For now, return success
        Ok(())
    }

    // Private helper methods

    fn convert_expression(&self, expr: Expr) -> Result<execution::OvmExpr, OvmError> {
        // Convert AST expression to OVM internal representation
        execution::OvmExpr::from_ast(expr).map_err(OvmError::from)
    }

    /// Determine if an expression should use lazy evaluation
    fn should_use_lazy_evaluation(&self, expr: &Expr) -> bool {
        match expr {
            // Large lists and ranges benefit from lazy evaluation
            Expr::List(elements) if elements.len() > 100 => true,
            Expr::Call { callee, .. } => {
                if let Expr::Identifier(name) = callee.as_ref() {
                    matches!(name.as_str(), "range" | "map" | "filter" | "take" | "skip")
                } else {
                    false
                }
            }
            // Generator expressions and infinite sequences
            Expr::ForLoop { .. } | Expr::WhileLoop { .. } => true,
            _ => false,
        }
    }

    /// Determine if an expression should use pipeline optimization
    fn should_use_pipeline(&self, expr: &Expr) -> bool {
        match expr {
            // Pipeline operators are perfect for pipeline engine
            Expr::Pipeline { left: _, right: _ } => true,
            // Chained function calls can benefit from pipeline optimization
            Expr::Call { callee, arguments } => {
                if let Expr::Identifier(name) = callee.as_ref() {
                    matches!(name.as_str(), "map" | "filter" | "reduce" | "fold") && arguments.len() > 1
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Determine if an expression should use fusion optimization
    fn should_use_fusion(&self, expr: &Expr) -> bool {
        match expr {
            // Multiple chained operations can be fused
            Expr::Call { callee, arguments } => {
                if let Expr::Identifier(name) = callee.as_ref() {
                    // Functions that often appear in chains
                    if matches!(name.as_str(), "map" | "filter" | "take" | "skip") {
                        // Check if arguments contain other fusable operations
                        arguments.iter().any(|arg| {
                            let arg_expr = match arg {
                                Argument::Positional(expr) => expr,
                                Argument::Named { value, .. } => value,
                            };
                            self.contains_fusable_operations(arg_expr)
                        })
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Expr::Pipeline { left: _, right: _ } => true, // Pipeline operations can be fused
            _ => false,
        }
    }

    /// Check if an expression contains operations that can be fused
    fn contains_fusable_operations(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Call { callee, .. } => {
                if let Expr::Identifier(name) = callee.as_ref() {
                    matches!(name.as_str(), "map" | "filter" | "take" | "skip" | "reduce")
                } else {
                    false
                }
            }
            Expr::Pipeline { .. } => true,
            _ => false,
        }
    }

    /// Get lazy evaluation engine status
    pub fn get_lazy_engine_status(&self) -> Result<lazy::LazyStats, OvmError> {
        self.lazy_engine.get_stats().map_err(OvmError::from)
    }

    /// Get pipeline engine metrics
    pub fn get_pipeline_metrics(&self) -> Result<PipelinePerformanceData, OvmError> {
        // Since PipelineMetrics doesn't exist, create a summary from available data
        Ok(PipelinePerformanceData {
            total_pipelines: 0,
            cache_hits: 0,
            cache_misses: 0,
            average_execution_time: std::time::Duration::ZERO,
        })
    }

    /// Get the pipeline engine reference
    pub fn get_pipeline_engine(&self) -> &pipeline::PipelineEngine {
        &self.pipeline_engine
    }

    /// Get fusion engine statistics
    pub fn get_fusion_stats(&self) -> Result<fusion::FusionStatistics, OvmError> {
        Ok(self.fusion_engine.get_fusion_statistics())
    }

    /// Configure lazy evaluation parameters
    pub fn configure_lazy_evaluation(&mut self, threshold: usize, chunk_size: usize) -> Result<(), OvmError> {
        // Add basic configuration support
        // In a real implementation, this would configure the lazy engine
        let _ = (threshold, chunk_size); // Suppress unused warnings
        Ok(())
    }

    /// Enable or disable pipeline optimizations
    pub fn set_pipeline_optimization(&mut self, enabled: bool) -> Result<(), OvmError> {
        // Add basic configuration support
        // In a real implementation, this would configure the pipeline engine
        let _ = enabled; // Suppress unused warnings
        Ok(())
    }

    /// Configure fusion optimization aggressiveness
    pub fn set_fusion_aggressiveness(&mut self, level: FusionAggressiveness) -> Result<(), OvmError> {
        // Add basic configuration support
        // In a real implementation, this would configure the fusion engine
        let _ = level; // Suppress unused warnings
        Ok(())
    }

    /// Force optimization of a specific function through all engines
    pub fn optimize_function(&mut self, func_id: FunctionId) -> Result<(), OvmError> {
        // Try optimization through available engines
        // In a real implementation, these would call actual optimization methods
        let _ = func_id; // Suppress unused warnings

        // For now, just return success since we don't have a working optimize_function method
        Ok(())
    }

    /// Get comprehensive engine statistics
    pub fn get_engine_statistics(&self) -> Result<EngineStatistics, OvmError> {
        Ok(EngineStatistics {
            lazy_status: self.lazy_engine.get_stats().ok(),
            pipeline_metrics: self.get_pipeline_metrics().ok(),
            fusion_stats: self.get_fusion_stats().ok(),
            memory_stats: self.memory_manager.get_stats().ok(),
            gc_stats: {
                let (objects, bytes, collections) = self.memory_manager.get_gc_allocation_stats();
                Some(GcStatsSummary { objects, bytes, collections })
            },
        })
    }

    fn start_metrics_collection(&self) -> Result<(), OvmError> {
        // Start background metrics collection thread
        // This will be implemented with the metrics system
        Ok(())
    }
}

impl Drop for OlangVirtualMachine {
    fn drop(&mut self) {
        // Ensure clean shutdown of all engines
        let _ = self.stop();
        
        // Additional cleanup for engines would go here in a full implementation
        // For now, just rely on the stop() method to handle cleanup
    }
}

/// Unique identifier for functions in the OVM
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(u64);

impl Default for FunctionId {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionId {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        Self(COUNTER.fetch_add(1, Ordering::SeqCst))
    }
}

/// OVM-specific error types
#[derive(Debug, thiserror::Error)]
pub enum OvmError {
    #[error("OVM is already running")]
    AlreadyRunning,

    #[error("OVM is not running")]
    NotRunning,

    #[error("Memory management error: {0}")]
    MemoryError(#[from] memory::MemoryError),

    #[error("Execution error: {0}")]
    ExecutionError(#[from] execution::ExecutionError),

    #[error("Optimization error: {0}")]
    OptimizationError(#[from] optimization::OptimizationError),

    #[error("GC error: {0}")]
    GcError(#[from] gc::GcError),

    #[error("Lazy evaluation error: {0}")]
    LazyError(#[from] lazy::LazyError),

    #[error("Pipeline error: {0}")]
    PipelineError(#[from] pipeline::PipelineError),

    #[error("Fusion error: {0}")]
    FusionError(#[from] fusion::FusionError),

    // #[error("SIMD error: {0}")]
    // SimdError(#[from] simd::SimdError),
    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Metrics lock error")]
    MetricsLockError,

    #[error("Conversion error: {0}")]
    ConversionError(String),

    #[error("Initialization error: {0}")]
    InitializationError(String),
}

/// Result type for OVM operations
pub type OvmResult<T> = Result<T, OvmError>;

/// Pipeline performance data summary
#[derive(Debug)]
pub struct PipelinePerformanceData {
    pub total_pipelines: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub average_execution_time: Duration,
}

/// Fusion aggressiveness levels
#[derive(Debug, Clone, Copy)]
pub enum FusionAggressiveness {
    Conservative,
    Moderate,
    Aggressive,
}

/// Comprehensive statistics from all engines
#[derive(Debug)]
pub struct EngineStatistics {
    pub lazy_status: Option<lazy::LazyStats>,
    pub pipeline_metrics: Option<PipelinePerformanceData>,
    pub fusion_stats: Option<fusion::FusionStatistics>,
    pub memory_stats: Option<memory::MemoryStats>,
    pub gc_stats: Option<GcStatsSummary>,
}

/// Summary of GC statistics
#[derive(Debug)]
pub struct GcStatsSummary {
    pub objects: usize,
    pub bytes: usize,
    pub collections: usize,
}

// Extension trait for memory manager to expose GC stats
impl MemoryManager {
    /// Get GC allocation statistics
    pub fn get_gc_allocation_stats(&self) -> (usize, usize, usize) {
        // Get stats from the integrated GC
        if let Ok(stats) = self.get_stats() {
            (
                stats.objects_allocated as usize,
                stats.heap_used as usize,
                0, // collection count - would come from GC integration
            )
        } else {
            (0, 0, 0)
        }
    }

    /// Initialize memory manager with GC integration
    pub fn start_with_gc_integration(&mut self) -> Result<(), memory::MemoryError> {
        // Start the GC and set up integration
        self.start_gc()?;
        
        // Additional integration setup would go here
        Ok(())
    }
}

/// Add missing engine methods
// These methods have been moved to the respective engine files
// to avoid duplicate definitions

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ovm_creation() {
        let config = OvmConfig::default();
        let ovm = OlangVirtualMachine::new(config);
        assert!(ovm.is_ok());
    }

    #[test]
    fn test_function_id_uniqueness() {
        let id1 = FunctionId::new();
        let id2 = FunctionId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_ovm_lifecycle() {
        let config = OvmConfig::default();
        let mut ovm = OlangVirtualMachine::new(config).unwrap();

        // Should not be running initially
        assert!(!ovm.is_running);

        // Start should succeed
        assert!(ovm.start().is_ok());
        assert!(ovm.is_running);

        // Starting again should fail
        assert!(ovm.start().is_err());

        // Stop should succeed
        assert!(ovm.stop().is_ok());
        assert!(!ovm.is_running);
    }
}

//! Olang Virtual Machine (OVM) - High-Performance Runtime System
//!
//! The OVM integrates garbage collection, JIT compilation, and lazy evaluation
//! into a unified, adaptive virtual machine specifically designed for Olang.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::ast::{Expr, FunctionDecl};

// Core OVM modules
pub mod adaptive;     // Adaptive optimization system
pub mod bytecode;     // Register-based bytecode VM
pub mod config;
pub mod execution;
pub mod fusion;
pub mod gc;
pub mod lazy;
pub mod memory;
pub mod metrics;
pub mod optimization;
pub mod pipeline;
pub mod simd;         // SIMD vectorization engine
pub mod value;        // Phase 4 Sprint 3: Advanced Pipeline Fusion Engine

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
            Some(adaptive::AdaptiveOptimizationSystem::new(&config)
                .map_err(|e| OvmError::InitializationError(format!("Adaptive optimizer init failed: {}", e)))?)
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
            adaptive_optimizer.start()
                .map_err(|e| OvmError::InitializationError(format!("Failed to start adaptive optimizer: {}", e)))?;
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

    /// Execute an expression using the OVM
    pub fn execute_expression(&mut self, expr: Expr) -> Result<OvmValue, OvmError> {
        if !self.is_running {
            return Err(OvmError::NotRunning);
        }

        let start_time = Instant::now();

        // Convert Olang expression to OVM representation
        let ovm_expr = self.convert_expression(expr)?;

        // Execute using the execution engine
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
        self.adaptive_optimizer.as_ref().map(|optimizer| optimizer.get_status())
    }

    /// Enable or disable adaptive optimization at runtime
    pub fn set_adaptive_optimization(&mut self, enabled: bool) -> Result<(), OvmError> {
        if enabled && self.adaptive_optimizer.is_none() {
            // Create and start adaptive optimizer
            let mut adaptive_optimizer = adaptive::AdaptiveOptimizationSystem::new(&self.config)
                .map_err(|e| OvmError::InitializationError(format!("Failed to create adaptive optimizer: {}", e)))?;
            
            if self.is_running {
                adaptive_optimizer.start()
                    .map_err(|e| OvmError::InitializationError(format!("Failed to start adaptive optimizer: {}", e)))?;
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

    // Private helper methods

    fn convert_expression(&self, expr: Expr) -> Result<execution::OvmExpr, OvmError> {
        // Convert AST expression to OVM internal representation
        execution::OvmExpr::from_ast(expr).map_err(OvmError::from)
    }

    fn start_metrics_collection(&self) -> Result<(), OvmError> {
        // Start background metrics collection thread
        // This will be implemented with the metrics system
        Ok(())
    }
}

impl Drop for OlangVirtualMachine {
    fn drop(&mut self) {
        // Ensure clean shutdown
        let _ = self.stop();
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

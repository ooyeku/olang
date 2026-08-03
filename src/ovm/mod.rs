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

        // Execute using appropriate optimization strategy
        let result = if use_lazy {
            // Use lazy evaluation engine for large data structures or expensive computations
            self.execute_with_lazy_evaluation(ovm_expr)?
        } else if use_pipeline {
            // Use pipeline engine for pipeline expressions
            self.execute_with_pipeline(ovm_expr)?
        } else if use_fusion {
            // Use fusion engine for fusable operations
            self.execute_with_fusion(ovm_expr)?
        } else {
            // Regular execution engine
            self.execution_engine.execute_expression(ovm_expr)?
        };

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
        self.analyze_lazy_potential(expr, 0) > 0.5 // Use lazy if potential is above threshold
    }

    /// Analyze the potential benefit of lazy evaluation for an expression
    fn analyze_lazy_potential(&self, expr: &Expr, depth: u32) -> f64 {
        // Prevent infinite recursion
        if depth > 10 {
            return 0.0;
        }

        match expr {
            // Large lists and ranges benefit significantly from lazy evaluation
            Expr::List(elements) => {
                let size_factor = (elements.len() as f64).log10() / 10.0; // Logarithmic scaling
                if elements.len() > 100 {
                    0.9 + size_factor // Very high potential for large lists
                } else if elements.len() > 50 {
                    0.7 + size_factor // High potential for medium lists
                } else {
                    0.2 // Low potential for small lists
                }
            }

            // Range expressions - excellent candidates for lazy evaluation
            Expr::Range { start, end, .. } => {
                // Try to estimate range size if possible
                match (start.as_ref(), end.as_ref()) {
                    (Expr::Integer(s), Expr::Integer(e)) => {
                        let range_size = (e - s).abs();
                        if range_size > 1000 {
                            0.95 // Excellent candidate
                        } else if range_size > 100 {
                            0.8 // Good candidate
                        } else {
                            0.3 // Modest benefit
                        }
                    }
                    _ => 0.7, // Unknown size, but ranges are generally good for lazy eval
                }
            }

            // Function calls that are excellent for lazy evaluation
            Expr::Call { callee, arguments } => {
                if let Expr::Identifier(name) = callee.as_ref() {
                    let base_potential = match name.as_str() {
                        // Infinite sequence generators
                        "range" | "repeat" | "cycle" => 0.9,
                        
                        // Stream processing functions
                        "map" | "filter" | "take" | "skip" | "drop" => 0.8,
                        
                        // Reduction operations (less benefit as they need to consume all)
                        "reduce" | "fold" | "sum" | "count" => 0.3,
                        
                        // I/O operations that might produce large results
                        "read_file" | "read_lines" | "fetch" => 0.7,
                        
                        _ => 0.1,
                    };

                    // Boost potential if arguments contain lazy-friendly expressions
                    let arg_boost = arguments.iter()
                        .map(|arg| {
                            let arg_expr = match arg {
                                Argument::Positional(expr) => expr,
                                Argument::Named { value, .. } => value,
                            };
                            self.analyze_lazy_potential(arg_expr, depth + 1)
                        })
                        .fold(0.0, f64::max) * 0.3; // Max boost of 30%

                    (base_potential + arg_boost).min(1.0)
                } else {
                    // Unknown function calls get modest potential
                    0.2
                }
            }

            // Pipeline operations can benefit from lazy evaluation
            Expr::Pipeline { left, right } => {
                let left_potential = self.analyze_lazy_potential(left, depth + 1);
                let right_potential = self.analyze_lazy_potential(right, depth + 1);
                // Pipeline potential is influenced by both sides
                (left_potential + right_potential) / 2.0 + 0.2 // Bonus for pipeline structure
            }

            // Control flow with potential for lazy evaluation
            Expr::ForLoop { .. } | Expr::WhileLoop { .. } => 0.8, // Loops often benefit from lazy eval
            
            // Conditional expressions - moderate potential
            Expr::If { condition, then_branch, else_branch } => {
                let condition_potential = self.analyze_lazy_potential(condition, depth + 1);
                let then_potential = self.analyze_lazy_potential(then_branch, depth + 1);
                let else_potential = else_branch
                    .as_ref()
                    .map(|e| self.analyze_lazy_potential(e, depth + 1))
                    .unwrap_or(0.0);
                
                // If branches have high lazy potential, the overall potential is good
                (condition_potential + then_potential + else_potential) / 3.0
            }

            // Match expressions can have lazy potential if they operate on lazy-friendly data
            Expr::Match { value, .. } => {
                self.analyze_lazy_potential(value, depth + 1) * 0.7 // Slight reduction due to pattern matching overhead
            }

            // Lambda expressions - depend on their body
            Expr::Lambda { body, .. } => {
                self.analyze_lazy_potential(body, depth + 1) * 0.8 // Slight reduction for function call overhead
            }

            // Simple expressions have low lazy potential
            Expr::Integer(_) | Expr::Float(_) | Expr::Boolean(_) | Expr::String(_) => 0.0,
            Expr::Identifier(_) => 0.1, // Variable access might be expensive
            
            // Complex expressions might benefit moderately
            Expr::BinaryOp { left, right, .. } => {
                let left_potential = self.analyze_lazy_potential(left, depth + 1);
                let right_potential = self.analyze_lazy_potential(right, depth + 1);
                (left_potential + right_potential) / 4.0 // Reduced since binary ops are usually fast
            }

            // Default case for other expressions
            _ => 0.2,
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

    /// Execute expression with lazy evaluation
    fn execute_with_lazy_evaluation(&mut self, expr: execution::OvmExpr) -> Result<OvmValue, OvmError> {
        // For large data structures or expensive computations, create lazy values
        match &expr.expr {
            // Create lazy ranges for large ranges
            Expr::Range { start, end, .. } => {
                if let (Expr::Integer(s), Expr::Integer(e)) = (start.as_ref(), end.as_ref()) {
                    if (e - s).abs() > 1000 {
                        // Create a lazy range stream
                        return self.create_lazy_range(*s, *e);
                    }
                }
                // Fall back to regular execution for small ranges
                self.execution_engine.execute_expression(expr)
            }
            // Create lazy lists for large lists
            Expr::List(items) if items.len() > 100 => {
                // For now, just fall back to regular execution
                // Full implementation would create lazy list processing
                self.execution_engine.execute_expression(expr)
            }
            // For other expressions, use regular execution
            _ => self.execution_engine.execute_expression(expr),
        }
        .map_err(OvmError::from)
    }

    /// Execute expression with pipeline optimization
    fn execute_with_pipeline(&mut self, expr: execution::OvmExpr) -> Result<OvmValue, OvmError> {
        // Use pipeline engine for pipeline expressions - simplified for now
        // Full implementation would optimize pipeline operations
        self.execution_engine.execute_expression(expr)
            .map_err(OvmError::from)
    }

    /// Execute expression with fusion optimization
    fn execute_with_fusion(&mut self, expr: execution::OvmExpr) -> Result<OvmValue, OvmError> {
        // Use fusion engine for fusable operations - simplified for now
        // Full implementation would fuse operations for performance
        self.execution_engine.execute_expression(expr)
            .map_err(OvmError::from)
    }

    /// Create a lazy range stream
    fn create_lazy_range(&mut self, start: i64, end: i64) -> Result<OvmValue, OvmError> {
        use crate::ovm::value::{GeneratorFunction, StreamObject, TypeTag, LazyState, ExecutionTier, ValueHeader, ValueData};
        use std::sync::atomic::AtomicU32;

        // Create a lazy stream that generates range values on demand
        let step = if start < end { 1 } else { -1 };
        let generator = GeneratorFunction::Range { start, end, step };
        
        let stream_obj = StreamObject {
            generator: Mutex::new(generator),
            buffer: Mutex::new(Vec::new()),
            buffer_position: Mutex::new(0),
            is_infinite: false,
            chunk_size: 64, // Reasonable chunk size
        };

        let stream_ptr = std::sync::Arc::new(stream_obj);
        
        Ok(OvmValue {
            header: ValueHeader {
                type_tag: TypeTag::Stream,
                lazy_state: LazyState::Stream,
                tier: ExecutionTier::Interpreter,
                gc_bits: AtomicU32::new(0),
                optimization_data: std::mem::size_of::<StreamObject>() as u32,
                force_count: AtomicU32::new(0),
                ref_count: AtomicU32::new(1),
                gc_mark: false,
                age: 0,
                size: std::mem::size_of::<StreamObject>() as u32,
            },
            data: ValueData::Stream(stream_ptr),
        })
    }

    /// Create a lazy list (simplified implementation)
    #[allow(dead_code)]
    fn create_lazy_list(&mut self, _items: Vec<execution::OvmExpr>) -> Result<OvmValue, OvmError> {
        use crate::ovm::value::{LazyListObject, TransformationChain, TypeTag, LazyState, ExecutionTier, ValueHeader, ValueData};
        use std::sync::atomic::AtomicU32;

        // For now, create a simple lazy list with a unit source
        // This is a simplified implementation
        let source_value = OvmValue::new_unit();
        
        let lazy_list_obj = LazyListObject {
            source: Box::new(source_value),
            transformation: TransformationChain::Identity,
            materialized_prefix: Mutex::new(Vec::new()),
            materialization_point: Mutex::new(0),
        };

        let lazy_list_ptr = std::sync::Arc::new(lazy_list_obj);
        
        Ok(OvmValue {
            header: ValueHeader {
                type_tag: TypeTag::LazyList,
                lazy_state: LazyState::Lazy,
                tier: ExecutionTier::Interpreter,
                gc_bits: AtomicU32::new(0),
                optimization_data: std::mem::size_of::<LazyListObject>() as u32,
                force_count: AtomicU32::new(0),
                ref_count: AtomicU32::new(1),
                gc_mark: false,
                age: 0,
                size: std::mem::size_of::<LazyListObject>() as u32,
            },
            data: ValueData::LazyList(lazy_list_ptr),
        })
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

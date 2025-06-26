//! OVM Execution Engine
//!
//! Provides tiered execution with interpreter, bytecode VM, and JIT compilation

use crate::ast::{Expr, FunctionDecl, Statement, Value};
use crate::interpreter::{Interpreter, InterpreterError};
use crate::ovm::{FunctionId, MemoryManager, OvmConfig, OvmValue};
use crate::ovm::bytecode::{BytecodeVm, BytecodeError};
use crate::ovm::gc::SafepointManager;
use crate::ovm::optimization::{OptimizationEngine, OptimizationError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};


/// Main execution engine with tiered execution
pub struct ExecutionEngine {
    // Interpreter for immediate execution and fallback
    interpreter: Arc<Mutex<Interpreter>>,

    // Bytecode VM for intermediate tier execution
    bytecode_vm: Arc<Mutex<BytecodeVm>>,

    // Function registry
    functions: HashMap<FunctionId, FunctionDecl>,

    // Execution statistics for tier management
    execution_stats: HashMap<FunctionId, ExecutionStats>,

    // Configuration
    config: ExecutionConfig,

    // Safepoint manager for GC coordination
    safepoint_manager: Arc<SafepointManager>,

    // JIT compilation and optimization engine
    optimization_engine: Arc<Mutex<OptimizationEngine>>,
}

/// Internal OVM expression representation
pub struct OvmExpr {
    pub expr: Expr,
    pub execution_tier: ExecutionTier,
}

/// Execution statistics for functions
#[derive(Debug, Clone)]
struct ExecutionStats {
    call_count: u32,
    total_time_ns: u64,
    tier: ExecutionTier,
}

/// Current execution tier for expressions/functions
#[derive(Debug, Clone, Copy)]
pub enum ExecutionTier {
    Interpreter,
    Bytecode,
    Native,
}

/// Execution configuration
#[derive(Debug, Clone)]
struct ExecutionConfig {
    // Thresholds for tier transitions
    bytecode_threshold: u32, // Calls before moving to bytecode
    native_threshold: u32,   // Calls before JIT compilation

    // Optimization settings
    enable_tier_transition: bool,
    enable_profiling: bool,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            bytecode_threshold: 100,
            native_threshold: 1000,
            enable_tier_transition: true,
            enable_profiling: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("Execution failed: {0}")]
    Failed(String),

    #[error("Interpreter error: {0}")]
    InterpreterError(#[from] InterpreterError),

    #[error("Bytecode error: {0}")]
    BytecodeError(#[from] BytecodeError),

    #[error("Optimization error: {0}")]
    OptimizationError(#[from] OptimizationError),

    #[error("Function not found: {0:?}")]
    FunctionNotFound(FunctionId),

    #[error("Conversion error: {0}")]
    ConversionError(String),

    #[error("Tier transition failed: {0}")]
    TierTransitionFailed(String),
}

/// RAII guard for thread safepoint coordination
struct ThreadSafepointGuard {
    safepoint_manager: Arc<SafepointManager>,
}

impl ThreadSafepointGuard {
    fn new(safepoint_manager: Arc<SafepointManager>) -> Self {
        Self { safepoint_manager }
    }
}

impl Drop for ThreadSafepointGuard {
    fn drop(&mut self) {
        self.safepoint_manager.unregister_thread();
    }
}

impl ExecutionEngine {
    pub fn new(config: &OvmConfig, _memory: &MemoryManager) -> Result<Self, ExecutionError> {
        let execution_config = ExecutionConfig::default();

        Ok(Self {
            interpreter: Arc::new(Mutex::new(Interpreter::new())),
            bytecode_vm: Arc::new(Mutex::new(BytecodeVm::new())),
            functions: HashMap::new(),
            execution_stats: HashMap::new(),
            config: execution_config,
            safepoint_manager: Arc::new(SafepointManager::new()),
            optimization_engine: Arc::new(Mutex::new(OptimizationEngine::new(config)?)),
        })
    }

    /// Execute an OVM expression
    pub fn execute_expression(&mut self, ovm_expr: OvmExpr) -> Result<OvmValue, ExecutionError> {
        // Register thread for safepoint coordination
        self.safepoint_manager.register_thread();
        
        // Ensure thread is unregistered when done
        let _guard = ThreadSafepointGuard::new(Arc::clone(&self.safepoint_manager));
        
        let start_time = std::time::Instant::now();

        // Safepoint poll before execution
        self.safepoint_manager.safepoint_poll()
            .map_err(|e| ExecutionError::Failed(format!("Safepoint coordination failed: {}", e)))?;

        // Route expressions through appropriate execution tier
        let result = match ovm_expr.execution_tier {
            ExecutionTier::Interpreter => self.execute_with_interpreter(ovm_expr.expr)?,
            ExecutionTier::Bytecode => {
                // For expressions, fall back to interpreter for now
                // Full bytecode compilation is more suitable for functions
                self.execute_with_interpreter(ovm_expr.expr)?
            }
            ExecutionTier::Native => {
                // For expressions, fall back to interpreter for now  
                // JIT compilation is more suitable for functions
                self.execute_with_interpreter(ovm_expr.expr)?
            }
        };

        let execution_time = start_time.elapsed();

        // Record execution statistics for profiling
        if self.config.enable_profiling {
            // TODO: Record stats for expression-level profiling
        }

        Ok(result)
    }

    /// Execute a function by ID
    pub fn execute_function(
        &mut self,
        func_id: FunctionId,
        args: &[OvmValue],
    ) -> Result<OvmValue, ExecutionError> {
        // Register thread for safepoint coordination
        self.safepoint_manager.register_thread();
        
        // Ensure thread is unregistered when done
        let _guard = ThreadSafepointGuard::new(Arc::clone(&self.safepoint_manager));
        
        let func_decl = self
            .functions
            .get(&func_id)
            .ok_or(ExecutionError::FunctionNotFound(func_id))?
            .clone();

        let start_time = std::time::Instant::now();

        // Safepoint poll before execution
        self.safepoint_manager.safepoint_poll()
            .map_err(|e| ExecutionError::Failed(format!("Safepoint coordination failed: {}", e)))?;

        // Convert OVM values to AST values for interpreter
        let mut ast_args = Vec::new();
        for arg in args {
            ast_args.push(self.ovm_value_to_ast(arg)?);
        }

        // Get current execution tier for this function
        let tier = self
            .execution_stats
            .get(&func_id)
            .map(|stats| stats.tier)
            .unwrap_or(ExecutionTier::Interpreter);

        let result = match tier {
            ExecutionTier::Interpreter => {
                self.execute_function_with_interpreter(&func_decl, &ast_args)?
            }
            ExecutionTier::Bytecode => {
                // Execute with bytecode VM
                self.execute_function_with_bytecode(func_id, &func_decl, args)?
            }
            ExecutionTier::Native => {
                // Execute with JIT compiled code
                self.execute_function_with_native_code(func_id, &ast_args)?
            }
        };

        let execution_time = start_time.elapsed();

        // Update execution statistics
        self.update_function_stats(func_id, execution_time);

        // Check for tier transition
        if self.config.enable_tier_transition {
            self.consider_tier_transition(func_id);
        }

        Ok(result)
    }

    /// Register a function for execution
    pub fn register_function(
        &mut self,
        func_id: FunctionId,
        func: FunctionDecl,
    ) -> Result<(), ExecutionError> {
        self.functions.insert(func_id, func);
        self.execution_stats.insert(
            func_id,
            ExecutionStats {
                call_count: 0,
                total_time_ns: 0,
                tier: ExecutionTier::Interpreter,
            },
        );
        Ok(())
    }

    // Private implementation methods

    /// Execute expression using the interpreter
    fn execute_with_interpreter(&mut self, expr: Expr) -> Result<OvmValue, ExecutionError> {
        let mut interpreter = self
            .interpreter
            .lock()
            .map_err(|_| ExecutionError::Failed("Failed to lock interpreter".to_string()))?;

        // Wrap expression in a statement to use the public interface
        let statement = Statement::Expression(expr);
        let ast_result = interpreter.eval_statement(statement)?;
        self.ast_value_to_ovm(&ast_result)
    }

    /// Execute function using the interpreter
    fn execute_function_with_interpreter(
        &mut self,
        func_decl: &FunctionDecl,
        args: &[Value],
    ) -> Result<OvmValue, ExecutionError> {
        let mut interpreter = self
            .interpreter
            .lock()
            .map_err(|_| ExecutionError::Failed("Failed to lock interpreter".to_string()))?;

        // Create function value
        let function_value = Value::Function(crate::ast::Function {
            name: Some(func_decl.name.clone()),
            parameters: func_decl.parameters.clone(),
            body: func_decl.body.clone(),
            closure: HashMap::new(),
        });

        // Call the function
        let ast_result = interpreter.call_function(function_value, args.to_vec())?;
        self.ast_value_to_ovm(&ast_result)
    }

    /// Convert AST value to OVM value
    fn ast_value_to_ovm(&self, value: &Value) -> Result<OvmValue, ExecutionError> {
        // For now, use a simple conversion
        // TODO: Implement proper OVM value creation with GC integration
        Ok(OvmValue::from_ast(value.clone()))
    }

    /// Convert OVM value to AST value
    fn ovm_value_to_ast(&self, value: &OvmValue) -> Result<Value, ExecutionError> {
        value.to_ast().map_err(|e| {
            ExecutionError::ConversionError(format!("OVM to AST conversion failed: {:?}", e))
        })
    }

    /// Update execution statistics for a function
    fn update_function_stats(&mut self, func_id: FunctionId, execution_time: std::time::Duration) {
        if let Some(stats) = self.execution_stats.get_mut(&func_id) {
            stats.call_count += 1;
            stats.total_time_ns += execution_time.as_nanos() as u64;
        }
    }

    /// Consider whether a function should transition to a higher tier
    fn consider_tier_transition(&mut self, func_id: FunctionId) {
        if let Some(stats) = self.execution_stats.get_mut(&func_id) {
            match stats.tier {
                ExecutionTier::Interpreter => {
                    if stats.call_count >= self.config.bytecode_threshold {
                        // Transition to bytecode tier
                        stats.tier = ExecutionTier::Bytecode;
                        println!("Function {:?} promoted to bytecode tier (call count: {})", func_id, stats.call_count);
                        
                        // Pre-compile function to bytecode for next execution
                        if let Some(func_decl) = self.functions.get(&func_id) {
                            if let Ok(mut bytecode_vm) = self.bytecode_vm.lock() {
                                if let Err(e) = bytecode_vm.compile_function(func_id, func_decl) {
                                    println!("Bytecode compilation failed for {:?}: {}", func_id, e);
                                    // Stay at interpreter tier
                                    stats.tier = ExecutionTier::Interpreter;
                                }
                            }
                        }
                    }
                }
                ExecutionTier::Bytecode => {
                    if stats.call_count >= self.config.native_threshold {
                        // Transition to native tier (JIT compilation)
                        stats.tier = ExecutionTier::Native;
                        
                        // Trigger JIT compilation in background
                        if let Ok(mut opt_engine) = self.optimization_engine.lock() {
                            let func_name = format!("func_{:?}", func_id);
                            if let Err(e) = opt_engine.compile_function_sync(func_id, func_name) {
                                println!("JIT compilation failed for {:?}: {}", func_id, e);
                                // Fall back to bytecode tier
                                stats.tier = ExecutionTier::Bytecode;
                            } else {
                                println!("Function {:?} JIT compiled successfully (call count: {})", func_id, stats.call_count);
                            }
                        }
                    }
                }
                ExecutionTier::Native => {
                    // Already at highest tier - check for deoptimization conditions
                    let avg_time = if stats.call_count > 0 {
                        stats.total_time_ns / stats.call_count as u64
                    } else {
                        0
                    };
                    
                    // If performance degrades significantly, consider deoptimization
                    if avg_time > 1_000_000 && stats.call_count % 100 == 0 { // 1ms threshold, check every 100 calls
                        if let Ok(mut opt_engine) = self.optimization_engine.lock() {
                            if let Err(e) = opt_engine.deoptimize_function(func_id) {
                                println!("Deoptimization failed for {:?}: {}", func_id, e);
                            } else {
                                println!("Function {:?} deoptimized back to bytecode tier", func_id);
                                stats.tier = ExecutionTier::Bytecode; // Deoptimize to bytecode, not interpreter
                            }
                        }
                    }
                }
            }
        }
    }

    /// Execute a function using native JIT compiled code
    fn execute_function_with_native_code(
        &mut self,
        func_id: FunctionId,
        args: &[Value],
    ) -> Result<OvmValue, ExecutionError> {
        // Convert AST values to OVM values for JIT execution
        let mut ovm_args = Vec::new();
        for arg in args {
            ovm_args.push(OvmValue::from_ast(arg.clone()));
        }

        // Execute with JIT compiler
        let execution_result = if let Ok(mut opt_engine) = self.optimization_engine.lock() {
            opt_engine.execute_compiled_function(func_id, &ovm_args)
        } else {
            Err(OptimizationError::Failed("Optimization engine lock failed".to_string()))
        };

        match execution_result {
            Ok(result) => Ok(result),
            Err(OptimizationError::FunctionNotFound(_)) => {
                // Function not compiled yet, fall back to interpreter
                self.execute_function_with_interpreter_fallback(func_id, args)
            }
            Err(e) => {
                println!("Native execution failed for {:?}: {}", func_id, e);
                // Deoptimize and fall back to interpreter
                if let Some(stats) = self.execution_stats.get_mut(&func_id) {
                    stats.tier = ExecutionTier::Interpreter;
                }
                self.execute_function_with_interpreter_fallback(func_id, args)
            }
        }
    }

    /// Fallback to interpreter execution with function lookup
    fn execute_function_with_interpreter_fallback(
        &mut self,
        func_id: FunctionId,
        args: &[Value],
    ) -> Result<OvmValue, ExecutionError> {
        let func_decl = self
            .functions
            .get(&func_id)
            .ok_or(ExecutionError::FunctionNotFound(func_id))?
            .clone();
        
        self.execute_function_with_interpreter(&func_decl, args)
    }

    /// Execute function using bytecode VM
    fn execute_function_with_bytecode(
        &mut self,
        func_id: FunctionId,
        func_decl: &FunctionDecl,
        args: &[OvmValue],
    ) -> Result<OvmValue, ExecutionError> {
        // Try to execute with bytecode VM
        let bytecode_result = {
            if let Ok(mut bytecode_vm) = self.bytecode_vm.lock() {
                // Check if function is already compiled to bytecode
                if !bytecode_vm.has_bytecode(func_id) {
                    // Compile function to bytecode
                    bytecode_vm.compile_function(func_id, func_decl)?;
                    println!("Function {:?} compiled to bytecode", func_id);
                }

                // Execute with bytecode
                Some(bytecode_vm.execute(func_id, args).map_err(ExecutionError::from))
            } else {
                None
            }
        };

        // Handle result or fall back to interpreter
        match bytecode_result {
            Some(result) => result,
            None => {
                // Fall back to interpreter if bytecode VM is unavailable
                println!("Bytecode VM lock failed, falling back to interpreter for {:?}", func_id);
                let ast_args: Result<Vec<_>, _> = args.iter().map(|arg| self.ovm_value_to_ast(arg)).collect();
                self.execute_function_with_interpreter(func_decl, &ast_args?)   
            }
        }
    }

    /// Get JIT compilation statistics
    pub fn get_jit_stats(&self) -> Option<String> {
        if let Ok(opt_engine) = self.optimization_engine.lock() {
            let stats = opt_engine.get_stats();
            Some(format!(
                "JIT Stats: {} functions compiled, {:.2}ms total compilation time, {} cache hits, {} deoptimizations",
                stats.functions_compiled,
                stats.total_compilation_time.as_millis(),
                stats.cache_hits,
                stats.deoptimizations
            ))
        } else {
            None
        }
    }

    /// Register a function with the optimization engine for profiling
    pub fn register_function_for_optimization(
        &mut self,
        func_id: FunctionId,
        func: &FunctionDecl,
    ) -> Result<(), ExecutionError> {
        if let Ok(mut opt_engine) = self.optimization_engine.lock() {
            opt_engine.register_function(func_id, func.clone())?;
        }
        Ok(())
    }

    /// Get bytecode VM statistics  
    pub fn get_bytecode_stats(&self) -> Option<String> {
        if let Ok(bytecode_vm) = self.bytecode_vm.lock() {
            let stats = bytecode_vm.get_stats();
            Some(format!(
                "Bytecode VM Stats: {} instructions executed, {} function calls, {:.2}ms compilation time, {:.2}ms execution time",
                stats.instructions_executed,
                stats.function_calls,
                stats.compilation_time.as_millis(),
                stats.execution_time.as_millis()
            ))
        } else {
            None
        }
    }
}

impl OvmExpr {
    /// Create OVM expression from AST expression
    pub fn from_ast(expr: Expr) -> Result<Self, ExecutionError> {
        Ok(Self {
            expr,
            execution_tier: ExecutionTier::Interpreter, // Start with interpreter
        })
    }

    /// Create with specific execution tier
    pub fn with_tier(expr: Expr, tier: ExecutionTier) -> Self {
        Self {
            expr,
            execution_tier: tier,
        }
    }
}

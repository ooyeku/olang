//! OVM Execution Engine
//!
//! Provides tiered execution with interpreter, bytecode VM, and JIT compilation

use crate::ast::{Argument, Expr, FunctionDecl, Statement, Value};
use crate::builtin::BuiltinFunctions;
use crate::interpreter::{Interpreter, InterpreterError};
use crate::ovm::bytecode::{BytecodeError, BytecodeVm};
use crate::ovm::gc::SafepointManager;
use crate::ovm::optimization::{OptimizationEngine, OptimizationError};
use crate::ovm::{FunctionId, MemoryManager, OvmConfig, OvmValue};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Main execution engine with tiered execution
pub struct ExecutionEngine {
    // Interpreter for immediate execution and fallback
    interpreter: Arc<Mutex<Interpreter>>,

    // Bytecode VM for intermediate tier execution
    bytecode_vm: Arc<Mutex<BytecodeVm>>,

    // Builtin functions registry for OVM-native builtin execution
    builtin_functions: BuiltinFunctions,

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            builtin_functions: BuiltinFunctions::new(),
            functions: HashMap::new(),
            execution_stats: HashMap::new(),
            config: execution_config,
            safepoint_manager: Arc::new(SafepointManager::new()),
            optimization_engine: Arc::new(Mutex::new(OptimizationEngine::new(config)?)),
        })
    }

    /// Execute a builtin function call natively in OVM
    pub fn execute_builtin(
        &mut self,
        function_name: &str,
        args: &[OvmValue],
    ) -> Result<OvmValue, ExecutionError> {
        // Convert OVM values to AST values for builtin execution
        let mut ast_args = Vec::new();
        for arg in args {
            ast_args.push(arg.to_ast().map_err(|e| {
                ExecutionError::ConversionError(format!("OVM to AST conversion failed: {:?}", e))
            })?);
        }

        // Execute builtin function using the interpreter's builtin system
        let mut interpreter = self
            .interpreter
            .lock()
            .map_err(|_| ExecutionError::Failed("Failed to lock interpreter".to_string()))?;

        let result = BuiltinFunctions::call(
            &self.builtin_functions,
            function_name,
            ast_args,
            &mut interpreter,
        )?;

        // Convert result back to OVM value
        Ok(OvmValue::from_ast(result))
    }

    /// Check if a function name is a builtin function
    pub fn is_builtin_function(&self, name: &str) -> bool {
        self.builtin_functions.get_functions().contains_key(name)
    }

    /// Get the list of available builtin functions
    pub fn get_builtin_functions(&self) -> Vec<String> {
        self.builtin_functions
            .get_functions()
            .keys()
            .cloned()
            .collect()
    }

    /// Execute an OVM expression, with enhanced builtin support
    pub fn execute_expression(&mut self, ovm_expr: OvmExpr) -> Result<OvmValue, ExecutionError> {
        // Register thread for safepoint coordination
        self.safepoint_manager.register_thread();

        // Ensure thread is unregistered when done
        let _guard = ThreadSafepointGuard::new(Arc::clone(&self.safepoint_manager));

        let start_time = std::time::Instant::now();

        // Safepoint poll before execution
        self.safepoint_manager
            .safepoint_poll()
            .map_err(|e| ExecutionError::Failed(format!("Safepoint coordination failed: {}", e)))?;

        // Check if this is a builtin function call
        let result = if let Some((builtin_name, args)) = self.extract_builtin_call(&ovm_expr.expr) {
            // Execute builtin function natively in OVM
            self.execute_builtin(&builtin_name, &args)?
        } else {
            // Route expressions through appropriate execution tier
            match ovm_expr.execution_tier {
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
            }
        };

        let _execution_time = start_time.elapsed();

        // Record execution statistics for profiling
        if self.config.enable_profiling {
            // TODO: Record stats for expression-level profiling
        }

        Ok(result)
    }

    /// Extract builtin function call information from an expression
    fn extract_builtin_call(&self, expr: &Expr) -> Option<(String, Vec<OvmValue>)> {
        match expr {
            Expr::Call { callee, arguments } => {
                if let Expr::Identifier(name) = callee.as_ref() {
                    if self.is_builtin_function(name) {
                        // Convert arguments to OVM values
                        let mut ovm_args = Vec::new();
                        for arg in arguments {
                            let arg_expr = match arg {
                                Argument::Positional(expr) => expr,
                                Argument::Named { value, .. } => value,
                            };
                            // For now, we'll evaluate arguments using the interpreter
                            // In a full implementation, we'd recursively evaluate them in OVM
                            if let Ok(ast_value) = self.evaluate_expr_to_ast(arg_expr) {
                                ovm_args.push(OvmValue::from_ast(ast_value));
                            } else {
                                // If argument evaluation fails, don't treat as builtin call
                                return None;
                            }
                        }
                        return Some((name.clone(), ovm_args));
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// Helper to evaluate an expression to AST value (temporary implementation)
    fn evaluate_expr_to_ast(&self, expr: &Expr) -> Result<Value, ExecutionError> {
        // This is a temporary implementation - in production, we'd evaluate recursively in OVM
        let mut interpreter = self
            .interpreter
            .lock()
            .map_err(|_| ExecutionError::Failed("Failed to lock interpreter".to_string()))?;

        interpreter
            .eval_statement(&Statement::Expression(expr.clone()))
            .map_err(ExecutionError::from)
    }

    /// Execute a function with tiered execution strategy
    pub fn execute_function(
        &mut self,
        func_id: FunctionId,
        args: &[OvmValue],
    ) -> Result<OvmValue, ExecutionError> {
        // Register thread for safepoint coordination
        self.safepoint_manager.register_thread();

        // Ensure thread is unregistered when done
        let _guard = ThreadSafepointGuard::new(Arc::clone(&self.safepoint_manager));

        let start_time = std::time::Instant::now();

        // **Phase 3: Enhanced tiered execution with JIT compilation**
        
        // First, check if we have compiled native code and execute if available
        let has_native_code = {
            if let Ok(optimization_engine) = self.optimization_engine.lock() {
                optimization_engine.has_compiled_function(func_id)
            } else {
                false
            }
        };

        if has_native_code {
            // **Phase 3: Execute with native JIT code**
            let result = {
                if let Ok(mut opt_engine) = self.optimization_engine.lock() {
                    opt_engine.execute_compiled_function(func_id, args)
                } else {
                    Err(OptimizationError::Failed("JIT compiler lock failed".to_string()))
                }
            };

            // Only return on success — a JIT failure falls through to the
            // bytecode/interpreter tiers instead of aborting the call
            if let Ok(value) = result {
                let execution_time = start_time.elapsed();
                self.update_function_stats(func_id, execution_time);
                return Ok(value);
            }
        }

        // Check if we have bytecode for this function and execute if available
        let has_bytecode = {
            if let Ok(bytecode_vm) = self.bytecode_vm.lock() {
                bytecode_vm.has_bytecode(func_id)
            } else {
                false
            }
        };

        if has_bytecode {
            // Execute with bytecode VM
            let result = {
                if let Ok(mut vm) = self.bytecode_vm.lock() {
                    vm.execute(func_id, args)
                } else {
                    Err(BytecodeError::RuntimeError("Bytecode VM lock failed".to_string()))
                }
            };
            
            // Update execution statistics
            let execution_time = start_time.elapsed();
            self.update_function_stats(func_id, execution_time);
            
            return result.map_err(ExecutionError::BytecodeError);
        }

        // **Phase 3: Fallback to interpreter with compilation consideration**
        if let Some(func_decl) = self.functions.get(&func_id) {
            let func_decl = func_decl.clone();
            // Convert OVM values to AST values for interpreter
            let mut ast_args = Vec::new();
            for arg in args {
                ast_args.push(self.ovm_value_to_ast(arg)?);
            }

            // Execute with interpreter
            let result = self.execute_function_with_interpreter(&func_decl, &ast_args)?;

            // **Phase 3: Consider compilation after interpreter execution**
            let execution_time = start_time.elapsed();
            self.update_function_stats(func_id, execution_time);
            self.consider_tier_transition(func_id);

            Ok(result)
        } else {
            Err(ExecutionError::FunctionNotFound(func_id))
        }
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
        let ast_result = interpreter.eval_statement(&statement)?;
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
            body: std::sync::Arc::new(func_decl.body.clone()),
            closure: std::sync::Arc::new(im::HashMap::new()),
        });

        // Call the function
        let ast_result = interpreter.call_function(function_value, args.to_vec())?;
        self.ast_value_to_ovm(&ast_result)
    }

    /// Convert AST value to OVM value with proper GC integration
    fn ast_value_to_ovm(&self, value: &Value) -> Result<OvmValue, ExecutionError> {
        // Create OVM value with proper GC integration and metadata
        let ovm_value = OvmValue::from_ast_with_gc(value.clone(), &self.safepoint_manager)
            .map_err(|e| ExecutionError::ConversionError(format!("GC integration failed: {:?}", e)))?;
        
        // Update allocation statistics for GC triggering
        self.safepoint_manager.record_allocation(std::mem::size_of::<OvmValue>());
        
        // Check if we should trigger GC collection
        if self.safepoint_manager.should_collect() {
            // Perform safepoint-coordinated GC if threshold reached
            self.safepoint_manager.request_collection();
        }
        
        Ok(ovm_value)
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

    /// **Phase 3: Enhanced tier transition with JIT compilation**
    fn consider_tier_transition(&mut self, func_id: FunctionId) {
        if !self.config.enable_tier_transition {
            return;
        }

        if let Some(stats) = self.execution_stats.get(&func_id) {
            let current_tier = stats.tier;

            // **Phase 3: JIT compilation threshold check**
            if current_tier == ExecutionTier::Bytecode && stats.call_count >= self.config.native_threshold {
                // Consider JIT compilation
                if let Ok(mut optimization_engine) = self.optimization_engine.lock() {
                    let readiness = optimization_engine.assess_compilation_readiness(func_id);
                    
                    match readiness {
                        crate::ovm::optimization::CompilationReadiness::HighPriority => {
                            // Force JIT compilation
                            if let Some(_func_decl) = self.functions.get(&func_id) {
                                let _ = optimization_engine.force_compile_function(
                                    func_id,
                                    format!("func_{}", func_id.0),
                                    crate::ovm::optimization::CompilationTier::OptimizedJit,
                                );
                                
                                // Update tier
                                if let Some(stats) = self.execution_stats.get_mut(&func_id) {
                                    stats.tier = ExecutionTier::Native;
                                }
                                
                                println!("Phase 3: Function {:?} promoted to Native tier", func_id);
                            }
                        }
                        crate::ovm::optimization::CompilationReadiness::Medium => {
                            // Queue for background compilation
                            if let Some(_func_decl) = self.functions.get(&func_id) {
                                let _ = optimization_engine.compile_function_with_body(
                                    func_id,
                                    format!("func_{}", func_id.0),
                                    Some("compiled_function".to_string()),
                                );
                            }
                        }
                        _ => {
                            // Not ready for compilation yet
                        }
                    }
                }
            }
            // **Phase 3: Bytecode compilation threshold check**
            else if current_tier == ExecutionTier::Interpreter && stats.call_count >= self.config.bytecode_threshold {
                // Compile to bytecode
                if let Some(func_decl) = self.functions.get(&func_id) {
                    if let Ok(mut vm) = self.bytecode_vm.lock() {
                        if let Ok(()) = vm.compile_function(func_id, func_decl) {
                            // Update tier
                            if let Some(stats) = self.execution_stats.get_mut(&func_id) {
                                stats.tier = ExecutionTier::Bytecode;
                            }
                            
                            println!("Phase 3: Function {:?} promoted to Bytecode tier", func_id);
                        }
                    }
                }
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

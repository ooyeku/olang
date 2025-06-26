//! OVM Integration Layer
//!
//! Provides seamless integration between the main interpreter with the Olang interpreter
//! and the Olang Virtual Machine (OVM) for enhanced performance.

use crate::ast::{FunctionDecl, Program, Value};
use crate::interpreter::{Interpreter, InterpreterError};
use crate::ovm::{FunctionId, OlangVirtualMachine, OvmConfig, OvmError, OvmValue};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Enhanced interpreter that can use either traditional interpretation or OVM execution
pub struct OvmInterpreter {
    /// Traditional interpreter for compatibility
    classic_interpreter: Interpreter,

    /// OVM instance for high-performance execution
    ovm: Option<OlangVirtualMachine>,

    /// Configuration for OVM integration
    integration_config: IntegrationConfig,

    /// Function registry mapping names to OVM function IDs
    function_registry: HashMap<String, FunctionId>,

    /// Performance statistics
    execution_stats: Arc<Mutex<ExecutionStats>>,
}

/// Configuration for OVM integration behavior
#[derive(Debug, Clone)]
pub struct IntegrationConfig {
    /// Whether to use OVM by default for new expressions
    pub use_ovm_by_default: bool,

    /// Threshold for switching to OVM (based on complexity)
    pub ovm_complexity_threshold: usize,

    /// Whether to enable automatic function compilation
    pub auto_compile_functions: bool,

    /// Whether to enable lazy evaluation through OVM
    pub enable_ovm_lazy_eval: bool,

    /// Fallback to classic interpreter on OVM errors
    pub fallback_on_error: bool,

    /// Whether to enable OVM builtin execution
    pub enable_ovm_builtins: bool,

    /// List of builtin functions that should use OVM
    pub ovm_preferred_builtins: Vec<String>,
}

impl Default for IntegrationConfig {
    fn default() -> Self {
        Self {
            use_ovm_by_default: true,
            ovm_complexity_threshold: 10,
            auto_compile_functions: true,
            enable_ovm_lazy_eval: true,
            fallback_on_error: true,
            enable_ovm_builtins: true,
            ovm_preferred_builtins: vec![
                // Simple mathematical builtins that can benefit from OVM
                "len".to_string(),
                "typeof".to_string(),
                "to_string".to_string(),
                "to_int".to_string(),
                "to_float".to_string(),
                "sum".to_string(),
                "average".to_string(),
                "min".to_string(),
                "max".to_string(),
                "clamp".to_string(),
                "reverse".to_string(),
                "sort".to_string(),
                "contains".to_string(),
                "starts_with".to_string(),
                "ends_with".to_string(),
                "flatten".to_string(),
                // Simple list operations
                "head".to_string(),
                "tail".to_string(),
                "cons".to_string(),
            ],
        }
    }
}

/// Statistics for tracking execution performance
#[derive(Debug, Default)]
pub struct ExecutionStats {
    pub classic_executions: u64,
    pub ovm_executions: u64,
    pub fallback_executions: u64,
    pub compilation_count: u64,
    pub average_classic_time_ms: f64,
    pub average_ovm_time_ms: f64,
}

/// Detailed OVM status information
#[derive(Debug, Clone)]
pub struct OvmStatus {
    pub initialized: bool,
    pub running: bool,
    pub memory_usage: usize,
    pub uptime: std::time::Duration,
}

impl OvmInterpreter {
    /// Create a new OVM-integrated interpreter
    pub fn new() -> Self {
        Self {
            classic_interpreter: Interpreter::new(),
            ovm: None,
            integration_config: IntegrationConfig::default(),
            function_registry: HashMap::new(),
            execution_stats: Arc::new(Mutex::new(ExecutionStats::default())),
        }
    }

    /// Create with custom integration configuration
    pub fn with_config(integration_config: IntegrationConfig) -> Self {
        Self {
            classic_interpreter: Interpreter::new(),
            ovm: None,
            integration_config,
            function_registry: HashMap::new(),
            execution_stats: Arc::new(Mutex::new(ExecutionStats::default())),
        }
    }

    /// Initialize the OVM with given configuration
    pub fn initialize_ovm(&mut self, ovm_config: OvmConfig) -> Result<(), IntegrationError> {
        // Stop any existing OVM first
        if let Some(ovm) = &mut self.ovm {
            let _ = ovm.stop();
        }
        self.ovm = None;

        let mut ovm =
            OlangVirtualMachine::new(ovm_config).map_err(IntegrationError::OvmInitError)?;

        ovm.start().map_err(IntegrationError::OvmInitError)?;

        self.ovm = Some(ovm);
        Ok(())
    }

    /// Initialize OVM with default configuration
    pub fn initialize_ovm_default(&mut self) -> Result<(), IntegrationError> {
        match self.initialize_ovm(OvmConfig::default()) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("OVM initialization failed: {}", e);
                Err(e)
            }
        }
    }

    /// Check if OVM is available and running
    pub fn is_ovm_available(&self) -> bool {
        self.ovm.is_some()
    }

    /// Evaluate a program using the best available execution method
    pub fn eval_program(&mut self, program: Program) -> Result<Value, IntegrationError> {
        let complexity = self.estimate_complexity(&program);
        let should_use_ovm = self.should_use_ovm(&program, complexity);

        if should_use_ovm {
            self.eval_program_ovm(program)
        } else {
            self.eval_program_classic(program)
        }
    }

    /// Evaluate using classic interpreter
    pub fn eval_program_classic(&mut self, program: Program) -> Result<Value, IntegrationError> {
        let start = std::time::Instant::now();
        let result = self
            .classic_interpreter
            .eval_program(program)
            .map_err(IntegrationError::ClassicInterpreterError)?;
        let duration = start.elapsed();

        self.update_classic_stats(duration);
        Ok(result)
    }

    /// Evaluate using OVM with enhanced builtin support
    pub fn eval_program_ovm(&mut self, program: Program) -> Result<Value, IntegrationError> {
        if self.ovm.is_none() {
            return Err(IntegrationError::OvmNotInitialized);
        }

        let start = std::time::Instant::now();
        let auto_compile = self.integration_config.auto_compile_functions;

        // Execute each statement through OVM
        let mut last_value = Value::Unit;
        for statement in program.statements {
            match statement {
                crate::ast::Statement::Expression(expr) => {
                    // Enhanced builtin routing logic
                    if self.should_use_ovm_for_expression(&expr) {
                        // Use OVM for enhanced execution
                        let ovm_value = self
                            .ovm
                            .as_mut()
                            .unwrap()
                            .execute_expression(expr)
                            .map_err(IntegrationError::OvmExecutionError)?;
                        last_value = self.convert_ovm_to_ast_value(ovm_value)?;
                    } else {
                        // Fallback to classic interpreter for complex builtins and pipelines
                        last_value = self
                            .classic_interpreter
                            .eval_statement(crate::ast::Statement::Expression(expr))
                            .map_err(IntegrationError::ClassicInterpreterError)?;
                        self.increment_fallback_count();
                    }
                }
                crate::ast::Statement::FunctionDecl(func_decl) => {
                    if auto_compile {
                        let func_id = self
                            .ovm
                            .as_mut()
                            .unwrap()
                            .register_function(func_decl.clone())
                            .map_err(IntegrationError::OvmExecutionError)?;
                        
                        self.function_registry.insert(func_decl.name.clone(), func_id);
                        self.increment_compilation_count();
                    }

                    // Also register with classic interpreter for compatibility
                    last_value = self
                        .classic_interpreter
                        .eval_statement(crate::ast::Statement::FunctionDecl(func_decl))
                        .map_err(IntegrationError::ClassicInterpreterError)?;
                }
                other_statement => {
                    // Handle other statement types with classic interpreter
                    last_value = self
                        .classic_interpreter
                        .eval_statement(other_statement)
                        .map_err(IntegrationError::ClassicInterpreterError)?;
                }
            }
        }

        let duration = start.elapsed();
        self.update_ovm_stats(duration);
        Ok(last_value)
    }

    /// Force garbage collection in OVM if available
    pub fn force_gc(&mut self) -> Result<(), IntegrationError> {
        if let Some(ovm) = &mut self.ovm {
            ovm.force_gc()
                .map_err(IntegrationError::OvmExecutionError)?;
        }
        Ok(())
    }

    /// Get performance statistics
    pub fn get_stats(&self) -> ExecutionStats {
        if let Ok(stats) = self.execution_stats.lock() {
            stats.clone()
        } else {
            ExecutionStats::default()
        }
    }

    /// Get the classic interpreter for direct access
    pub fn get_classic_interpreter(&mut self) -> &mut Interpreter {
        &mut self.classic_interpreter
    }

    /// Check if OVM is healthy and running
    pub fn is_ovm_healthy(&self) -> bool {
        // For now, consider OVM healthy if it exists
        // In a future version, we could expose a proper health check method from OVM
        self.ovm.is_some()
    }

    /// Restart OVM if it has stopped
    pub fn ensure_ovm_running(&mut self) -> Result<(), IntegrationError> {
        if self.ovm.is_none() {
            // Initialize OVM if not present
            self.initialize_ovm_default()?;
        }
        Ok(())
    }

    /// Get detailed OVM status
    pub fn get_ovm_status(&self) -> OvmStatus {
        if let Some(_ovm) = &self.ovm {
            OvmStatus {
                initialized: true,
                running: true, // Assume running if OVM exists
                memory_usage: 0, // Would need additional OVM methods for this
                uptime: std::time::Duration::ZERO, // Would need additional OVM methods for this
            }
        } else {
            OvmStatus {
                initialized: false,
                running: false,
                memory_usage: 0,
                uptime: std::time::Duration::ZERO,
            }
        }
    }

    /// Register a function for OVM compilation
    pub fn register_function(
        &mut self,
        func_decl: FunctionDecl,
    ) -> Result<FunctionId, IntegrationError> {
        let ovm = self
            .ovm
            .as_mut()
            .ok_or(IntegrationError::OvmNotInitialized)?;

        let func_id = ovm
            .register_function(func_decl.clone())
            .map_err(IntegrationError::OvmExecutionError)?;

        self.function_registry
            .insert(func_decl.name.clone(), func_id);
        self.increment_compilation_count();

        Ok(func_id)
    }

    // Private helper methods

    fn should_use_ovm(&self, _program: &Program, complexity: usize) -> bool {
        self.is_ovm_available()
            && (self.integration_config.use_ovm_by_default
                || complexity >= self.integration_config.ovm_complexity_threshold)
    }

    fn estimate_complexity(&self, program: &Program) -> usize {
        // Simple complexity estimation based on AST nodes
        program.statements.len() * 2 // Placeholder implementation
    }

    fn convert_ovm_to_ast_value(&self, ovm_value: OvmValue) -> Result<Value, IntegrationError> {
        // Convert OVM value back to AST value
        ovm_value.to_ast().map_err(|e| {
            IntegrationError::ConversionError(format!("OVM to AST conversion failed: {:?}", e))
        })
    }

    fn update_classic_stats(&self, duration: std::time::Duration) {
        if let Ok(mut stats) = self.execution_stats.lock() {
            stats.classic_executions += 1;
            let duration_ms = duration.as_secs_f64() * 1000.0;
            stats.average_classic_time_ms = (stats.average_classic_time_ms
                * (stats.classic_executions - 1) as f64
                + duration_ms)
                / stats.classic_executions as f64;
        }
    }

    fn update_ovm_stats(&self, duration: std::time::Duration) {
        if let Ok(mut stats) = self.execution_stats.lock() {
            stats.ovm_executions += 1;
            let duration_ms = duration.as_secs_f64() * 1000.0;
            stats.average_ovm_time_ms =
                (stats.average_ovm_time_ms * (stats.ovm_executions - 1) as f64 + duration_ms)
                    / stats.ovm_executions as f64;
        }
    }

    fn increment_fallback_count(&self) {
        if let Ok(mut stats) = self.execution_stats.lock() {
            stats.fallback_executions += 1;
        }
    }

    fn increment_compilation_count(&self) {
        if let Ok(mut stats) = self.execution_stats.lock() {
            stats.compilation_count += 1;
        }
    }

    /// Enhanced expression routing logic
    fn should_use_ovm_for_expression(&self, expr: &crate::ast::Expr) -> bool {
        match expr {
            // Function calls - enhanced builtin routing
            crate::ast::Expr::Call { callee, .. } => {
                match callee.as_ref() {
                    crate::ast::Expr::Identifier(name) => {
                        if self.is_builtin_function(name) {
                            // Check if this builtin should use OVM
                            self.should_use_ovm_for_builtin(name)
                        } else {
                            // User functions can use OVM
                            self.integration_config.use_ovm_by_default
                        }
                    }
                    _ => false, // Complex callees use classic interpreter for safety
                }
            }
            
            // Pipeline expressions should still use classic interpreter for now
            crate::ast::Expr::Pipeline { .. } => false,
            
            // Identifiers, loops should use classic interpreter for environment consistency
            crate::ast::Expr::Identifier(_) => false,
            crate::ast::Expr::ForLoop { .. } => false,
            crate::ast::Expr::WhileLoop { .. } => false,
            crate::ast::Expr::Loop { .. } => false,
            
            // Simple expressions can use OVM
            crate::ast::Expr::Integer(_) => true,
            crate::ast::Expr::Float(_) => true,
            crate::ast::Expr::String(_) => true,
            crate::ast::Expr::Boolean(_) => true,
            crate::ast::Expr::List(_) => true,
            crate::ast::Expr::Tuple(_) => true,
            crate::ast::Expr::BinaryOp { .. } => true,
            crate::ast::Expr::UnaryOp { .. } => true,
            
            // Complex expressions that can benefit from OVM optimization
            crate::ast::Expr::Lambda { .. } => true,
            crate::ast::Expr::Match { .. } => false, // Pattern matching uses classic for now
            crate::ast::Expr::StructLiteral(_) => true,
            crate::ast::Expr::FieldAccess { .. } => true,
            crate::ast::Expr::Index { .. } => true,
            
            // Default to OVM if configured
            _ => self.integration_config.use_ovm_by_default,
        }
    }

    /// Determine if a builtin function should use OVM execution
    fn should_use_ovm_for_builtin(&self, builtin_name: &str) -> bool {
        // Check if OVM builtins are enabled
        if !self.integration_config.enable_ovm_builtins {
            return false;
        }

        // Check if this builtin is in the OVM-preferred list
        self.integration_config.ovm_preferred_builtins.contains(&builtin_name.to_string())
    }

    /// Get OVM execution statistics
    pub fn get_ovm_builtin_stats(&self) -> HashMap<String, u32> {
        // In a full implementation, this would track per-builtin execution counts
        HashMap::new()
    }

    /// Force enable/disable OVM builtins at runtime
    pub fn set_ovm_builtins_enabled(&mut self, enabled: bool) {
        self.integration_config.enable_ovm_builtins = enabled;
    }

    /// Add a builtin to the OVM-preferred list
    pub fn add_ovm_preferred_builtin(&mut self, builtin_name: String) {
        if !self.integration_config.ovm_preferred_builtins.contains(&builtin_name) {
            self.integration_config.ovm_preferred_builtins.push(builtin_name);
        }
    }

    /// Remove a builtin from the OVM-preferred list
    pub fn remove_ovm_preferred_builtin(&mut self, builtin_name: &str) {
        self.integration_config.ovm_preferred_builtins.retain(|name| name != builtin_name);
    }

    /// Check if an expression should use the classic interpreter (updated logic)
    fn should_use_classic_interpreter(&self, expr: &crate::ast::Expr) -> bool {
        // Use the inverse of the enhanced OVM routing logic
        !self.should_use_ovm_for_expression(expr)
    }

    /// Check if a name corresponds to a builtin function
    fn is_builtin_function(&self, name: &str) -> bool {
        // List of builtin functions that should use classic interpreter
        matches!(
            name,
            "println"
                | "print"
                | "typeof"
                | "len"
                | "head"
                | "tail"
                | "cons"
                | "to_string"
                | "to_int"
                | "to_float"
                | "range"
                | "zip"
                | "map"
                | "filter"
                | "reduce"
                | "fold"
                | "reverse"
                | "sort"
                | "join"
                | "split"
                | "contains"
                | "sum"
                | "average"
                | "min"
                | "max"
                | "clamp"
                | "flatten"
                | "chunk"
                | "enumerate"
                | "find"
                | "starts_with"
                | "ends_with"
                | "group_by"
                | "set_parallel"
                | "take"
                | "skip"
                | "force"
                | "lazy"
        ) || name.contains('.') // Module functions like math.sqrt, fs.read_file, etc.
    }
}

impl Default for OvmInterpreter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for OvmInterpreter {
    fn drop(&mut self) {
        // Ensure OVM is properly shut down
        if let Some(ovm) = &mut self.ovm {
            let _ = ovm.stop();
        }
    }
}

/// Integration-specific error types
#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("Classic interpreter error: {0}")]
    ClassicInterpreterError(#[from] InterpreterError),

    #[error("OVM initialization error: {0}")]
    OvmInitError(#[from] OvmError),

    #[error("OVM execution error: {0}")]
    OvmExecutionError(OvmError),

    #[error("OVM not initialized")]
    OvmNotInitialized,

    #[error("Value conversion error: {0}")]
    ConversionError(String),
}

/// Result type for integration operations
pub type IntegrationResult<T> = Result<T, IntegrationError>;

// Implement Clone for ExecutionStats to make it accessible
impl Clone for ExecutionStats {
    fn clone(&self) -> Self {
        Self {
            classic_executions: self.classic_executions,
            ovm_executions: self.ovm_executions,
            fallback_executions: self.fallback_executions,
            compilation_count: self.compilation_count,
            average_classic_time_ms: self.average_classic_time_ms,
            average_ovm_time_ms: self.average_ovm_time_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, Statement};

    #[test]
    fn test_ovm_interpreter_creation() {
        let interpreter = OvmInterpreter::new();
        assert!(!interpreter.is_ovm_available());
    }

    #[test]
    fn test_ovm_initialization() {
        let mut interpreter = OvmInterpreter::new();
        assert!(interpreter.initialize_ovm_default().is_ok());
        assert!(interpreter.is_ovm_available());
    }

    #[test]
    fn test_classic_fallback() {
        let mut interpreter = OvmInterpreter::new();
        let program = Program {
            statements: vec![Statement::Expression(Expr::Integer(42))],
        };

        // Should use classic interpreter when OVM not available
        let result = interpreter.eval_program(program).unwrap();
        assert_eq!(result, Value::Integer(42));

        let stats = interpreter.get_stats();
        assert_eq!(stats.classic_executions, 1);
        assert_eq!(stats.ovm_executions, 0);
    }

    #[test]
    fn test_builtin_routing_configuration() {
        let mut interpreter = OvmInterpreter::new();
        
        // Test default configuration
        assert!(interpreter.integration_config.enable_ovm_builtins);
        assert!(interpreter.integration_config.ovm_preferred_builtins.contains(&"len".to_string()));
        
        // Test runtime configuration changes
        interpreter.set_ovm_builtins_enabled(false);
        assert!(!interpreter.integration_config.enable_ovm_builtins);
        
        interpreter.add_ovm_preferred_builtin("custom_builtin".to_string());
        assert!(interpreter.integration_config.ovm_preferred_builtins.contains(&"custom_builtin".to_string()));
        
        interpreter.remove_ovm_preferred_builtin("len");
        assert!(!interpreter.integration_config.ovm_preferred_builtins.contains(&"len".to_string()));
    }

    #[test]
    fn test_builtin_function_detection() {
        let interpreter = OvmInterpreter::new();
        
        // Test builtin detection
        assert!(interpreter.is_builtin_function("len"));
        assert!(interpreter.is_builtin_function("map"));
        assert!(interpreter.is_builtin_function("println"));
        assert!(!interpreter.is_builtin_function("not_a_builtin"));
        
        // Test OVM builtin preferences
        assert!(interpreter.should_use_ovm_for_builtin("len"));
        assert!(interpreter.should_use_ovm_for_builtin("sum"));
        assert!(!interpreter.should_use_ovm_for_builtin("map")); // Not in OVM preferred list
    }

    #[test]
    fn test_expression_routing() {
        let interpreter = OvmInterpreter::new();
        
        // Test simple expressions route to OVM
        let simple_expr = Expr::Integer(42);
        assert!(interpreter.should_use_ovm_for_expression(&simple_expr));
        
        // Test complex expressions route to classic
        let identifier_expr = Expr::Identifier("some_var".to_string());
        assert!(!interpreter.should_use_ovm_for_expression(&identifier_expr));
        
        // Test OVM-preferred builtin call routes to OVM
        let len_call = Expr::Call {
            callee: Box::new(Expr::Identifier("len".to_string())),
            arguments: vec![Expr::List(std::rc::Rc::from([Expr::Integer(1), Expr::Integer(2)] as [Expr; 2]))],
        };
        assert!(interpreter.should_use_ovm_for_expression(&len_call));
        
        // Test non-OVM builtin call routes to classic
        let map_call = Expr::Call {
            callee: Box::new(Expr::Identifier("map".to_string())),
            arguments: vec![
                Expr::List(std::rc::Rc::from([Expr::Integer(1), Expr::Integer(2)] as [Expr; 2])),
                Expr::Lambda {
                    parameters: vec![crate::ast::Parameter {
                        name: "x".to_string(),
                        type_annotation: None,
                    }],
                    body: Box::new(Expr::BinaryOp {
                        left: Box::new(Expr::Identifier("x".to_string())),
                        op: crate::ast::BinaryOp::Add,
                        right: Box::new(Expr::Integer(1)),
                    }),
                    return_type: None,
                },
            ],
        };
        assert!(!interpreter.should_use_ovm_for_expression(&map_call));
    }
}
